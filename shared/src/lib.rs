// cloudflare-shared is a library crate used by all 3 Workers.
// Each `pub mod` declares a submodule - a separate .rs file in this directory.
// Modules are the unit of organization: related functions grouped together.
// Other crates import via path: cloudflare_shared::crypto::hash_password().
pub mod bindings; // Env structs: typed access to Cloudflare bindings (KV, D1, R2, Queue, AI)
pub mod bootstrap; // D1 table creation on worker startup (CREATE TABLE IF NOT EXISTS)
pub mod crypto; // Password hashing: pbkdf2 (production) + legacy SHA256 (migration)
pub mod error; // AppError enum: typed errors for the whole project
pub mod observability;
pub mod response; // JSON response helpers: standardized format + CORS headers
pub mod session; // HMAC-signed stateless session tokens (no KV lookup)
pub mod tracing; // X-Request-Id: extract from request or generate fresh // W3C trace context, structured logging, metrics, health checks
