terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
  }

  # Local backend (no remote state).
  # R2 S3-compatible endpoint (https://b6d892f66c18ab372241fe474f507d90.r2.cloudflarestorage.com)
  # returns TLS handshake failure from this network. Enable remote state after:
  #   1. Verify R2 S3 API is enabled in Cloudflare dashboard
  #   2. Run: terraform init -migrate-state -backend-config="endpoints={s3=\"https://YOUR_ACCOUNT.r2.cloudflarestorage.com\"}"
  backend "local" {}
}

provider "cloudflare" {
  api_token = var.cloudflare_api_token
}
