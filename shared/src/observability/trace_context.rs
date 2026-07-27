// W3C Trace Context propagation for Cloudflare Workers.
//
// Implements the W3C traceparent header format:
//   traceparent: 00-<trace_id_32hex>-<span_id_16hex>-<trace_flags_2hex>
//
// This is the same format used by OpenTelemetry, enabling compatibility
// with any OTel-compatible backend (SigNoz, Jaeger, Zipkin, etc.).
//
// Trace ID: 16 random bytes (32 hex chars) — globally unique per request tree.
// Span ID:  8 random bytes (16 hex chars) — unique per service hop.
// Flags:    01 = sampled (recorded), 00 = not sampled.

use getrandom::getrandom;

/// Represents a parsed or generated W3C trace context.
#[derive(Debug, Clone)]
pub struct TraceContext {
    /// 16-byte trace ID as 32 hex chars.
    pub trace_id: String,
    /// 8-byte span ID as 16 hex chars.
    pub span_id: String,
    /// Trace flags: 01 = sampled, 00 = not sampled.
    pub trace_flags: String,
}

impl TraceContext {
    /// Generate a fresh trace context with a new trace ID and root span.
    /// Default: sampled (01) so all traces are recorded.
    pub fn new() -> Self {
        Self {
            trace_id: generate_hex(16),
            span_id: generate_hex(8),
            trace_flags: "01".to_string(),
        }
    }

    /// Generate a child span within the same trace.
    /// Keeps the same trace_id and trace_flags, generates a new span_id.
    pub fn child_span(&self) -> Self {
        Self {
            trace_id: self.trace_id.clone(),
            span_id: generate_hex(8),
            trace_flags: self.trace_flags.clone(),
        }
    }

    /// Parse a W3C traceparent header value.
    /// Format: "00-<trace_id>-<span_id>-<flags>"
    /// Returns None if the header is missing or malformed.
    pub fn from_traceparent(header: &str) -> Option<Self> {
        let parts: Vec<&str> = header.splitn(4, '-').collect();
        if parts.len() != 4 {
            return None;
        }
        // Version must be 00 (or 01+ for future, but we only support 00).
        if parts[0] != "00" {
            return None;
        }
        let trace_id = parts[1].to_string();
        let span_id = parts[2].to_string();
        let trace_flags = parts[3].to_string();

        // Validate lengths.
        if trace_id.len() != 32 || span_id.len() != 16 || trace_flags.len() != 2 {
            return None;
        }
        // Validate hex characters.
        if !trace_id.chars().all(|c| c.is_ascii_hexdigit())
            || !span_id.chars().all(|c| c.is_ascii_hexdigit())
            || !trace_flags.chars().all(|c| c.is_ascii_hexdigit())
        {
            return None;
        }

        Some(Self {
            trace_id,
            span_id,
            trace_flags,
        })
    }

    /// Format as a W3C traceparent header value.
    pub fn to_traceparent(&self) -> String {
        format!("00-{}-{}-{}", self.trace_id, self.span_id, self.trace_flags)
    }

    /// Extract trace context from request headers.
    /// If X-Trace-Id is present, use it as the trace_id and generate a new span.
    /// Otherwise, generate a fresh trace context.
    pub fn from_request(req: &worker::Request) -> Result<Self, String> {
        // Try W3C traceparent header first.
        if let Ok(Some(header)) = req.headers().get("traceparent") {
            if let Some(ctx) = Self::from_traceparent(&header) {
                return Ok(ctx);
            }
        }
        // Fallback: check X-Trace-Id (legacy).
        if let Ok(Some(trace_id)) = req.headers().get("X-Trace-Id") {
            if trace_id.len() == 32 && trace_id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Ok(Self {
                    trace_id,
                    span_id: generate_hex(8),
                    trace_flags: "01".to_string(),
                });
            }
        }
        // Generate fresh.
        Ok(Self::new())
    }

    /// Set traceparent header on an outgoing request for propagation.
    pub fn inject_into_request(&self, req: &mut worker::Request) -> Result<(), String> {
        req.headers()
            .set("traceparent", &self.to_traceparent())
            .map_err(|e| format!("failed to set traceparent: {:?}", e))
    }

    /// Set traceparent header on an outgoing response.
    pub fn inject_into_response(&self, resp: &mut worker::Response) -> Result<(), String> {
        resp.headers()
            .set("traceparent", &self.to_traceparent())
            .map_err(|e| format!("failed to set traceparent: {:?}", e))
    }
}

impl Default for TraceContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a hex string of `byte_count` random bytes.
/// Result is `byte_count * 2` hex chars long.
fn generate_hex(byte_count: usize) -> String {
    let mut buf = vec![0u8; byte_count];
    getrandom(&mut buf).expect("getrandom failed for trace context");
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_trace_context() {
        let ctx = TraceContext::new();
        assert_eq!(ctx.trace_id.len(), 32);
        assert_eq!(ctx.span_id.len(), 16);
        assert_eq!(ctx.trace_flags, "01");
        assert!(ctx.trace_id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(ctx.span_id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_child_span() {
        let parent = TraceContext::new();
        let child = parent.child_span();
        assert_eq!(child.trace_id, parent.trace_id);
        assert_ne!(child.span_id, parent.span_id);
        assert_eq!(child.trace_flags, parent.trace_flags);
    }

    #[test]
    fn test_roundtrip_traceparent() {
        let ctx = TraceContext::new();
        let header = ctx.to_traceparent();
        let parsed = TraceContext::from_traceparent(&header).unwrap();
        assert_eq!(parsed.trace_id, ctx.trace_id);
        assert_eq!(parsed.span_id, ctx.span_id);
        assert_eq!(parsed.trace_flags, ctx.trace_flags);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(TraceContext::from_traceparent("").is_none());
        assert!(TraceContext::from_traceparent("00-abc-123-01").is_none());
        assert!(TraceContext::from_traceparent("01-abc-def-01").is_none());
    }
}
