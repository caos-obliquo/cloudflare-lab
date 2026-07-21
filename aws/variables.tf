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
