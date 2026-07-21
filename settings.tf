# Zone-level settings disabled — API token lacks Zone permissions.
# Errors: ssl/min_tls_version/security_level got 9109 (Unauthorized),
#         always_use_https/automatic_https_rewrites/browser_cache_ttl got 10000 (Auth error).
# To enable: add Zone:Settings permission to the API token, then uncomment.
#
# resource "cloudflare_zone_setting" "ssl" {
#   zone_id    = var.zone_id
#   setting_id = "ssl"
#   value      = "full"
# }
#
# resource "cloudflare_zone_setting" "min_tls_version" {
#   zone_id    = var.zone_id
#   setting_id = "min_tls_version"
#   value      = "1.2"
# }
#
# resource "cloudflare_zone_setting" "always_use_https" {
#   zone_id    = var.zone_id
#   setting_id = "always_use_https"
#   value      = "on"
# }
#
# resource "cloudflare_zone_setting" "automatic_https_rewrites" {
#   zone_id    = var.zone_id
#   setting_id = "automatic_https_rewrites"
#   value      = "on"
# }
#
# resource "cloudflare_zone_setting" "security_level" {
#   zone_id    = var.zone_id
#   setting_id = "security_level"
#   value      = "high"
# }
#
# resource "cloudflare_zone_setting" "browser_cache_ttl" {
#   zone_id    = var.zone_id
#   setting_id = "browser_cache_ttl"
#   value      = "14400"
# }
