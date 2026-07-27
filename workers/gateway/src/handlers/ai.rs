use crate::utils::response::json_response;
use cloudflare_shared::bindings::{AiInput, EnvBindings};
use worker::*;

// GET /ai — Workers AI (Llama 3.1 8B)
pub async fn handler(env: &Env) -> Result<Response> {
    let bindings = EnvBindings::from_env(env)?;

    let input = AiInput {
        prompt: "Say hello in exactly 5 words.".to_string(),
        max_tokens: 20,
    };

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
