// k6 load test for Cloudflare Workers gateway.
//
// Simulates user traffic against gateway worker endpoints and enforces
// SLO thresholds: p99 latency < 500ms, error rate < 1%.
//
// Run: k6 run k6/load.js
// Override target: GATEWAY_URL=http://192.168.1.100:8787 k6 run k6/load.js

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Rate } from 'k6/metrics';

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const GATEWAY_URL = __ENV.GATEWAY_URL || 'http://127.0.0.1:8787';
const API_TOKEN = __ENV.API_TOKEN || '';

// Custom metrics for per-route checks
const healthCheck = new Rate('health_route_check');
const apiWorkersCheck = new Rate('api_workers_route_check');

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

export const options = {
  stages: [
    { duration: '30s', target: 5 },   // Ramp up to 5 VUs
    { duration: '2m', target: 10 },    // Stay at 10 VUs
    { duration: '30s', target: 0 },    // Ramp down
  ],
  thresholds: {
    // Global latency: p99 < 500ms
    http_req_duration: ['p(99)<500'],
    // Global failure rate < 1%
    http_req_failed: ['rate<0.01'],
    // Per-route checks: >= 99% success
    'health_route_check': ['rate>=0.99'],
    'api_workers_route_check': ['rate>=0.99'],
  },
  tags: {
    // Default tags applied to all requests
    test_run: `cloudflare-lab-${Date.now()}`,
  },
};

// ---------------------------------------------------------------------------
// Setup — validate gateway reachable before starting VUs
// ---------------------------------------------------------------------------

export function setup() {
  const res = http.get(`${GATEWAY_URL}/health`, { tags: { route: 'setup' } });

  if (res.status === 200 || res.status === 401 || res.status === 403) {
    // 401/403 means the gateway is reachable even if auth rejects anon
    console.log(`Gateway reachable at ${GATEWAY_URL} (status ${res.status})`);
    return { ok: true };
  }

  const msg = `Gateway unreachable: ${GATEWAY_URL}/health returned ${res.status} — aborting`;
  console.error(msg);
  // Throw in setup() aborts the entire test cleanly
  throw new Error(msg);
}

// ---------------------------------------------------------------------------
// Main VU code
// ---------------------------------------------------------------------------

export default function (data) {
  if (!data || !data.ok) {
    // Setup failed; skip gracefully (should not be reached if setup threw)
    sleep(1);
    return;
  }

  // --- Route: GET /health ---
  {
    const params = { tags: { route: 'health' } };
    const res = http.get(`${GATEWAY_URL}/health`, params);
    const passed = check(res, {
      'health status is OK': (r) =>
        r.status === 200 || r.status === 401 || r.status === 403,
    });
    healthCheck.add(passed, { route: 'health' });
  }

  sleep(0.5);

  // --- Route: GET /api/workers ---
  {
    const params = {
      tags: { route: 'api_workers' },
      headers: API_TOKEN ? { Authorization: `Bearer ${API_TOKEN}` } : {},
    };
    const res = http.get(`${GATEWAY_URL}/api/workers`, params);
    // Tolerate 401/403 as "reachable" — the endpoint may require auth
    const passed = check(res, {
      'api/workers status is OK': (r) =>
        r.status === 200 || r.status === 401 || r.status === 403,
    });
    apiWorkersCheck.add(passed, { route: 'api_workers' });
  }

  sleep(1);
}
