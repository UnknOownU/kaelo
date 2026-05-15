#[derive(Debug, thiserror::Error)]
pub enum KaeloError {
    #[error("fetch failed: {0}")]
    FetchFailed(String),

    #[error("cache error: {0}")]
    CacheError(String),

    #[error("extraction failed: {0}")]
    ExtractionFailed(String),

    #[error("config error: {0}")]
    ConfigError(String),

    #[error("router error: {0}")]
    RouterError(String),

    #[error("storage error: {0}")]
    StorageError(String),

    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    SerializationError(String),
}
