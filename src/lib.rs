pub mod core;
pub mod storage;
pub mod error;
pub mod engine;
pub mod state_machine;
pub mod backup;
pub mod checkpoint;
pub mod git_sync;
pub mod cli;

// Re-export common types
pub use error::{StorageError, StorageResult, StratumError};
pub use crate::error as err;
