use thiserror::Error;

/// Exit code definitions for CLI
pub mod exit_codes {
    pub const SUCCESS: i32 = 0;
    pub const GENERAL_ERROR: i32 = 1;
    pub const USAGE_ERROR: i32 = 2;
}

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
    #[error("{context}")]
    Cli {
        context: String,
        suggestion: Option<String>,
    },

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

impl StratumError {
    /// Get the appropriate exit code for this error
    pub fn exit_code(&self) -> i32 {
        match self {
            StratumError::Cli { .. } => exit_codes::USAGE_ERROR,
            StratumError::NotFound(_) => exit_codes::GENERAL_ERROR,
            _ => exit_codes::GENERAL_ERROR,
        }
    }

    /// Format error with CLI-friendly context and suggestion
    pub fn format_cli(&self) -> String {
        match self {
            StratumError::Cli {
                context,
                suggestion,
            } => {
                if let Some(suggestion) = suggestion {
                    format!("error: {}\n  hint: {}", context, suggestion)
                } else {
                    format!("error: {}", context)
                }
            }
            StratumError::NotFound(entity) => {
                format!(
                    "error: {} not found\n  hint: check that the name or ID is correct",
                    entity
                )
            }
            StratumError::Storage(e) => {
                format!("error: storage operation failed\n  detail: {}\n  hint: check database integrity and permissions", e)
            }
            StratumError::Engine(e) => {
                format!("error: engine operation failed\n  detail: {}", e)
            }
            StratumError::StateMachine(e) => {
                format!("error: state transition rejected\n  detail: {}", e)
            }
            StratumError::Checkpoint(e) => {
                format!("error: checkpoint operation failed\n  detail: {}", e)
            }
            StratumError::GitSync(e) => {
                format!("error: git sync failed\n  detail: {}", e)
            }
            StratumError::Gc(e) => {
                format!("error: garbage collection failed\n  detail: {}", e)
            }
            StratumError::General(e) => {
                format!("error: {}", e)
            }
            StratumError::Serialization(e) => {
                format!("error: serialization failed\n  detail: {}", e)
            }
        }
    }
}

impl StratumError {
    /// Convenience constructor for CLI errors with suggestion
    pub fn cli(context: impl Into<String>, suggestion: impl Into<String>) -> Self {
        StratumError::Cli {
            context: context.into(),
            suggestion: Some(suggestion.into()),
        }
    }

    /// Convenience constructor for CLI errors without suggestion
    pub fn cli_simple(context: impl Into<String>) -> Self {
        StratumError::Cli {
            context: context.into(),
            suggestion: None,
        }
    }
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
