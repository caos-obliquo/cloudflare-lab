use crate::handlers::{ai, d1, kv, queue};
use crate::utils::response::json_response;
use cloudflare_shared::tracing::request_id_for_request;
use worker::*;

// Unit struct as namespace for route(). Hand-rolled (no framework) to keep WASM binary small.
pub struct Router;

impl Router {
    pub async fn route(req: Request, env: Env) -> Result<Response> {
        let url = req.url()?;
        let path = url.path();
        let method = req.method().to_string();
        let req_id = request_id_for_request(&req)?;
        console_log!("[req-{}] handling {} {}", req_id, method, path);

        // CORS preflight: browser sends OPTIONS before cross-origin requests.
        if req.method() == Method::Options {
            let headers = Headers::new();
            headers.set("Access-Control-Allow-Origin", "*")?;
            headers.set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")?;
            headers.set("Access-Control-Allow-Headers", "Content-Type, Authorization")?;
            let resp = Response::empty()?.with_status(204).with_headers(headers);
            resp.headers().set("X-Request-Id", &req_id)?;
            return Ok(resp);
        }

        let resp = match path {
            "/" => json_response(200, &serde_json::json!({"status":"ok","service":"gateway-worker","language":"rust","routes":["/kv","/d1","/queue","/ai","/protected","/health","/livez","/readyz","/v1/models"]})),
            "/kv" => kv::handler(&env).await,
            "/d1" => d1::handler(&env).await,
            "/queue" => queue::handler(&env).await,
            "/ai" => ai::handler(&env).await,

            // Deep health: checks ALL bindings accessible. 200 if all ok, 503 if any missing.
            "/health" => {
                let kv_ok = env.kv("TEST_KV").is_ok();
                let d1_ok = env.d1("D1").is_ok();
                let queue_ok = env.queue("QUEUE").is_ok();
                let ai_ok = env.ai("AI").is_ok();
                let auth_ok = env.service("AUTH").is_ok();
                let all_ok = kv_ok && d1_ok && queue_ok && ai_ok && auth_ok;
                json_response(if all_ok { 200 } else { 503 }, &serde_json::json!({"status":if all_ok{"healthy"}else{"degraded"},"bindings":{"kv":kv_ok,"d1":d1_ok,"queue":queue_ok,"ai":ai_ok,"auth":auth_ok}}))
            }
            // Kubernetes liveness: worker responds = alive.
            "/livez" => json_response(200, &serde_json::json!({"status":"alive"})),
            // Kubernetes readiness: same check as /health.
            "/readyz" => {
                let kv_ok = env.kv("TEST_KV").is_ok();
                let d1_ok = env.d1("D1").is_ok();
                let queue_ok = env.queue("QUEUE").is_ok();
                let ai_ok = env.ai("AI").is_ok();
                let auth_ok = env.service("AUTH").is_ok();
                let all_ok = kv_ok && d1_ok && queue_ok && ai_ok && auth_ok;
                json_response(if all_ok { 200 } else { 503 }, &serde_json::json!({"status":if all_ok{"ready"}else{"not_ready"},"checks":{"kv":kv_ok,"d1":d1_ok,"queue":queue_ok,"ai":ai_ok,"auth":auth_ok}}))
            }
            // OpenAI-compatible model listing for AI SDK compatibility.
            "/v1/models" => json_response(200, &serde_json::json!({"object":"list","data":[{"id":"@cf/meta/llama-3.1-8b-instruct-fast","name":"Llama 3.1 8B Instruct (Fast)"},{"id":"@cf/meta/llama-3.3-70b-instruct-fp8-fast","name":"Llama 3.3 70B Instruct (FP8 Fast)"},{"id":"@cf/moonshotai/kimi-k2.6","name":"Kimi K2.6"},{"id":"@cf/qwen/qwq-32b","name":"Qwen QwQ 32B"},{"id":"@cf/meta/llama-3.2-3b-instruct","name":"Llama 3.2 3B Instruct"},{"id":"@cf/meta/llama-3.2-1b-instruct","name":"Llama 3.2 1B Instruct"}]})),

            // Proxy to AWS Lambda via SigV4-signed request (handlers/lambda.rs).
            "/lambda/query" => crate::handlers::lambda::handler(req, &env).await,

            // Auth gateway: internal service binding to auth-worker /verify (sub-request, <5ms).
            "/protected" => {
                let auth_worker = env.service("AUTH")?;
                let headers = Headers::new();
                if let Some(auth_header) = req.headers().get("Authorization")? {
                    headers.set("Authorization", &auth_header)?;
                }
                let mut init = RequestInit::new();
                init.with_method(Method::Get);
                init.with_headers(headers);
                let auth_response = auth_worker.fetch("https://auth-worker.internal/verify", Some(init)).await?;
                if auth_response.status_code() == 200 {
                    json_response(200, &serde_json::json!({"status":"ok","message":"Access granted - token verified via auth-worker","auth_status":auth_response.status_code()}))
                } else {
                    json_response(403, &serde_json::json!({"status":"error","error":"Access denied - invalid or missing token","auth_status":auth_response.status_code()}))
                }
            }

            _ => json_response(404, &serde_json::json!({"status":"error","error":"Not found","available_routes":["/","/kv","/d1","/queue","/ai","/protected","/health","/livez","/readyz","/v1/models"]})),
        }?;
        resp.headers().set("X-Request-Id", &req_id)?;
        Ok(resp)
    }
}
