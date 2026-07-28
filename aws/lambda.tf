# Build: cargo build --release -p devops-api. Binary: bootstrap (Lambda custom runtime).
data "archive_file" "lambda_zip" {
  type        = "zip"
  source_file = "${path.module}/../target/release/bootstrap"
  output_path = "${path.module}/../target/lambda/devops-api/deploy.zip"
}

resource "aws_lambda_function" "devops_api" {
  filename         = data.archive_file.lambda_zip.output_path
  source_code_hash = data.archive_file.lambda_zip.output_base64sha256
  function_name    = "${var.function_name}-${var.environment}"
  role             = aws_iam_role.lambda_exec.arn
  handler          = "bootstrap"
  runtime          = "provided.al2023"
  timeout          = 30
  memory_size      = 128
  tracing_config {
    mode = "Active"
  }
  layers = ["arn:aws:lambda:${var.aws_region}:901920570463:layer:aws-otel-collector-amd64-ver-0-120-0:1"]

  environment {
    variables = {
      WORKER_GATEWAY_URL = var.worker_gateway_url
      WORKER_AUTH_URL    = var.worker_auth_url
      ENVIRONMENT        = var.environment
    }
  }
}

# Function URL: HTTPS with IAM auth. Free (no API Gateway).
resource "aws_lambda_function_url" "devops_api" {
  function_name      = aws_lambda_function.devops_api.function_name
  authorization_type = "AWS_IAM"

  cors {
    allow_origins = ["*"]
    allow_methods = ["GET", "POST", "OPTIONS"]
    allow_headers = ["content-type", "x-request-id", "authorization"]
    expose_headers = ["x-request-id"]
    max_age = 86400
  }
}

# Allow IAM-authenticated principals to invoke via Function URL
resource "aws_lambda_permission" "function_url" {
  statement_id  = "AllowFunctionUrlInvoke"
  action        = "lambda:InvokeFunctionUrl"
  function_name = aws_lambda_function.devops_api.function_name
  principal     = "*"
}
