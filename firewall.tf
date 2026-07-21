resource "cloudflare_ruleset" "waf_custom" {
  zone_id = var.zone_id
  name    = "Custom WAF rules${local.env}"
  kind    = "zone"
  phase   = "http_request_firewall_custom"

  rules = [
    {
      action      = "block"
      expression  = "(http.request.uri.path contains \"/wp-admin\")"
      description = "block wp-admin"
    }
  ]
}
