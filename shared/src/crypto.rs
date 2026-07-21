use std::num::NonZeroU32;
use ring::digest;
use ring::hmac;
use ring::pbkdf2;

// PBKDF2-HMAC-SHA512: OWASP-recommended password hashing.
// 100k iterations makes GPU brute-force expensive. 16-byte salt prevents rainbow tables.
const PBKDF2_ALGO: pbkdf2::Algorithm = pbkdf2::PBKDF2_HMAC_SHA512;
const PBKDF2_ITERATIONS: u32 = 100_000;
const SALT_LEN: usize = 16;
const KEY_LEN: usize = 64; // 64 bytes = SHA512 output

// Hash password -> self-describing string: algorithm, params, salt, derived key.
// Format: $pbkdf2-sha512$i=100000$<base64-salt>$<base64-dk>
pub fn hash_password(password: &str) -> String {
    let mut salt = [0u8; SALT_LEN];
    getrandom::getrandom(&mut salt).expect("rng");

    let mut dk = [0u8; KEY_LEN];
    pbkdf2::derive(
        PBKDF2_ALGO,
        NonZeroU32::new(PBKDF2_ITERATIONS).unwrap(),
        &salt,
        password.as_bytes(),
        &mut dk,
    );

    format!(
        "$pbkdf2-sha512$i={}${}${}",
        PBKDF2_ITERATIONS,
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, salt),
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, dk),
    )
}

// Verify password against pbkdf2 hash. Returns false for malformed hashes (no panic).
// Uses constant-time comparison to prevent timing attacks.
pub fn verify_password(password: &str, hash: &str) -> bool {
    let parts: Vec<&str> = hash.split('$').collect();
    if parts.len() != 5 || parts[1] != "pbkdf2-sha512" {
        return false;
    }
    let Ok(iterations) = parts[2].strip_prefix("i=").unwrap_or("").parse::<u32>() else {
        return false;
    };
    let Ok(salt) = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        parts[3],
    ) else {
        return false;
    };
    let Ok(stored_dk) = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        parts[4],
    ) else {
        return false;
    };
    let Some(iterations) = NonZeroU32::new(iterations) else {
        return false;
    };
    pbkdf2::verify(PBKDF2_ALGO, iterations, &salt, password.as_bytes(), &stored_dk).is_ok()
}

// Legacy SHA256 migration. Fires once per old user on their next login -> re-hashed to pbkdf2.
pub fn verify_legacy_sha256(password: &str, hash: &str) -> bool {
    let d = digest::digest(&digest::SHA256, password.as_bytes());
    hex_from_bytes(d.as_ref()) == hash
}

// --- HMAC-SHA256 signing for stateless session tokens ---

const HMAC_TAG: &hmac::Algorithm = &hmac::HMAC_SHA256;

// Sign data with HMAC-SHA256 using the given key.
// The key is used directly as the HMAC key (must be at least 32 bytes for SHA256 security).
// Returns the 32-byte HMAC-SHA256 tag as a Vec<u8>.
pub fn hmac_sign(key: &[u8], data: &[u8]) -> Vec<u8> {
    let k = hmac::Key::new(*HMAC_TAG, key);
    hmac::sign(&k, data).as_ref().to_vec()
}

// Verify HMAC-SHA256 signature in constant time.
// Prevents timing side-channels that could leak the signature byte-by-byte.
// ring::hmac::verify() uses constant-time comparison internally.
pub fn hmac_verify(key: &[u8], data: &[u8], signature: &[u8]) -> bool {
    let k = hmac::Key::new(*HMAC_TAG, key);
    // hmac::verify returns Result<(), Unspecified> - Ok means match, Err means mismatch.
    // Matching on result to avoid unwrap which would panic.
    match hmac::verify(&k, data, signature) {
        Ok(()) => true,
        Err(_) => false,
    }
}

fn hex_from_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
