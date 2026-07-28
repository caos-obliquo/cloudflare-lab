use cloudflare_shared::{
    observability::{
        health::{DependencyHealth, HealthStatus},
        loki::{buffer_event, push_logs},
        otel::export_span,
        trace_context::TraceContext,
    },
    tracing::request_id_for_request,
};
use worker::*;

use crate::{
    handlers::{ai, d1, kv, queue},
    log_buffer, logger, metrics,
    utils::response::json_response,
};

// Hand-rolled router. No framework — keeps WASM binary small. OTel export via cx.wait_until() (async, non-blocking).
pub struct Router;

impl Router {
    pub async fn route(req: Request, env: Env, cx: Context) -> Result<Response> {
        let url = req.url()?;
        let path = url.path();
        let method = req.method().to_string();
        let req_id = request_id_for_request(&req)?;
        let ctx = TraceContext::from_request(&req)?;
        let start_ms = Date::now().as_millis();

        // Capture Origin before req is moved/matched.
        let req_origin = req.headers().get("Origin")?.unwrap_or_default();

        // CORS preflight: browser sends OPTIONS before cross-origin requests.
        if req.method() == Method::Options {
            let headers = Headers::new();
            // Reflect origin when available; fallback to wildcard (safe for OPTIONS).
            if req_origin.is_empty() {
                headers.set("Access-Control-Allow-Origin", "*")?;
            } else {
                headers.set("Access-Control-Allow-Origin", &req_origin)?;
            }
            headers.set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")?;
            headers.set(
                "Access-Control-Allow-Headers",
                "Content-Type, Authorization, traceparent, x-request-id",
            )?;
            let mut resp = Response::empty()?.with_status(204).with_headers(headers);
            resp.headers().set("X-Request-Id", &req_id)?;
            ctx.inject_into_response(&mut resp)?;
            let duration_ms = Date::now().as_millis() - start_ms;
            logger().response(&method, path, 204, duration_ms, &ctx).emit();
            return Ok(resp);
        }

        // Log incoming request.
        logger().request(&method, path, &ctx).emit();

        let mut resp = match path {
            "/" => json_response(
                200,
                &serde_json::json!({"status":"ok","service":"gateway-worker","language":"rust","routes":["/kv","/d1","/queue","/ai","/protected","/health","/livez","/readyz","/v1/models","/metrics","/logs"]}),
            ),
            "/kv" => kv::handler(&env).await,
            "/d1" => d1::handler(&env).await,
            "/queue" => queue::handler(&env).await,
            "/ai" => ai::handler(&env).await,

            // /metrics: Prometheus text format for all collected metrics.
            "/metrics" => {
                let prom = metrics().export_prometheus();
                let resp = Response::from_bytes(prom.as_bytes().to_vec())?;
                resp.headers().set("content-type", "text/plain; charset=utf-8")?;
                Ok(resp)
            }

            // /logs: recent structured log events in JSON.
            "/logs" => {
                let recent = log_buffer().recent(50);
                json_response(200, &serde_json::json!({"status":"ok","logs":recent}))
            }

            // Deep health: checks ALL bindings accessible with latency.
            "/health" => {
                let results = check_bindings(&env);
                let all_healthy = results.iter().all(|d| d.status == HealthStatus::Healthy);
                let overall = if all_healthy {
                    "healthy"
                } else if results.iter().any(|d| d.status == HealthStatus::Unhealthy) {
                    "unhealthy"
                } else {
                    "degraded"
                };
                let status_code = if all_healthy { 200 } else { 503 };
                json_response(status_code, &serde_json::json!({"status":overall,"checks":results}))
            }
            // Kubernetes liveness: worker responds = alive.
            "/livez" => json_response(200, &serde_json::json!({"status":"alive"})),
            // Kubernetes readiness: same check as /health.
            "/readyz" => {
                let results = check_bindings(&env);
                let all_ready = results.iter().all(|d| d.status == HealthStatus::Healthy);
                let status_str = if all_ready {
                    "ready"
                } else if results.iter().any(|d| d.status == HealthStatus::Unhealthy) {
                    "not_ready"
                } else {
                    "degraded"
                };
                let code = if all_ready { 200 } else { 503 };
                json_response(code, &serde_json::json!({"status":status_str,"checks":results}))
            }
            // OpenAI-compatible model listing for AI SDK compatibility.
            "/v1/models" => json_response(
                200,
                &serde_json::json!({"object":"list","data":[{"id":"@cf/meta/llama-3.1-8b-instruct-fast","name":"Llama 3.1 8B Instruct (Fast)"},{"id":"@cf/meta/llama-3.3-70b-instruct-fp8-fast","name":"Llama 3.3 70B Instruct (FP8 Fast)"},{"id":"@cf/moonshotai/kimi-k2.6","name":"Kimi K2.6"},{"id":"@cf/qwen/qwq-32b","name":"Qwen QwQ 32B"},{"id":"@cf/meta/llama-3.2-3b-instruct","name":"Llama 3.2 3B Instruct"},{"id":"@cf/meta/llama-3.2-1b-instruct","name":"Llama 3.2 1B Instruct"}]}),
            ),

            // Proxy to AWS Lambda via SigV4-signed request (handlers/lambda.rs).
            "/lambda/query" => crate::handlers::lambda::handler(req, &env, &ctx).await,

            // Auth gateway: internal service binding to auth-worker /verify (sub-request, <5ms).
            "/protected" => {
                let auth_worker = env.service("AUTH")?;
                let headers = Headers::new();
                if let Some(auth_header) = req.headers().get("Authorization")? {
                    headers.set("Authorization", &auth_header)?;
                }
                // Propagate trace context to auth-worker.
                if let Ok(Some(tp)) = req.headers().get("traceparent") {
                    let _ = headers.set("traceparent", &tp);
                }
                let mut init = RequestInit::new();
                init.with_method(Method::Get);
                init.with_headers(headers);
                let auth_response = auth_worker
                    .fetch("https://auth-worker.internal/verify", Some(init))
                    .await?;
                if auth_response.status_code() == 200 {
                    json_response(
                        200,
                        &serde_json::json!({"status":"ok","message":"Access granted - token verified via auth-worker","auth_status":auth_response.status_code()}),
                    )
                } else {
                    json_response(
                        403,
                        &serde_json::json!({"status":"error","error":"Access denied - invalid or missing token","auth_status":auth_response.status_code()}),
                    )
                }
            }

            _ => json_response(
                404,
                &serde_json::json!({"status":"error","error":"Not found","available_routes":["/","/kv","/d1","/queue","/ai","/protected","/health","/livez","/readyz","/v1/models","/metrics","/logs"]}),
            ),
        }?;

        // CORS: reflect origin on all responses (safe for API-only worker).
        if req_origin.is_empty() {
            resp.headers().set("Access-Control-Allow-Origin", "*")?;
        } else {
            resp.headers().set("Access-Control-Allow-Origin", &req_origin)?;
        }
        resp.headers()
            .set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")?;
        resp.headers().set(
            "Access-Control-Allow-Headers",
            "Content-Type, Authorization, traceparent, x-request-id",
        )?;

        let duration_ms = Date::now().as_millis() - start_ms;
        let status = resp.status_code();

        // Record metrics.
        let ep = metrics().register(&method, path);
        ep.record(status, start_ms as f64);

        // Structured log for completed response.
        logger().response(&method, path, status, duration_ms, &ctx).emit();
        log_buffer().push(logger().response(&method, path, status, duration_ms, &ctx));

        buffer_event(logger().response(&method, path, status, duration_ms, &ctx));

        resp.headers().set("X-Request-Id", &req_id)?;
        ctx.inject_into_response(&mut resp)?;

        if let Ok(otel_url) = env.var("SIGNOZ_OTEL_ENDPOINT") {
            let url = otel_url.to_string();
            let tc = ctx.clone();
            let m = method.clone();
            let p = path.to_string();
            let nm = format!("{} {}", method, path);
            let st = start_ms as f64;
            let sc = status;
            cx.wait_until(async move {
                let end = Date::now().as_millis() as f64;
                if let Err(e) = export_span(&url, "gateway", &tc, None, &nm, st, end, &m, &p, sc, None).await {
                    console_log!("otel export error: {}", e);
                }
            });
        }

        if let Ok(loki_url) = env.var("LOKI_ENDPOINT") {
            let loki_url = loki_url.to_string();
            let tenant_id = env.var("LOKI_TENANT_ID").ok().map(|v| v.to_string());
            cx.wait_until(async move {
                push_logs(&loki_url, tenant_id.as_deref()).await;
            });
        }

        Ok(resp)
    }
}

fn check_bindings(env: &Env) -> Vec<DependencyHealth> {
    let mut r = Vec::new();
    let mut add = |name: &str, res: Result<(), String>| {
        let s = Date::now().as_millis() as f64;
        match res {
            Ok(()) => r.push(DependencyHealth::healthy(
                name,
                (Date::now().as_millis() as f64 - s) as u64,
            )),
            Err(e) => r.push(DependencyHealth::unhealthy(name, &e)),
        }
    };
    add("kv", env.kv("TEST_KV").map(|_| ()).map_err(|e| format!("{:?}", e)));
    add("d1", env.d1("D1").map(|_| ()).map_err(|e| format!("{:?}", e)));
    add("queue", env.queue("QUEUE").map(|_| ()).map_err(|e| format!("{:?}", e)));
    add("ai", env.ai("AI").map(|_| ()).map_err(|e| format!("{:?}", e)));
    add("auth", env.service("AUTH").map(|_| ()).map_err(|e| format!("{:?}", e)));
    r
}
