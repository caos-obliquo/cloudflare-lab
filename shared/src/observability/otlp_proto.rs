//! OTLP protobuf message types for the ExportTraceServiceRequest.
//!
//! Minimal subset of the OTLP protobuf spec needed to export a single span.
//! Uses `prost` for encoding. See:
//!   https://github.com/open-telemetry/opentelemetry-proto/blob/main/opentelemetry/proto/trace/v1/trace.proto
//!   https://github.com/open-telemetry/opentelemetry-proto/blob/main/opentelemetry/proto/collector/trace/v1/trace_service.proto

use prost::Message;

// ---------------------------------------------------------------------------
// ExportTraceServiceRequest (top-level)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct ExportTraceServiceRequest {
    #[prost(message, repeated, tag = "1")]
    pub resource_spans: Vec<ResourceSpans>,
}

// ---------------------------------------------------------------------------
// ResourceSpans
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct ResourceSpans {
    #[prost(message, optional, tag = "1")]
    pub resource: Option<Resource>,
    #[prost(message, repeated, tag = "2")]
    pub scope_spans: Vec<ScopeSpans>,
}

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct Resource {
    #[prost(message, repeated, tag = "1")]
    pub attributes: Vec<KeyValue>,
}

// ---------------------------------------------------------------------------
// ScopeSpans
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct ScopeSpans {
    #[prost(message, optional, tag = "1")]
    pub scope: Option<Scope>,
    #[prost(message, repeated, tag = "2")]
    pub spans: Vec<Span>,
}

// ---------------------------------------------------------------------------
// InstrumentationScope
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct Scope {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub version: String,
}

// ---------------------------------------------------------------------------
// Span (core trace span)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct Span {
    /// Trace ID in binary (16 bytes).
    #[prost(bytes, tag = "1")]
    pub trace_id: Vec<u8>,
    /// Span ID in binary (8 bytes).
    #[prost(bytes, tag = "2")]
    pub span_id: Vec<u8>,
    /// W3C trace state (opaque string, often empty).
    #[prost(string, tag = "3")]
    pub trace_state: String,
    /// Parent span ID (8 bytes). Empty = root span.
    #[prost(bytes, tag = "4")]
    pub parent_span_id: Vec<u8>,
    /// Span name (e.g. HTTP method + path).
    #[prost(string, tag = "5")]
    pub name: String,
    /// Span kind (0=Unspecified, 1=Internal, 2=Server, 3=Client, 4=Producer, 5=Consumer).
    #[prost(int32, tag = "6")]
    pub kind: i32,
    /// Start time as unix nano (fixed64 for exact encoding).
    #[prost(fixed64, tag = "7")]
    pub start_time_unix_nano: u64,
    /// End time as unix nano (fixed64).
    #[prost(fixed64, tag = "8")]
    pub end_time_unix_nano: u64,
    /// Span attributes (key-value pairs).
    #[prost(message, repeated, tag = "9")]
    pub attributes: Vec<KeyValue>,
    /// Number of dropped attributes (usually 0).
    #[prost(uint32, tag = "10")]
    pub dropped_attributes_count: u32,
    /// Span status (OK / Error / Unset).
    /// NOTE: field 15 in OTLP v1.44.0 (pre-stabilization field numbering).
    #[prost(message, optional, tag = "15")]
    pub status: Option<Status>,
}

pub mod span {
    /// SpanKind enum values matching the OTLP protobuf spec.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum Kind {
        #[default]
        Unspecified = 0,
        Internal = 1,
        Server = 2,
        Client = 3,
        Producer = 4,
        Consumer = 5,
    }

    impl Kind {
        pub fn as_str(&self) -> &'static str {
            match self {
                Kind::Unspecified => "SPAN_KIND_UNSPECIFIED",
                Kind::Internal => "SPAN_KIND_INTERNAL",
                Kind::Server => "SPAN_KIND_SERVER",
                Kind::Client => "SPAN_KIND_CLIENT",
                Kind::Producer => "SPAN_KIND_PRODUCER",
                Kind::Consumer => "SPAN_KIND_CONSUMER",
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct Status {
    /// 0=Unset, 1=Ok, 2=Error.
    /// NOTE: field 3 in OTLP v1.44.0 (not field 1 as in later versions).
    #[prost(int32, tag = "3")]
    pub code: i32,
    /// Error description (set when code=2).
    #[prost(string, tag = "2")]
    pub message: String,
}



// ---------------------------------------------------------------------------
// KeyValue
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct KeyValue {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(message, optional, tag = "2")]
    pub value: Option<AnyValue>,
}

// ---------------------------------------------------------------------------
// AnyValue (oneof wrapper)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct AnyValue {
    #[prost(oneof = "any_value::Value", tags = "1, 2, 3, 4")]
    pub value: Option<any_value::Value>,
}

pub mod any_value {
    use prost::Oneof;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Value {
        #[prost(string, tag = "1")]
        StringValue(String),
        #[prost(int64, tag = "2")]
        IntValue(i64),
        #[prost(double, tag = "3")]
        DoubleValue(f64),
        #[prost(bool, tag = "4")]
        BoolValue(bool),
    }
}

// ---------------------------------------------------------------------------
// Helper constructors
// ---------------------------------------------------------------------------

/// Build an OTLP KeyValue with a string value.
pub fn kv_str(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(value.to_string())),
        }),
    }
}

/// Build an OTLP KeyValue with an int value.
pub fn kv_int(key: &str, value: i64) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(any_value::Value::IntValue(value)),
        }),
    }
}
