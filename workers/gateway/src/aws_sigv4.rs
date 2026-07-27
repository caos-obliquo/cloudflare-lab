//! SigV4 signing for Lambda Function URLs. Digest + HMAC-SHA256 via ring.

use ring::hmac;
use ring::digest;
use std::fmt::Write;

pub struct SigV4Signer {
    access_key: String,
    secret_key: String,
    region: String,  // default: us-east-1
    service: String, // default: lambda
}

impl SigV4Signer {
    pub fn new(access_key: &str, secret_key: &str) -> Self {
        Self {
            access_key: access_key.to_string(),
            secret_key: secret_key.to_string(),
            region: "us-east-1".to_string(),
            service: "lambda".to_string(),
        }
    }

    // Returns the Authorization header value for a SigV4-signed request.
    pub fn sign_request(
        &self,
        method: &str,
        url: &str,
        body: &[u8],
        amz_date: &str,
    ) -> String {
        let payload_hash = sha256_hex(body);
        let host = extract_host(url);

        // Headers must be alphabetical (host < x-amz-content-sha256 < x-amz-date).
        let canonical_headers = format!(
            "host:{}\nx-amz-content-sha256:{}\nx-amz-date:{}",
            host, payload_hash, amz_date
        );
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";

        // Canonical request: METHOD + \n + / + \n + (empty query) + \n + headers + \n + signed_headers + \n + payload_hash
        let canonical_request = format!(
            "{}\n/\n\n{}\n{}\n{}",
            method.to_uppercase(),
            canonical_headers,
            signed_headers,
            payload_hash,
        );
        let canonical_hash = sha256_hex(canonical_request.as_bytes());

        // Credential scope bounds signature to date/region/service.
        let credential_scope = format!(
            "{}/{}/{}/aws4_request",
            &amz_date[..8],
            self.region,
            self.service
        );

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date, credential_scope, canonical_hash,
        );

        let signing_key = self.derive_key(&amz_date[..8]);
        let signature = hex_encode(
            hmac::sign(&signing_key, string_to_sign.as_bytes()).as_ref(),
        );

        format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.access_key, credential_scope, signed_headers, signature,
        )
    }

    // Multi-layer key derivation: each HMAC layer narrows scope.
    //   kDate     = HMAC("AWS4" + secret, date)
    //   kRegion   = HMAC(kDate, region)
    //   kService  = HMAC(kRegion, service)
    //   kSigning  = HMAC(kService, "aws4_request")
    fn derive_key(&self, date: &str) -> hmac::Key {
        let k1 = hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, format!("AWS4{}", self.secret_key).as_bytes()),
            date.as_bytes(),
        );
        let k2 = hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, k1.as_ref()),
            self.region.as_bytes(),
        );
        let k3 = hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, k2.as_ref()),
            self.service.as_bytes(),
        );
        hmac::Key::new(
            hmac::HMAC_SHA256,
            hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, k3.as_ref()), b"aws4_request").as_ref(),
        )
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex_encode(digest::digest(&digest::SHA256, data).as_ref())
}

// Strip protocol and path from URL -> bare host.
// "https://abc123.lambda-url.us-east-1.on.aws/" -> "abc123.lambda-url.us-east-1.on.aws"
fn extract_host(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut acc, b| {
        let _ = write!(acc, "{:02x}", b);
        acc
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_host() {
        assert_eq!(extract_host("https://abc123.lambda-url.us-east-1.on.aws/"), "abc123.lambda-url.us-east-1.on.aws");
    }

    #[test]
    fn test_sha256_hex() {
        let result = sha256_hex(b"hello");
        assert_eq!(result.len(), 64);
        assert_eq!(result, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    #[test]
    fn test_signing_key_derivation() {
        let signer = SigV4Signer::new("AKIDEXAMPLE", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY");
        let auth = signer.sign_request("POST", "https://lambda.us-east-1.amazonaws.com/", b"{}", "20260720T120000Z");
        assert!(auth.starts_with("AWS4-HMAC-SHA256"));
    }
}
