// Integration tests for W3C Trace Context.
// Pure logic — getrandom works on native (uses OS entropy when js feature irrelevant).

use cloudflare_shared::observability::trace_context::TraceContext;

// ---------------------------------------------------------------------------
// SpanContext::new — unique IDs
// ---------------------------------------------------------------------------

#[test]
fn test_new_trace_context_fields() {
    let ctx = TraceContext::new();
    assert_eq!(ctx.trace_id.len(), 32, "trace_id must be 32 hex chars");
    assert_eq!(ctx.span_id.len(), 16, "span_id must be 16 hex chars");
    assert_eq!(ctx.trace_flags, "01", "default trace_flags should be sampled");
    assert!(
        ctx.trace_id.chars().all(|c| c.is_ascii_hexdigit()),
        "trace_id must be hex"
    );
    assert!(
        ctx.span_id.chars().all(|c| c.is_ascii_hexdigit()),
        "span_id must be hex"
    );
}

#[test]
fn test_new_generates_unique_ids() {
    let a = TraceContext::new();
    let b = TraceContext::new();
    assert_ne!(a.trace_id, b.trace_id, "consecutive trace_ids must differ");
    assert_ne!(a.span_id, b.span_id, "consecutive span_ids must differ");
}

// ---------------------------------------------------------------------------
// Child span
// ---------------------------------------------------------------------------

#[test]
fn test_child_span_inherits_trace() {
    let parent = TraceContext::new();
    let child = parent.child_span();
    assert_eq!(child.trace_id, parent.trace_id, "child shares trace_id");
    assert_ne!(child.span_id, parent.span_id, "child has new span_id");
    assert_eq!(child.trace_flags, parent.trace_flags, "child shares flags");
}

// ---------------------------------------------------------------------------
// Valid traceparent parsing
// ---------------------------------------------------------------------------

#[test]
fn test_parse_valid_traceparent() {
    let tid = "0af7651916cd43dd8448eb211c80319c";
    let sid = "b7ad6b7169203331";
    let header = format!("00-{}-{}-01", tid, sid);
    let ctx = TraceContext::from_traceparent(&header).expect("valid traceparent");
    assert_eq!(ctx.trace_id, tid);
    assert_eq!(ctx.span_id, sid);
    assert_eq!(ctx.trace_flags, "01");
}

#[test]
fn test_parse_valid_traceparent_not_sampled() {
    let header = "00-00000000000000000000000000000000-0000000000000000-00";
    let ctx = TraceContext::from_traceparent(header).expect("valid not-sampled");
    assert_eq!(ctx.trace_flags, "00");
}

// ---------------------------------------------------------------------------
// Invalid traceparent rejection
// ---------------------------------------------------------------------------

#[test]
fn test_parse_wrong_version() {
    assert!(
        TraceContext::from_traceparent("01-00000000000000000000000000000000-0000000000000000-01").is_none(),
        "version 01 should be rejected"
    );
    assert!(
        TraceContext::from_traceparent("ff-00000000000000000000000000000000-0000000000000000-01").is_none(),
        "version ff should be rejected"
    );
}

#[test]
fn test_parse_empty_string() {
    assert!(TraceContext::from_traceparent("").is_none());
}

#[test]
fn test_parse_too_few_parts() {
    assert!(TraceContext::from_traceparent("00-abc").is_none());
    assert!(TraceContext::from_traceparent("00-abc-def").is_none());
}

#[test]
fn test_parse_too_many_parts() {
    // 5 parts is invalid
    assert!(TraceContext::from_traceparent("00-a-b-c-d").is_none());
}

#[test]
fn test_parse_bad_length_trace_id() {
    // 31 hex chars (should be 32)
    assert!(
        TraceContext::from_traceparent("00-0000000000000000000000000000000-0000000000000000-01").is_none()
    );
    // 33 hex chars
    assert!(
        TraceContext::from_traceparent("00-000000000000000000000000000000000-0000000000000000-01").is_none()
    );
}

#[test]
fn test_parse_bad_length_span_id() {
    // 15 hex chars (should be 16)
    assert!(
        TraceContext::from_traceparent("00-00000000000000000000000000000000-000000000000000-01").is_none()
    );
    // 17 hex chars
    assert!(
        TraceContext::from_traceparent("00-00000000000000000000000000000000-00000000000000000-01").is_none()
    );
}

#[test]
fn test_parse_bad_length_flags() {
    // 1 hex char
    assert!(
        TraceContext::from_traceparent("00-00000000000000000000000000000000-0000000000000000-1").is_none()
    );
    // 3 hex chars
    assert!(
        TraceContext::from_traceparent("00-00000000000000000000000000000000-0000000000000000-111").is_none()
    );
}

#[test]
fn test_parse_non_hex_trace_id() {
    assert!(
        TraceContext::from_traceparent("00-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-0000000000000000-01").is_none(),
        "non-hex trace_id should be rejected"
    );
}

#[test]
fn test_parse_non_hex_span_id() {
    assert!(
        TraceContext::from_traceparent("00-00000000000000000000000000000000-xxxxxxxxxxxxxxxx-01").is_none(),
        "non-hex span_id should be rejected"
    );
}

#[test]
fn test_parse_non_hex_flags() {
    assert!(
        TraceContext::from_traceparent("00-00000000000000000000000000000000-0000000000000000-xx").is_none(),
        "non-hex flags should be rejected"
    );
}

#[test]
fn test_parse_uppercase_hex() {
    let header = "00-00000000000000000000000000000000-ABCDEFABCDEFABCD-01";
    let ctx = TraceContext::from_traceparent(header);
    assert!(ctx.is_some(), "uppercase hex should be accepted");
    assert_eq!(ctx.unwrap().span_id, "ABCDEFABCDEFABCD");
}

// ---------------------------------------------------------------------------
// Roundtrip
// ---------------------------------------------------------------------------

#[test]
fn test_to_traceparent_roundtrip() {
    let ctx = TraceContext::new();
    let header = ctx.to_traceparent();
    let parsed = TraceContext::from_traceparent(&header).expect("roundtrip parse");
    assert_eq!(parsed.trace_id, ctx.trace_id);
    assert_eq!(parsed.span_id, ctx.span_id);
    assert_eq!(parsed.trace_flags, ctx.trace_flags);
}

#[test]
fn test_to_traceparent_format() {
    let ctx = TraceContext {
        trace_id: "a".repeat(32),
        span_id: "b".repeat(16),
        trace_flags: "01".to_string(),
    };
    let header = ctx.to_traceparent();
    assert_eq!(header.len(), 55, "00-<32>-<16>-01 = 2+1+32+1+16+1+2 = 55");
    assert!(header.starts_with("00-"));
    assert_eq!(&header[3..35], "a".repeat(32));
}

// ---------------------------------------------------------------------------
// TraceFlags edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_parse_various_flags() {
    // Any 2 hex chars for flags should be accepted
    let valid_flags = ["00", "01", "ff", "ab"];
    for flags in &valid_flags {
        let header = format!(
            "00-00000000000000000000000000000000-0000000000000000-{}",
            flags
        );
        let ctx = TraceContext::from_traceparent(&header);
        assert!(ctx.is_some(), "flags {} should be valid", flags);
    }
}

// ---------------------------------------------------------------------------
// Default trait
// ---------------------------------------------------------------------------

#[test]
fn test_default_is_same_as_new() {
    let a = TraceContext::default();
    let b = TraceContext::new();
    assert_eq!(a.trace_id.len(), b.trace_id.len());
    assert_eq!(a.span_id.len(), b.span_id.len());
    assert_eq!(a.trace_flags, b.trace_flags);
}
