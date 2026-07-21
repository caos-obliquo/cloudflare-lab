output "function_name" {
  description = "Lambda function name"
  value       = aws_lambda_function.devops_api.function_name
}

output "function_url" {
  description = "Lambda Function URL endpoint (IAM-auth required)"
  value       = aws_lambda_function_url.devops_api.function_url
}

output "function_arn" {
  description = "Lambda function ARN"
  value       = aws_lambda_function.devops_api.arn
}

output "iam_role_arn" {
  description = "IAM execution role ARN"
  value       = aws_iam_role.lambda_exec.arn
}
