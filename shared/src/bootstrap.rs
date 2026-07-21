use worker::*;

// Called at worker startup. D1 has no migration runner, so CREATE TABLE IF NOT EXISTS
// is the standard bootstrap pattern. Atomic, idempotent, safe to call every startup.
pub async fn ensure_users_table(db: &D1Database) -> Result<()> {
    db.prepare(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password TEXT NOT NULL
        )",
    )
    .run()
    .await?;
    console_log!("[bootstrap] users table ready");
    Ok(())
}

// Separated because auth-worker and analytics-worker use different D1 databases.
pub async fn ensure_analytics_events_table(db: &D1Database) -> Result<()> {
    db.prepare(
        "CREATE TABLE IF NOT EXISTS analytics_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_type TEXT NOT NULL,
            event_data TEXT DEFAULT '',
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .run()
    .await?;
    console_log!("[bootstrap] analytics_events table ready");
    Ok(())
}
