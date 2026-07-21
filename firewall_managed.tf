# Managed WAF — disabled: requires Pro+ plan.
# Free plan API tokens cannot execute Cloudflare Managed or OWASP rulesets.
# Error: "not entitled to execute this managed ruleset" for both rule IDs.
# Upgrade to Pro/Business/Enterprise plan, then uncomment.
#
# resource "cloudflare_ruleset" "waf_managed" {
#   zone_id = var.zone_id
#   name    = "Managed WAF${local.env}"
#   kind    = "zone"
#   phase   = "http_request_firewall_managed"
#
#   rules = [
#     {
#       action      = "execute"
#       expression  = "true"
#       description = "Cloudflare Managed Ruleset"
#       enabled     = true
#       action_parameters = {
#         id = "efb7b8c949ac4650a09736fc376e9aee"
#         overrides = {
#           enabled = true
#         }
#       }
#     },
#     {
#       action      = "execute"
#       expression  = "true"
#       description = "OWASP Core Ruleset"
#       enabled     = true
#       action_parameters = {
#         id = "4814384a9e5d4991b9815dcfc25d2f1f"
#         overrides = {
#           enabled = true
#         }
#       }
#     }
#   ]
# }
