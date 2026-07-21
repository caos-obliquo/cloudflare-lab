use worker::*;

// CSPRNG 16-byte request ID (32 hex chars). NOT UUID v4 - no dashes, no version bits.
// getrandom: browser crypto.getRandomValues on WASM, getrandom syscall on Linux.
pub fn generate_request_id() -> String {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).expect("getrandom failed");
    buf.iter().map(|b| format!("{:02x}", b)).collect::<String>()
}

// Propagate existing X-Request-Id or generate fresh. Called at handler entry.
pub fn request_id_for_request(req: &Request) -> Result<String> {
    match req.headers().get("X-Request-Id")? {
        Some(id) if !id.is_empty() => Ok(id),
        _ => Ok(generate_request_id()),
    }
}
