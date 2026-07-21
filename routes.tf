# Workers routes - closes split-state gap between wrangler and Terraform.
# Script names must match the `name` field in each worker's wrangler.toml.

resource "cloudflare_workers_route" "gateway" {
  zone_id = var.zone_id
  pattern = "${local.gateway}/*"
  script  = "gateway-worker"
}

resource "cloudflare_workers_route" "auth" {
  zone_id = var.zone_id
  pattern = "${local.auth}/*"
  script  = "auth-worker"
}

resource "cloudflare_workers_route" "analytics" {
  zone_id = var.zone_id
  pattern = "${local.analytics}/*"
  script  = "analytics-worker"
}