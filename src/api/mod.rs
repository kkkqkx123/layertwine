//! API module — unified access layer for Stratum operations
//!
//! Provides a single `ApiService` trait implemented by `ApiServiceImpl`,
//! with transport-specific wrappers for HTTP and gRPC behind feature gates.

pub mod service;
pub mod types;

pub use service::{ApiService, ApiServiceImpl, ServiceConfig};
pub use types::*;

#[cfg(feature = "http")]
pub mod http;

#[cfg(feature = "grpc")]
pub mod rpc;
