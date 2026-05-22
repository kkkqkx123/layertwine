//! API module — unified access layer for Stratum operations
//!
//! Provides a single `ApiService` trait implemented by `ApiServiceImpl`,
//! with transport-specific wrappers for CLI, HTTP, and gRPC behind feature gates.

pub mod types;
pub mod service;

pub use types::*;
pub use service::{ApiService, ApiServiceImpl, ServiceConfig};

#[cfg(feature = "cli")]
pub mod cli;

#[cfg(feature = "http")]
pub mod http;

#[cfg(feature = "grpc")]
pub mod rpc;