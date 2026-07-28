# ---------------------------------------------------------------------------
# AWS Budget — cost monitoring & alerting
# ---------------------------------------------------------------------------
# Budget: $50/month with 80%/100% threshold alerts to SNS.
# Prerequisite: create SNS topic "budget-alerts" in AWS Console or via
# a separate `aws_sns_topic` resource before applying.

resource "aws_budgets_budget" "monthly" {
  name              = "devops-api-monthly-${var.environment}"
  budget_type       = "COST"
  limit_amount      = var.budget_monthly_limit
  limit_unit        = "USD"
  time_period_start = "2025-01-01_00:00"
  time_unit         = "MONTHLY"

  cost_types {
    include_credit             = false
    include_discount           = false
    include_other_subscription = false
    include_recurring          = true
    include_refund             = false
    include_subscription       = true
    include_support            = true
    include_tax                = false
    include_upfront            = false
    use_amortized              = false
    use_blended                = false
  }

  notification {
    comparison_operator        = "GREATER_THAN"
    threshold                  = 80
    threshold_type             = "PERCENTAGE"
    notification_type          = "ACTUAL"
    subscriber_email_addresses = var.budget_alert_emails
  }

  notification {
    comparison_operator        = "GREATER_THAN"
    threshold                  = 100
    threshold_type             = "PERCENTAGE"
    notification_type          = "FORECASTED"
    subscriber_email_addresses = var.budget_alert_emails
  }
}