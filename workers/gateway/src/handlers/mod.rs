// Gateway route handlers, one per Cloudflare binding type.
// routes.rs dispatches HTTP requests to these by path.
pub mod ai;
pub mod d1;
pub mod kv;
pub mod lambda;
pub mod queue;
