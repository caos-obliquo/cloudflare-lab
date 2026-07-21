variable "cloudflare_api_token" {
  description = "Cloudflare API token for Terraform automation"
  type        = string
  sensitive   = true
}

variable "zone_id" {
  description = "Cloudflare zone ID for the target domain"
  type        = string
  sensitive   = true
}

variable "account_id" {
  description = "Cloudflare account ID"
  type        = string
  sensitive   = true
}

variable "environment" {
  description = "Environment tag (dev|staging|prod). Mirrors terraform.workspace."
  type        = string
  default     = "default"
  validation {
    condition     = contains(["dev", "staging", "prod", "default"], var.environment)
    error_message = "environment must be dev|staging|prod|default."
  }
}
