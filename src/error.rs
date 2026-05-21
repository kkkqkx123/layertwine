use thiserror::Error;

/// Stratum Global Error Type
#[derive(Error, Debug)]
pub enum StratumError {
    // Storage layer error.
    #[error("Storage layer error: {0}")]
    Storage(#[from] StorageError),

    // Engine-level error.
    #[error("Engine error: {0}")]
    Engine(String),

    // State machine error.
    #[error("State machine error: {0}")]
    StateMachine(String),

    // Checkpoint error.
    #[error("Checkpoint error: {0}")]
    Checkpoint(String),

    // Git synchronization error -
    #[error("Git synchronization error: {0}")]
    GitSync(String),

    // GC error.
    #[error("GC error: {0}")]
    Gc(String),

    // CLI error.
    #[error("CLI error: {0}")]
    Cli(String),

    // Generic error -
    #[error("{0}")]
    General(String),

    // Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    // Not found.
    #[error("Not found: {0}")]
    NotFound(String),
}

/// Storage Tier Specific Errors
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Record not found: {0}")]
    NotFound(String),

    #[error("Binding violation: {0}")]
    ConstraintViolation(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Migration error: {0}")]
    Migration(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenient Result Alias
pub type Result<T> = std::result::Result<T, StratumError>;

/// Storage Layer Result Alias
pub type StorageResult<T> = std::result::Result<T, StorageError>;

impl From<serde_json::Error> for StratumError {
    fn from(e: serde_json::Error) -> Self {
        StratumError::Serialization(e.to_string())
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::Serialization(e.to_string())
    }
}
