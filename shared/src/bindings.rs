use serde::Serialize;
use worker::*;

// Typed AI input for worker-rs WASM boundary (needs struct, not json!).
#[derive(Serialize)]
pub struct AiInput {
    pub prompt: String,
    pub max_tokens: u32,
}

// Bundles gateway bindings (names must match wrangler.toml exactly: TEST_KV, D1, QUEUE, AI).
pub struct EnvBindings {
    pub kv: KvStore,
    pub d1: D1Database,
    pub queue: Queue,
    pub ai: Ai,
}

impl EnvBindings {
    pub fn from_env(env: &Env) -> Result<Self> {
        Ok(Self {
            kv: env.kv("TEST_KV")?,
            d1: env.d1("D1")?,
            queue: env.queue("QUEUE")?,
            ai: env.ai("AI")?,
        })
    }
}
