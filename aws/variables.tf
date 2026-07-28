variable "aws_region" {
  description = "AWS region to deploy Lambda into"
  type        = string
  default     = "us-east-1"
}

variable "function_name" {
  description = "Name of the Lambda function"
  type        = string
  default     = "devops-api"
}

variable "environment" {
  description = "Deployment environment (dev/staging/prod)"
  type        = string
  default     = "dev"
  validation {
    condition     = contains(["dev", "staging", "prod"], var.environment)
    error_message = "environment must be dev, staging, or prod"
  }
}

variable "worker_gateway_url" {
  description = "URL of the Cloudflare gateway-worker for proxied requests"
  type        = string
  sensitive   = true
}

variable "worker_auth_url" {
  description = "URL of the Cloudflare auth-worker for proxied requests"
  type        = string
  sensitive   = true
}

# ─── Cost monitoring ──────────────────────────────────────

variable "budget_monthly_limit" {
  description = "Monthly budget limit in USD for AWS resources"
  type        = string
  default     = "50"
}

variable "budget_alert_emails" {
  description = "Email addresses for AWS budget alerts"
  type        = list(string)
  default     = ["alerts@example.com"]
}

# ─── Lambda performance ───────────────────────────────────

variable "lambda_provisioned_concurrency" {
  description = "Provisioned concurrency for Lambda (0 = disabled)"
  type        = number
  default     = 0
}

# ─── Log retention ────────────────────────────────────────

variable "log_retention_days" {
  description = "CloudWatch Log Group retention in days"
  type        = number
  default     = 30
}
