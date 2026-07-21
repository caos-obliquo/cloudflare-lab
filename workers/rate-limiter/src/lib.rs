// Rate limiter using Durable Object for atomic, globally-consistent limit tracking.
// Each DO instance tracks one key (IP:route combination).
// Single-threaded DO ensures no concurrent increments can bypass the limit.

use serde::Deserialize;
use worker::*;

#[derive(Deserialize)]
struct CheckRequest { limit: u64, window: u64 }

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct RateState { count: u64, reset_at: u64 }

#[durable_object]
pub struct RateLimiter {
    state: State,
    _env: Env,
}

impl DurableObject for RateLimiter {
    fn new(state: State, env: Env) -> Self {
        Self { state, _env: env }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let body: CheckRequest = req.json().await?;
        let now = js_sys::Date::now() as u64 / 1000;

        let mut st: RateState = self.state.storage().get("state").await?.unwrap_or_default();
        if now >= st.reset_at {
            st = RateState { count: 0, reset_at: now + body.window };
        }

        let allowed = st.count < body.limit;
        if allowed {
            st.count += 1;
            self.state.storage().put("state", &st).await?;
        }

        let remaining = if allowed { body.limit - st.count } else { 0 };

        Response::from_json(&serde_json::json!({"allowed": allowed, "remaining": remaining}))
    }
}

#[event(fetch)]
async fn fetch(_req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    Response::from_json(&serde_json::json!({"status": "ok", "service": "rate-limiter"}))
}
