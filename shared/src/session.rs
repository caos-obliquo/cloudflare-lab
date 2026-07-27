// Token: s2.<base64url({"u":"<user>","e":<exp>,"p":"<purpose>"})>.<base64url(HMAC-SHA256)>

use crate::crypto::{hmac_sign, hmac_verify};

// 7 days default expiry, in seconds.
const DEFAULT_TTL_SECS: u64 = 7 * 24 * 60 * 60;

// Parsed session token.
pub struct SessionToken {
    pub username: String,
    pub expires_at: u64, // unix timestamp
    pub purpose: String,
}

// Create an HMAC-signed session token string.
// secret: HMAC signing key from env var (e.g. SESSION_SECRET, CSRF_SECRET).
// purpose: "session" for auth tokens, "csrf" for CSRF tokens.
// ttl_secs: how long token is valid (default 7 days if 0).
pub fn create_token(secret: &[u8], username: &str, purpose: &str, ttl_secs: u64) -> Result<String, String> {
    let ttl = if ttl_secs == 0 { DEFAULT_TTL_SECS } else { ttl_secs };
    let now_secs = (js_sys::Date::now() / 1000.0) as u64;
    let exp = now_secs + ttl;

    // Build compact JSON payload with short keys.
    let payload = serde_json::json!({"u": username, "e": exp, "p": purpose});

    // Serialize and canonicalize by producing a predictable string.
    // serde_json::to_string produces compact JSON (no whitespace) which is deterministic
    // for the same data - ensuring signing and verification produce identical payloads.
    let payload_str = serde_json::to_string(&payload).map_err(|e| format!("serialize: {}", e))?;

    // Include the purpose tag in the signed data so a token can't be replayed across purposes.
    // Signed data = "purpose:" || payload_json
    let signed_data = format!("{}:{}", purpose, payload_str);

    let signature = hmac_sign(secret, signed_data.as_bytes());

    // base64url encode both parts (no padding = "=" stripped, URL-safe).
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload_b64 = base64::Engine::encode(&URL_SAFE_NO_PAD, payload_str.as_bytes());
    let sig_b64 = base64::Engine::encode(&URL_SAFE_NO_PAD, &signature);

    Ok(format!("s2.{}.{}", payload_b64, sig_b64))
}

// Parse and verify an HMAC-signed token.
// Returns Ok(SessionToken) on success, Err(string) on failure.
// Failures: malformed format, HMAC mismatch, expired, wrong purpose.
pub fn parse_token(token_str: &str, secret: &[u8], expected_purpose: &str) -> Result<SessionToken, String> {
    // Split on dots: s2.<payload>.<signature>
    let parts: Vec<&str> = token_str.splitn(3, '.').collect();
    if parts.len() != 3 || parts[0] != "s2" {
        return Err("bad format".into());
    }

    let payload_b64 = parts[1];
    let sig_b64 = parts[2];

    // Decode base64url payload.
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload_bytes = base64::Engine::decode(&URL_SAFE_NO_PAD, payload_b64).map_err(|_| "bad payload encoding")?;
    let payload_str = String::from_utf8(payload_bytes).map_err(|_| "bad payload utf8")?;

    // Decode signature.
    let signature = base64::Engine::decode(&URL_SAFE_NO_PAD, sig_b64).map_err(|_| "bad sig encoding")?;

    // Verify HMAC: signed_data = purpose + ":" + payload
    // The purpose is taken from the token's payload, NOT from expected_purpose.
    // This verifies the token wasn't tampered with.
    let signed_data = format!("{}:{}", expected_purpose, payload_str);
    if !hmac_verify(secret, signed_data.as_bytes(), &signature) {
        return Err("invalid signature".into());
    }

    // Parse JSON payload.
    let payload: serde_json::Value = serde_json::from_str(&payload_str).map_err(|_| "bad payload json")?;

    let username = payload["u"].as_str().ok_or("missing user")?.to_string();
    let expires_at = payload["e"].as_u64().ok_or("missing exp")?;
    let purpose = payload["p"].as_str().ok_or("missing purpose")?.to_string();

    // Verify purpose matches expected.
    if purpose != expected_purpose {
        return Err(format!(
            "wrong purpose: expected '{}', got '{}'",
            expected_purpose, purpose
        ));
    }

    // Check expiry.
    let now = (js_sys::Date::now() / 1000.0) as u64;
    if now > expires_at {
        return Err("token expired".into());
    }

    Ok(SessionToken {
        username,
        expires_at,
        purpose,
    })
}

// Quick validation for HTTP handlers: parse token, return username or None.
// This is a convenience wrapper for the common case.
// secret: HMAC signing key. expected_purpose: typically "session".
pub fn validate_token(token_str: &str, secret: &[u8], expected_purpose: &str) -> Option<String> {
    parse_token(token_str, secret, expected_purpose)
        .ok()
        .map(|t| t.username)
}
