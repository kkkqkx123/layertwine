pub mod core;
pub mod storage;
pub mod error;
pub mod engine;
pub mod layered;
pub mod backup;
pub mod checkpoint;
pub mod git_sync;
pub mod api;

// Re-export common types
pub use error::{StorageError, StorageResult, StratumError};
pub use crate::error as err;
