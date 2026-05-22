//! gRPC transport layer for Stratum API
//!
//! Provides a tonic-based gRPC server that wraps the ApiService trait.
//! Enabled with `feature = "grpc"`.
//!
//! ## Proto schema
//!
//! See `proto/stratum.proto` for the full service definition. To enable
//! the gRPC server in production, add `tonic-build` as a build dependency
//! and compile the proto in `build.rs`:
//!
//! ```ignore
//! // build.rs
//! fn main() {
//!     tonic_build::compile_protos("src/api/rpc/proto/stratum.proto")
//!         .expect("failed to compile protos");
//! }
//! ```
//!
//! Then uncomment the generated-code block below and implement handlers.
//!
//! ## Design
//!
//! - `StratumGrpc` wraps `Arc<dyn ApiService>` and implements the generated
//!   tonic `Stratum` service trait.
//! - Each RPC method converts protobuf request → `api::*Request` → call
//!   the corresponding `ApiService` method → convert response → protobuf.

use std::net::SocketAddr;
use std::sync::Arc;

use tonic::{transport::Server, Request, Response, Status};

use crate::api::service::ApiService;

/// gRPC service implementation wrapping ApiService
pub struct StratumGrpc {
    service: Arc<dyn ApiService>,
}

impl StratumGrpc {
    pub fn new(service: Arc<dyn ApiService>) -> Self {
        Self { service }
    }
}

// ── Tonic service trait implementation ──
//
// When tonic-build is wired up (see module doc), this module will `include!`
// the generated code from OUT_DIR and implement the generated trait.
//
// The generated trait will look approximately like:
//
// ```ignore
// #[tonic::async_trait]
// pub trait Stratum: Send + Sync {
//     async fn init(&self, req: Request<generated::InitRequest>)
//         -> Result<Response<generated::InitResponse>, Status>;
//     async fn status(&self, req: Request<generated::Empty>)
//         -> Result<Response<generated::StatusResponse>, Status>;
//     // ... every RPC
// }
// ```
//
// Each handler follows the same pattern:
//
// ```ignore
// async fn init(&self, req: Request<generated::InitRequest>) -> Result<Response<generated::InitResponse>, Status> {
//     let proto_req = req.into_inner();
//     let api_req = api::InitRequest {
//         db_path: proto_req.db_path,
//         git_repo: proto_req.git_repo,
//         git_ref: proto_req.git_ref,
//     };
//     match self.service.init(api_req) {
//         Ok(api_resp) => Ok(Response::new(generated::InitResponse {
//             db_path: api_resp.db_path,
//             manual_partition_id: api_resp.manual_partition_id,
//             staged_partition_id: api_resp.staged_partition_id,
//             branch: api_resp.branch,
//         })),
//         Err(e) => Err(Status::internal(e.to_string())),
//     }
// }
// ```
//
// The following RPCs map 1:1 from the proto service definition:
//   init, status, edit, agent_edit, agent_submit, approve,
//   commit, log, branch_create, branch_switch, branch_list,
//   merge, backup, restore, gc, push, pull

/// Start the gRPC server
///
/// ```no_run
/// use std::sync::Arc;
/// use std::net::SocketAddr;
/// use stratum::api::service::{ApiService, ApiServiceImpl, ServiceConfig};
/// use stratum::api::rpc;
///
/// # async fn example() {
/// let service = ApiServiceImpl::open(ServiceConfig::default()).unwrap();
/// let addr: SocketAddr = "127.0.0.1:50051".parse().unwrap();
/// rpc::serve(Arc::new(service), addr).await.unwrap();
/// # }
/// ```
pub async fn serve(service: Arc<dyn ApiService>, addr: SocketAddr) -> Result<(), crate::error::StratumError> {
    let _ = service; // reserved for handler implementation
    let _ = addr;

    // Once tonic-build is wired up and handlers are implemented:
    //
    // let grpc = StratumGrpc::new(service);
    // Server::builder()
    //     .add_service(stratum_proto_server::StratumServer::new(grpc))
    //     .serve(addr)
    //     .await
    //     .map_err(|e| StratumError::General(format!("gRPC server error: {}", e)))?;

    Err(crate::error::StratumError::General(
        "gRPC server requires tonic-build codegen; see src/api/rpc/proto/stratum.proto".into(),
    ))
}