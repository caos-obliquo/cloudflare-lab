output "kv_namespace_id" {
  description = "Workers KV namespace ID for the gateway worker binding"
  value       = cloudflare_workers_kv_namespace.test_kv.id
  sensitive   = true
}

output "d1_database_id" {
  description = "D1 database ID for auth and analytics workers"
  value       = cloudflare_d1_database.test_d1.id
  sensitive   = true
}

# R2 output disabled — R2 bucket resource is commented out (R2 not enabled on account).
# output "r2_bucket_name" {
#   description = "R2 bucket name for object storage"
#   value       = cloudflare_r2_bucket.test_r2.name
#   sensitive   = false
# }

output "queue_id" {
  description = "Queue ID for worker event processing"
  value       = cloudflare_queue.test_queue.id
  sensitive   = true
}

output "ai_gateway_id" {
  description = "AI Gateway ID for AI inference routing"
  value       = cloudflare_ai_gateway.test_ai_gateway.id
  sensitive   = false
}
