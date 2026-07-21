data "cloudflare_zone" "primary" {
  zone_id = var.zone_id
}

locals {
  env       = terraform.workspace == "default" ? "" : "-${terraform.workspace}"
  apex      = data.cloudflare_zone.primary.name
  gateway   = "gateway.${data.cloudflare_zone.primary.name}"
  auth      = "auth.${data.cloudflare_zone.primary.name}"
  analytics = "analytics.${data.cloudflare_zone.primary.name}"
}

# Proxied A records for worker subdomains.
# IP irrelevant when proxied - CF edge resolves via workers_route.
resource "cloudflare_dns_record" "worker_gateway" {
  zone_id = var.zone_id
  name    = local.gateway
  content = "192.0.2.1"
  type    = "A"
  proxied = true
  ttl     = 1
}

resource "cloudflare_dns_record" "worker_auth" {
  zone_id = var.zone_id
  name    = local.auth
  content = "192.0.2.1"
  type    = "A"
  proxied = true
  ttl     = 1
}

resource "cloudflare_dns_record" "worker_analytics" {
  zone_id = var.zone_id
  name    = local.analytics
  content = "192.0.2.1"
  type    = "A"
  proxied = true
  ttl     = 1
}

resource "cloudflare_dns_record" "verification_txt" {
  zone_id = var.zone_id
  name    = data.cloudflare_zone.primary.name
  content = "cloudflare-lab managed by terraform"
  type    = "TXT"
  ttl     = 3600
}