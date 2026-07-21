# 1. Workers KV Namespace
resource "cloudflare_workers_kv_namespace" "test_kv" {
  account_id = var.account_id
  title      = "test-kv-namespace"
}

resource "cloudflare_workers_kv_namespace" "sessions_kv" {
  account_id = var.account_id
  title      = "sessions-kv-namespace"
}

# 2. D1 Database
resource "cloudflare_d1_database" "test_d1" {
  account_id            = var.account_id
  name                  = "test-d1-database"
  primary_location_hint = "wnam"
  read_replication      = { mode = "disabled" }
}

# 3. R2 Bucket — disabled: R2 not enabled on this account (403 "enable R2 through Dashboard").
# Uncomment after enabling R2 via Cloudflare Dashboard (no API enablement).
# resource "cloudflare_r2_bucket" "test_r2" {
#   account_id = var.account_id
#   name       = "test-r2-bucket"
#   location   = "WNAM"
# }

# 4. Queue
resource "cloudflare_queue" "test_queue" {
  account_id = var.account_id
  queue_name = "test-queue"
}

# 5. AI Gateway
resource "cloudflare_ai_gateway" "test_ai_gateway" {
  account_id                 = var.account_id
  id                         = "test-ai-gateway"
  rate_limiting_interval     = 60
  rate_limiting_limit        = 1000
  collect_logs               = true
  cache_ttl                  = 3600
  cache_invalidate_on_update = false
  logpush                    = false
}
