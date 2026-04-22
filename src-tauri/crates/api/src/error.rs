use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("HTTP error {status}: {body}")]
    Http { status: u16, body: String },

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Rate limited — retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Keychain error: {0}")]
    Keychain(String),

    #[error("Stream ended unexpectedly")]
    StreamEnded,

    #[error("Provider error: {0}")]
    Provider(String),
}
