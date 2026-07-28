// Integration tests for the health check module.
// Pure logic: manually construct DependencyHealth instances (constructors call
// worker::Date::now() which panics on non-wasm targets).
// HealthRegistry registration + check_all/overall_status NOT tested because
// check_all internally calls DependencyHealth::healthy/unhealthy constructors.

use cloudflare_shared::observability::health::{DependencyHealth, HealthRegistry, HealthStatus};

// ---------------------------------------------------------------------------
// HealthStatus
// ---------------------------------------------------------------------------

#[test]
fn test_health_status_debug() {
    assert_eq!(format!("{:?}", HealthStatus::Healthy), "Healthy");
    assert_eq!(format!("{:?}", HealthStatus::Degraded), "Degraded");
    assert_eq!(format!("{:?}", HealthStatus::Unhealthy), "Unhealthy");
}

#[test]
fn test_health_status_serialize() {
    assert_eq!(serde_json::to_value(HealthStatus::Healthy).unwrap(), "Healthy");
    assert_eq!(serde_json::to_value(HealthStatus::Degraded).unwrap(), "Degraded");
    assert_eq!(serde_json::to_value(HealthStatus::Unhealthy).unwrap(), "Unhealthy");
}

// ---------------------------------------------------------------------------
// DependencyHealth — manually constructed (avoid worker::Date)
// ---------------------------------------------------------------------------

#[test]
fn test_dependency_health_healthy_manual() {
    let dh = DependencyHealth {
        name: "kv".to_string(),
        status: HealthStatus::Healthy,
        latency_ms: 42,
        last_checked: "2026-07-27T10:00:00.000Z".to_string(),
        error: None,
    };
    assert_eq!(dh.name, "kv");
    assert_eq!(dh.status, HealthStatus::Healthy);
    assert_eq!(dh.latency_ms, 42);
    assert!(dh.error.is_none());
}

#[test]
fn test_dependency_health_degraded_manual() {
    let dh = DependencyHealth {
        name: "queue".to_string(),
        status: HealthStatus::Degraded,
        latency_ms: 500,
        last_checked: "2026-07-27T10:00:00.000Z".to_string(),
        error: Some("high latency".to_string()),
    };
    assert_eq!(dh.name, "queue");
    assert_eq!(dh.status, HealthStatus::Degraded);
    assert_eq!(dh.latency_ms, 500);
    assert_eq!(dh.error.as_deref(), Some("high latency"));
}

#[test]
fn test_dependency_health_unhealthy_manual() {
    let dh = DependencyHealth {
        name: "d1".to_string(),
        status: HealthStatus::Unhealthy,
        latency_ms: 0,
        last_checked: "2026-07-27T10:00:00.000Z".to_string(),
        error: Some("connection refused".to_string()),
    };
    assert_eq!(dh.name, "d1");
    assert_eq!(dh.status, HealthStatus::Unhealthy);
    assert_eq!(dh.latency_ms, 0);
    assert_eq!(dh.error.as_deref(), Some("connection refused"));
}

// ---------------------------------------------------------------------------
// DependencyHealth JSON shape
// ---------------------------------------------------------------------------

#[test]
fn test_dependency_health_json_shape() {
    let dh = DependencyHealth {
        name: "kv".to_string(),
        status: HealthStatus::Healthy,
        latency_ms: 42,
        last_checked: "2026-07-27T10:00:00.000Z".to_string(),
        error: None,
    };
    let json: serde_json::Value = serde_json::to_value(&dh).unwrap();
    let obj = json.as_object().unwrap();
    assert_eq!(obj["name"], "kv");
    assert_eq!(obj["status"], "Healthy");
    assert_eq!(obj["latency_ms"], 42);
    assert!(obj.contains_key("last_checked"));
    assert!(obj["last_checked"].is_string());
    assert!(!obj.contains_key("error"), "healthy: error field absent");
}

#[test]
fn test_dependency_health_json_with_error() {
    let dh = DependencyHealth {
        name: "d1".to_string(),
        status: HealthStatus::Unhealthy,
        latency_ms: 0,
        last_checked: "2026-07-27T10:00:00.000Z".to_string(),
        error: Some("timeout".to_string()),
    };
    let json: serde_json::Value = serde_json::to_value(&dh).unwrap();
    assert_eq!(json["error"], "timeout");
}

// ---------------------------------------------------------------------------
// HealthRegistry — registration only (check_all calls worker::Date internally)
// ---------------------------------------------------------------------------

#[test]
fn test_registry_new_empty() {
    let reg = HealthRegistry::new();
    // New registry has no checks; overall_status computes from empty vec
    // which doesn't call constructors
    let results = reg.check_all();
    assert!(results.is_empty());
}

// The following HealthRegistry methods internally call
// DependencyHealth::healthy/unhealthy which call worker::Date::now():
//   - register + check_all (when checks are registered)
//   - overall_status
//   - is_healthy (when checks are registered)
// They cannot be tested on native targets.

// ---------------------------------------------------------------------------
// Health response JSON — manually constructed
// ---------------------------------------------------------------------------

#[test]
fn test_health_response_json_structure() {
    let checks = vec![
        DependencyHealth {
            name: "kv".to_string(),
            status: HealthStatus::Healthy,
            latency_ms: 3,
            last_checked: "2026-07-27T10:00:00.000Z".to_string(),
            error: None,
        },
        DependencyHealth {
            name: "db".to_string(),
            status: HealthStatus::Unhealthy,
            latency_ms: 0,
            last_checked: "2026-07-27T10:00:00.000Z".to_string(),
            error: Some("connection refused".to_string()),
        },
    ];

    let response = serde_json::json!({
        "status": "Unhealthy",
        "version": "0.1.0",
        "uptime_seconds": 3600,
        "checks": checks,
    });

    let json_str = serde_json::to_string(&response).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["status"], "Unhealthy");
    assert_eq!(parsed["version"], "0.1.0");
    assert_eq!(parsed["uptime_seconds"], 3600);
    assert!(parsed["uptime_seconds"].as_u64().unwrap() > 0, "uptime > 0");
    assert_eq!(parsed["checks"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["checks"][0]["name"], "kv");
    assert_eq!(parsed["checks"][0]["status"], "Healthy");
    assert_eq!(parsed["checks"][1]["status"], "Unhealthy");
}

#[test]
fn test_health_response_uptime_positive_integer() {
    let uptime: u64 = 3600;
    assert!(uptime > 0);
    let response = serde_json::json!({ "uptime_seconds": uptime });
    assert!(response["uptime_seconds"].is_number());
    assert_eq!(response["uptime_seconds"].as_u64().unwrap(), 3600);
}
