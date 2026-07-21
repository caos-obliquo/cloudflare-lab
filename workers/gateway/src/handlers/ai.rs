use crate::utils::response::json_response;
use cloudflare_shared::bindings::{AiInput, EnvBindings};
use worker::*;

// GET /ai: runs Workers AI inference on Llama 3.1 8B.
// Workers AI runs on Cloudflare's GPU network - low latency inference.
pub async fn handler(env: &Env) -> Result<Response> {
    let bindings = EnvBindings::from_env(env)?;

    let input = AiInput {
        prompt: "Say hello in exactly 5 words.".to_string(),
        max_tokens: 20,
    };

    // .run() serializes AiInput to JS object (WASM binding contract).
    // Model response lives in JSON field "response".
    let response: serde_json::Value = bindings
        .ai
        .run("@cf/meta/llama-3.1-8b-instruct-fast", &input)
        .await?;

    let result = response
        .get("response")
        .and_then(|v| v.as_str())
        .unwrap_or("No response from AI");

    json_response(
        200,
        &serde_json::json!({
            "status": "ok",
            "endpoint": "/ai",
            "response": result
        }),
    )
}
