// Enhanced health checks for Cloudflare Workers.
//
// Provides dependency-level health tracking with latency measurement,
// last-checked timestamps, and error details. Used by the gateway worker's
// /health and /readyz endpoints to report per-binding status.
//
// Design:
// - HealthRegistry holds a list of DependencyHealth entries
// - Each entry tracks: name, status (healthy/degraded/unhealthy), latency_ms,
//   last_checked timestamp, and optional error message
// - Checks are run on-demand (not background-polled) since Workers are
//   stateless and ephemeral

use std::sync::Mutex;

use serde::Serialize;
use worker::Date;

/// Health status for a dependency.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Health information for a single dependency.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyHealth {
    pub name: String,
    pub status: HealthStatus,
    pub latency_ms: u64,
    pub last_checked: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DependencyHealth {
    pub fn healthy(name: &str, latency_ms: u64) -> Self {
        Self {
            name: name.to_string(),
            status: HealthStatus::Healthy,
            latency_ms,
            last_checked: Date::now().to_string(),
            error: None,
        }
    }

    pub fn degraded(name: &str, latency_ms: u64, error: &str) -> Self {
        Self {
            name: name.to_string(),
            status: HealthStatus::Degraded,
            latency_ms,
            last_checked: Date::now().to_string(),
            error: Some(error.to_string()),
        }
    }

    pub fn unhealthy(name: &str, error: &str) -> Self {
        Self {
            name: name.to_string(),
            status: HealthStatus::Unhealthy,
            latency_ms: 0,
            last_checked: Date::now().to_string(),
            error: Some(error.to_string()),
        }
    }
}

/// A health check function for a specific dependency.
/// Returns Ok(latency_ms) on success, Err(error) on failure.
pub type HealthCheckFn = Box<dyn Fn() -> Result<u64, String> + Send>;

/// A registered health check with its name and check function.
pub struct HealthCheck {
    pub name: String,
    pub check: HealthCheckFn,
}

/// Registry of health checks that can be run on demand.
pub struct HealthRegistry {
    checks: Mutex<Vec<HealthCheck>>,
}

impl HealthRegistry {
    pub fn new() -> Self {
        Self {
            checks: Mutex::new(Vec::new()),
        }
    }

    /// Register a health check function.
    pub fn register<F>(&self, name: &str, check: F)
    where
        F: Fn() -> Result<u64, String> + Send + 'static,
    {
        self.checks.lock().unwrap().push(HealthCheck {
            name: name.to_string(),
            check: Box::new(check),
        });
    }

    /// Run all registered health checks and return results.
    pub fn check_all(&self) -> Vec<DependencyHealth> {
        let checks = self.checks.lock().unwrap();
        let mut results = Vec::with_capacity(checks.len());
        for hc in checks.iter() {
            let result = match (hc.check)() {
                Ok(latency_ms) => DependencyHealth::healthy(&hc.name, latency_ms),
                Err(err) => DependencyHealth::unhealthy(&hc.name, &err),
            };
            results.push(result);
        }
        results
    }

    /// Run all checks and return overall status.
    /// Healthy = all healthy, Degraded = any degraded, Unhealthy = any unhealthy.
    pub fn overall_status(&self) -> (HealthStatus, Vec<DependencyHealth>) {
        let results = self.check_all();
        let status = if results.iter().any(|d| d.status == HealthStatus::Unhealthy) {
            HealthStatus::Unhealthy
        } else if results.iter().any(|d| d.status == HealthStatus::Degraded) {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };
        (status, results)
    }

    /// Quick check: is the overall system healthy?
    pub fn is_healthy(&self) -> bool {
        let (status, _) = self.overall_status();
        status == HealthStatus::Healthy
    }
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self::new()
    }
}
