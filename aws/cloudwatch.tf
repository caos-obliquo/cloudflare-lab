# ---------------------------------------------------------------------------
# CloudWatch Dashboard — devops-api Lambda observability
# ---------------------------------------------------------------------------
# Widgets: invocations, errors, duration(p50/p95/p99), error rate,
# log volume, concurrent executions. Multi-environment via var.environment.
resource "aws_cloudwatch_dashboard" "devops_api" {
  dashboard_name = "devops-api-${var.environment}"
  dashboard_body = jsonencode({
    widgets = [
      {
        type   = "metric"
        x      = 0
        y      = 0
        width  = 12
        height = 6
        properties = {
          view    = "timeSeries"
          stacked = false
          metrics = [
            ["AWS/Lambda", "Invocations", { stat = "Sum", label = "Invocations" }],
            ["AWS/Lambda", "Errors", { stat = "Sum", label = "Errors" }],
            ["AWS/Lambda", "Throttles", { stat = "Sum", label = "Throttles" }],
          ]
          region = var.aws_region
          title  = "Lambda Invocations / Errors / Throttles (${var.environment})"
          period = 300
          stat   = "Sum"
        }
      },
      {
        type   = "metric"
        x      = 12
        y      = 0
        width  = 12
        height = 6
        properties = {
          view    = "timeSeries"
          stacked = false
          metrics = [
            ["AWS/Lambda", "Duration", { stat = "p50", label = "p50" }],
            ["AWS/Lambda", "Duration", { stat = "p95", label = "p95" }],
            ["AWS/Lambda", "Duration", { stat = "p99", label = "p99" }],
          ]
          region = var.aws_region
          title  = "Lambda Duration (p50/p95/p99, ms) — ${var.environment}"
          period = 300
          stat   = "p50"
        }
      },
      {
        type   = "metric"
        x      = 0
        y      = 6
        width  = 8
        height = 6
        properties = {
          view    = "timeSeries"
          stacked = false
          metrics = [
            ["AWS/Lambda", "ConcurrentExecutions", { stat = "Max", label = "Concurrent" }],
          ]
          region = var.aws_region
          title  = "Concurrent Executions — ${var.environment}"
          period = 300
          stat   = "Max"
        }
      },
      {
        type   = "metric"
        x      = 8
        y      = 6
        width  = 8
        height = 6
        properties = {
          view    = "timeSeries"
          stacked = false
          metrics = [
            ["AWS/Lambda", "IteratorAge", { stat = "Max", label = "IteratorAge" }],
          ]
          region = var.aws_region
          title  = "Iterator Age (ms) — ${var.environment}"
          period = 300
          stat   = "Max"
        }
      },
      {
        type   = "log"
        x      = 16
        y      = 6
        width  = 8
        height = 6
        properties = {
          query  = "SOURCE '/aws/lambda/devops-api-${var.environment}' | stats count() by level"
          region = var.aws_region
          title  = "Log Volume by Level — ${var.environment}"
          view   = "pie"
        }
      },
      {
        type   = "metric"
        x      = 0
        y      = 12
        width  = 24
        height = 6
        properties = {
          view    = "timeSeries"
          stacked = false
          metrics = [
            ["AWS/Lambda", "Errors", { stat = "Sum", label = "Error Count" }],
            [{ expression : "m1 / MAX(m1)", label : "Error Rate (normalized)" }],
          ]
          region = var.aws_region
          title  = "Error Rate — ${var.environment}"
          period = 300
          stat   = "Sum"
        }
      },
    ]
  })
}

# ---------------------------------------------------------------------------
# CloudWatch Alarms — Lambda health SLO
# ---------------------------------------------------------------------------
# Alarm: any error in a 5-minute window → critical
resource "aws_cloudwatch_metric_alarm" "lambda_errors" {
  alarm_name          = "devops-api-${var.environment}-errors"
  comparison_operator = "GreaterThanThreshold"
  evaluation_periods  = 1
  metric_name         = "Errors"
  namespace           = "AWS/Lambda"
  period              = 300
  statistic           = "Sum"
  threshold           = 0
  alarm_description   = "Lambda function returned errors in the last 5 minutes"
  alarm_actions       = [] # wire SNS topic ARN here
  ok_actions          = []

  dimensions = {
    FunctionName = aws_lambda_function.devops_api.function_name
  }
}

# Alarm: p95 duration > 5s (5000ms) → warning
resource "aws_cloudwatch_metric_alarm" "lambda_duration_p95" {
  alarm_name          = "devops-api-${var.environment}-duration-p95"
  comparison_operator = "GreaterThanThreshold"
  evaluation_periods  = 2
  metric_name         = "Duration"
  namespace           = "AWS/Lambda"
  period              = 300
  extended_statistic  = "p95"
  threshold           = 5000
  alarm_description   = "Lambda p95 duration exceeded 5s"
  alarm_actions       = []
  ok_actions          = []

  dimensions = {
    FunctionName = aws_lambda_function.devops_api.function_name
  }
}

# Alarm: throttles > 0 in 5 min → critical
resource "aws_cloudwatch_metric_alarm" "lambda_throttles" {
  alarm_name          = "devops-api-${var.environment}-throttles"
  comparison_operator = "GreaterThanThreshold"
  evaluation_periods  = 1
  metric_name         = "Throttles"
  namespace           = "AWS/Lambda"
  period              = 300
  statistic           = "Sum"
  threshold           = 0
  alarm_description   = "Lambda function was throttled — concurrency limit hit"
  alarm_actions       = []
  ok_actions          = []

  dimensions = {
    FunctionName = aws_lambda_function.devops_api.function_name
  }
}

# ---------------------------------------------------------------------------
# Log Metric Filter — ERROR level log → metric for alarm
# ---------------------------------------------------------------------------
resource "aws_cloudwatch_log_metric_filter" "lambda_errors_log" {
  name           = "devops-api-${var.environment}-error-log-count"
  pattern        = "\"level\":\"ERROR\""
  log_group_name = "/aws/lambda/devops-api-${var.environment}"

  metric_transformation {
    name          = "ErrorLogCount"
    namespace     = "DevOpsApi/LogMetrics"
    value         = "1"
    default_value = "0"
  }
}

resource "aws_cloudwatch_metric_alarm" "lambda_error_logs" {
  alarm_name          = "devops-api-${var.environment}-error-logs"
  comparison_operator = "GreaterThanThreshold"
  evaluation_periods  = 1
  metric_name         = "ErrorLogCount"
  namespace           = "DevOpsApi/LogMetrics"
  period              = 300
  statistic           = "Sum"
  threshold           = 0
  alarm_description   = "ERROR-level log entries detected in the last 5 minutes"
  alarm_actions       = []
  ok_actions          = []

  dimensions = {}
}

# ---------------------------------------------------------------------------
# Dashboard outputs
# ---------------------------------------------------------------------------
output "dashboard_name" {
  description = "CloudWatch dashboard name"
  value       = aws_cloudwatch_dashboard.devops_api.dashboard_name
}

output "alarm_names" {
  description = "CloudWatch alarm names created for the Lambda function"
  value = [
    aws_cloudwatch_metric_alarm.lambda_errors.alarm_name,
    aws_cloudwatch_metric_alarm.lambda_duration_p95.alarm_name,
    aws_cloudwatch_metric_alarm.lambda_throttles.alarm_name,
    aws_cloudwatch_metric_alarm.lambda_error_logs.alarm_name,
  ]
}