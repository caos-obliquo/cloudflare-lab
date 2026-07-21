// Lambda custom runtime: binary MUST be named "bootstrap".
// Build: cargo build --release -p devops-api -> target/release/bootstrap
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use serde_json::json;
use std::env;

async fn handler(req: Request) -> Result<Response<Body>, Error> {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();

    match (method.as_str(), path.as_str()) {
        ("GET", "/health") => ok(json!({"status":"ok","service":"devops-api"})),

        ("GET", "/config") => {
            let config = json!({
                "environment": env::var("ENVIRONMENT").unwrap_or_default(),
                "worker_gateway_url": env::var("WORKER_GATEWAY_URL").unwrap_or_default(),
                "worker_auth_url": env::var("WORKER_AUTH_URL").unwrap_or_default(),
            });
            ok(config)
        }

        // Proxy stubs - forward to Workers via reqwest when T25 wired.
        ("POST", "/workers/query") => {
            ok(json!({"status":"ok","message":"worker proxy endpoint","note":"wired by T25"}))
        }
        ("POST", "/d1/query") => {
            ok(json!({"status":"ok","message":"d1 proxy endpoint","note":"wired by T25"}))
        }
        ("POST", "/workers/register") => {
            ok(json!({"status":"ok","message":"register proxy endpoint","note":"wired by T25"}))
        }

        _ => {
            let body = json!({"status":"error","error":"not found","path":path});
            let resp = Response::builder()
                .status(404)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body)?))?;
            Ok(resp)
        }
    }
}

// 200 JSON response with trace ID. NOT async (no await).
fn ok(body: serde_json::Value) -> Result<Response<Body>, Error> {
    let resp = Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .header("x-request-id", uuid_v4())
        .body(Body::from(serde_json::to_string(&body)?))?;
    Ok(resp)
}

// Timestamp-based request ID (not crypto, not UUID). "lam-" prefix distinguishes from Worker IDs.
fn uuid_v4() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("lam-{:016x}", ts)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(handler)).await
}
