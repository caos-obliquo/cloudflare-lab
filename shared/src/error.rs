use thiserror::Error;
use worker::*;

// Typed errors per binding type. {0} is the variant payload (e.g., binding name).
// #[derive(Error)] generates Display + Error trait impls.
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Binding error: {0}")]
    Binding(String),
    #[error("KV error: {0}")]
    Kv(String),
    #[error("D1 error: {0}")]
    D1(String),
    #[error("Queue error: {0}")]
    Queue(String),
    #[error("AI error: {0}")]
    Ai(String),
    #[error("Not found: {0}")]
    NotFound(String),
}

// Enables ? on Result<T, AppError> in functions returning Result<T, worker::Error>.
impl From<AppError> for Error {
    fn from(err: AppError) -> Self {
        Error::from(err.to_string().as_str())
    }
}

impl AppError {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "status": "error",
            "error": self.to_string(),
            "code": self.status_code(),
        })
    }

    pub fn status_code(&self) -> u16 {
        match self {
            AppError::NotFound(_) => 404,
            _ => 500,
        }
    }
}

// Convenience alias: AppResult<T> = Result<T, AppError>.
pub type AppResult<T> = std::result::Result<T, AppError>;
