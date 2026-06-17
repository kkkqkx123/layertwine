//! gRPC transport layer for Stratum API
//!
//! Provides a tonic-based gRPC server that wraps the ApiService trait.
//! Enabled with `feature = "grpc"`.
//!
//! Proto schema is compiled at build time via `build.rs` using `tonic-build`.
//! Generated code is included via `tonic::include_proto!("stratum")`.

pub mod stratum_proto {
    tonic::include_proto!("stratum");
}

use std::net::SocketAddr;
use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::api::service::ApiService;
use crate::api::types::*;
use crate::error::StratumError;
use stratum_proto::stratum_server::{Stratum, StratumServer};

/// gRPC service implementation wrapping ApiService
pub struct StratumGrpc {
    service: Arc<dyn ApiService>,
}

impl StratumGrpc {
    pub fn new(service: Arc<dyn ApiService>) -> Self {
        Self { service }
    }
}

fn to_status(e: ApiError) -> Status {
    match e.code.as_str() {
        "NOT_FOUND" => Status::not_found(e.message),
        "INVALID_PARAMS" => Status::invalid_argument(e.message),
        "ALREADY_EXISTS" => Status::already_exists(e.message),
        "PERMISSION_DENIED" => Status::permission_denied(e.message),
        _ => Status::internal(format!("[{}] {}", e.code, e.message)),
    }
}

#[tonic::async_trait]
impl Stratum for StratumGrpc {
    async fn init(
        &self,
        request: Request<stratum_proto::InitRequest>,
    ) -> Result<Response<stratum_proto::InitResponse>, Status> {
        let req = request.into_inner();
        let api_req = InitRequest {
            db_path: req.db_path,
            git_repo: req.git_repo,
            git_ref: req.git_ref,
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.init(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::InitResponse {
            db_path: result.db_path,
            manual_partition_id: result.manual_partition_id,
            staged_partition_id: result.staged_partition_id,
            branch: result.branch,
        }))
    }

    async fn status(
        &self,
        _request: Request<stratum_proto::Empty>,
    ) -> Result<Response<stratum_proto::StatusResponse>, Status> {
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.status())
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        let partitions = result
            .partitions
            .into_iter()
            .map(|p| stratum_proto::PartitionInfo {
                layer: p.layer,
                name: p.name,
                current_snapshot: p.current_snapshot,
                history_len: p.history_len as u32,
            })
            .collect();
        Ok(Response::new(stratum_proto::StatusResponse { partitions }))
    }

    async fn edit(
        &self,
        request: Request<stratum_proto::EditRequest>,
    ) -> Result<Response<stratum_proto::EditResponse>, Status> {
        let req = request.into_inner();
        let api_req = EditRequest {
            file: req.file,
            content: req.content,
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.edit(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::EditResponse {
            snapshot_id: result.snapshot_id,
            staged_snapshot_id: result.staged_snapshot_id,
        }))
    }

    async fn agent_edit(
        &self,
        request: Request<stratum_proto::AgentEditRequest>,
    ) -> Result<Response<stratum_proto::EditResponse>, Status> {
        let req = request.into_inner();
        let api_req = AgentEditRequest {
            agent_id: req.agent_id,
            file: req.file,
            content: req.content,
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.agent_edit(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::EditResponse {
            snapshot_id: result.snapshot_id,
            staged_snapshot_id: result.staged_snapshot_id,
        }))
    }

    async fn agent_submit(
        &self,
        request: Request<stratum_proto::AgentSubmitRequest>,
    ) -> Result<Response<stratum_proto::SubmitResponse>, Status> {
        let req = request.into_inner();
        let api_req = AgentSubmitRequest {
            agent_id: req.agent_id,
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.agent_submit(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::SubmitResponse {
            snapshot_id: result.snapshot_id,
        }))
    }

    async fn approve(
        &self,
        request: Request<stratum_proto::ApproveRequest>,
    ) -> Result<Response<stratum_proto::ApproveResponse>, Status> {
        let req = request.into_inner();
        let api_req = ApproveRequest {
            agent_id: req.agent_id,
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.approve(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::ApproveResponse {
            integrated_snapshot_id: result.integrated_snapshot_id,
            staged_snapshot_id: result.staged_snapshot_id,
        }))
    }

    async fn commit(
        &self,
        request: Request<stratum_proto::CommitRequest>,
    ) -> Result<Response<stratum_proto::CommitResponse>, Status> {
        let req = request.into_inner();
        let api_req = CommitRequest {
            message: req.message,
            author: req.author,
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.commit(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::CommitResponse {
            checkpoint_id: result.checkpoint_id,
            message: result.message,
        }))
    }

    async fn log(
        &self,
        request: Request<stratum_proto::LogRequest>,
    ) -> Result<Response<stratum_proto::LogResponse>, Status> {
        let req = request.into_inner();
        let api_req = LogRequest {
            count: req.count.map(|c| c as usize),
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.log(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        let checkpoints = result
            .checkpoints
            .into_iter()
            .map(|cp| stratum_proto::CheckpointInfo {
                id: cp.id,
                author: cp.author,
                message: cp.message,
                parents: cp.parents,
                snapshots: cp.snapshots,
                created_at: cp.created_at,
                git_anchor: cp.git_anchor,
            })
            .collect();
        Ok(Response::new(stratum_proto::LogResponse {
            checkpoints,
            total: result.total as u32,
        }))
    }

    async fn branch_create(
        &self,
        request: Request<stratum_proto::BranchCreateRequest>,
    ) -> Result<Response<stratum_proto::BranchCreateResponse>, Status> {
        let req = request.into_inner();
        let api_req = BranchCreateRequest { name: req.name };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.branch_create(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::BranchCreateResponse {
            name: result.name,
            head: result.head,
        }))
    }

    async fn branch_switch(
        &self,
        request: Request<stratum_proto::BranchSwitchRequest>,
    ) -> Result<Response<stratum_proto::BranchSwitchResponse>, Status> {
        let req = request.into_inner();
        let api_req = BranchSwitchRequest { name: req.name };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.branch_switch(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::BranchSwitchResponse {
            name: result.name,
            checkpoint_id: result.checkpoint_id,
        }))
    }

    async fn branch_list(
        &self,
        _request: Request<stratum_proto::Empty>,
    ) -> Result<Response<stratum_proto::BranchListResponse>, Status> {
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.branch_list())
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        let branches = result
            .branches
            .into_iter()
            .map(|b| stratum_proto::BranchInfo {
                name: b.name,
                head: b.head,
                updated_at: b.updated_at,
                is_current: b.is_current,
            })
            .collect();
        Ok(Response::new(stratum_proto::BranchListResponse {
            branches,
            current: result.current,
        }))
    }

    async fn merge(
        &self,
        request: Request<stratum_proto::MergeRequest>,
    ) -> Result<Response<stratum_proto::MergeResponse>, Status> {
        let req = request.into_inner();
        let api_req = MergeRequest {
            branch: req.branch,
            message: req.message,
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.merge(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::MergeResponse {
            checkpoint_id: result.checkpoint_id,
            source_branch: result.source_branch,
            target_branch: result.target_branch,
        }))
    }

    async fn backup(
        &self,
        request: Request<stratum_proto::BackupRequest>,
    ) -> Result<Response<stratum_proto::BackupResponse>, Status> {
        let req = request.into_inner();
        let api_req = BackupRequest {
            snapshot_id: req.snapshot_id,
            label: req.label,
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.backup(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::BackupResponse {
            backup_id: result.backup_id,
            source_snapshot_id: result.source_snapshot_id,
            label: result.label,
        }))
    }

    async fn restore(
        &self,
        request: Request<stratum_proto::RestoreRequest>,
    ) -> Result<Response<stratum_proto::RestoreResponse>, Status> {
        let req = request.into_inner();
        let api_req = RestoreRequest {
            backup_id: req.backup_id,
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.restore(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::RestoreResponse {
            backup_id: result.backup_id,
            file: result.file,
            deltas_restored: result.deltas_restored as u32,
        }))
    }

    async fn gc(
        &self,
        _request: Request<stratum_proto::Empty>,
    ) -> Result<Response<stratum_proto::GcResponse>, Status> {
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.gc(GcRequest {}))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::GcResponse {
            removed_checkpoints: result.removed_checkpoints as u32,
            removed_snapshots: result.removed_snapshots as u32,
            freed_bytes: result.freed_bytes,
            delta_chain_depth_triggered: result.delta_chain_depth_triggered,
        }))
    }

    async fn push(
        &self,
        request: Request<stratum_proto::PushRequest>,
    ) -> Result<Response<stratum_proto::PushResponse>, Status> {
        let req = request.into_inner();
        let api_req = PushRequest {
            remote: req.remote,
            git_repo: req.git_repo,
            message: req.message,
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.push(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::PushResponse {
            remote: result.remote,
            git_commit_hash: result.git_commit_hash,
        }))
    }

    async fn pull(
        &self,
        request: Request<stratum_proto::PullRequest>,
    ) -> Result<Response<stratum_proto::PullResponse>, Status> {
        let req = request.into_inner();
        let api_req = PullRequest {
            remote: req.remote,
            git_repo: req.git_repo,
            git_ref: req.git_ref,
        };
        let service = self.service.clone();
        let result = tokio::task::spawn_blocking(move || service.pull(api_req))
            .await
            .map_err(|e| Status::internal(format!("join error: {}", e)))?
            .map_err(to_status)?;
        Ok(Response::new(stratum_proto::PullResponse {
            remote: result.remote,
            git_ref: result.git_ref,
        }))
    }
}

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
pub async fn serve(service: Arc<dyn ApiService>, addr: SocketAddr) -> Result<(), StratumError> {
    let grpc = StratumGrpc::new(service);

    tonic::transport::Server::builder()
        .add_service(StratumServer::new(grpc))
        .serve(addr)
        .await
        .map_err(|e| StratumError::General(format!("gRPC server error: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── to_status error mapping tests ──

    #[test]
    fn test_to_status_not_found() {
        let err = ApiError::not_found("partition 'test'");
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::NotFound);
        assert!(status.message().contains("partition 'test'"));
    }

    #[test]
    fn test_to_status_invalid_params() {
        let err = ApiError::invalid_params("missing file path");
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert!(status.message().contains("missing file path"));
    }

    #[test]
    fn test_to_status_internal() {
        let err = ApiError::internal("database corruption");
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status.message().contains("database corruption"));
    }

    #[test]
    fn test_to_status_storage_error() {
        let err = ApiError::storage("disk full");
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status.message().contains("disk full"));
    }

    #[test]
    fn test_to_status_engine_error() {
        let err = ApiError::engine("diff failed");
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status.message().contains("[ENGINE_ERROR] diff failed"));
    }

    #[test]
    fn test_to_status_state_machine_error() {
        let err = ApiError::state_machine("invalid transition");
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status.message().contains("invalid transition"));
    }

    #[test]
    fn test_to_status_checkpoint_error() {
        let err = ApiError::checkpoint("no changes to commit");
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status.message().contains("no changes to commit"));
    }

    #[test]
    fn test_to_status_git_sync_error() {
        let err = ApiError::git_sync("remote rejected");
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status.message().contains("remote rejected"));
    }

    #[test]
    fn test_to_status_gc_error() {
        let err = ApiError::gc("garbage collection failed");
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status.message().contains("garbage collection failed"));
    }

    #[test]
    fn test_to_status_general_error() {
        let err = ApiError::general("unknown error");
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status.message().contains("[ERROR] unknown error"));
    }

    #[test]
    fn test_to_status_already_exists() {
        let err = ApiError {
            code: "ALREADY_EXISTS".into(),
            message: "branch 'feature' already exists".into(),
            suggestion: Some("use a different name".into()),
            details: None,
        };
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::AlreadyExists);
        assert!(status.message().contains("branch 'feature' already exists"));
    }

    #[test]
    fn test_to_status_permission_denied() {
        let err = ApiError {
            code: "PERMISSION_DENIED".into(),
            message: "access denied".into(),
            suggestion: None,
            details: None,
        };
        let status = to_status(err);
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
    }

    // ── StratumGrpc construction test ──

    #[test]
    fn test_stratum_grpc_new() {
        let service = Arc::new(SuccessMockService);
        let _grpc = StratumGrpc::new(service);
    }

    // ── RPC handler delegation tests ──

    #[tokio::test]
    async fn test_rpc_init_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::InitRequest {
            db_path: Some(".stratum/test.db".into()),
            git_repo: None,
            git_ref: None,
        });
        let result = grpc.init(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.branch, "main");
    }

    #[tokio::test]
    async fn test_rpc_status_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let result = grpc
            .status(Request::new(stratum_proto::Empty {}))
            .await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.partitions.len(), 1);
        assert_eq!(resp.partitions[0].layer, "manual_edit");
    }

    #[tokio::test]
    async fn test_rpc_edit_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::EditRequest {
            file: "test.txt".into(),
            content: Some("hello".into()),
        });
        let result = grpc.edit(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.snapshot_id, "snap1");
        assert_eq!(resp.staged_snapshot_id, Some("staged1".into()));
    }

    #[tokio::test]
    async fn test_rpc_agent_edit_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::AgentEditRequest {
            agent_id: "agent-01".into(),
            file: "test.txt".into(),
            content: Some("hello".into()),
        });
        let result = grpc.agent_edit(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.snapshot_id, "agent-snap1");
        assert_eq!(resp.staged_snapshot_id, None);
    }

    #[tokio::test]
    async fn test_rpc_agent_submit_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::AgentSubmitRequest {
            agent_id: "agent-01".into(),
        });
        let result = grpc.agent_submit(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.snapshot_id, "submit1");
    }

    #[tokio::test]
    async fn test_rpc_approve_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::ApproveRequest {
            agent_id: "agent-01".into(),
        });
        let result = grpc.approve(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.integrated_snapshot_id, "int1");
        assert_eq!(resp.staged_snapshot_id, "staged1");
    }

    #[tokio::test]
    async fn test_rpc_commit_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::CommitRequest {
            message: "test commit".into(),
            author: Some("user".into()),
        });
        let result = grpc.commit(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.checkpoint_id, "cp1");
        assert_eq!(resp.message, "test");
    }

    #[tokio::test]
    async fn test_rpc_log_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::LogRequest { count: Some(10) });
        let result = grpc.log(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.total, 0);
        assert!(resp.checkpoints.is_empty());
    }

    #[tokio::test]
    async fn test_rpc_branch_create_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::BranchCreateRequest {
            name: "feature".into(),
        });
        let result = grpc.branch_create(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.name, "feature");
        assert_eq!(resp.head, "cp1");
    }

    #[tokio::test]
    async fn test_rpc_branch_switch_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::BranchSwitchRequest {
            name: "feature".into(),
        });
        let result = grpc.branch_switch(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.name, "feature");
    }

    #[tokio::test]
    async fn test_rpc_branch_list_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let result = grpc
            .branch_list(Request::new(stratum_proto::Empty {}))
            .await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.branches.len(), 1);
        assert_eq!(resp.branches[0].name, "main");
        assert_eq!(resp.branches[0].is_current, true);
    }

    #[tokio::test]
    async fn test_rpc_merge_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::MergeRequest {
            branch: "feature".into(),
            message: Some("merge feature".into()),
        });
        let result = grpc.merge(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.source_branch, "feature");
        assert_eq!(resp.target_branch, "main");
    }

    #[tokio::test]
    async fn test_rpc_backup_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::BackupRequest {
            snapshot_id: "snap1".into(),
            label: Some("test label".into()),
        });
        let result = grpc.backup(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.backup_id, "backup1");
        assert_eq!(resp.label, Some("label".into()));
    }

    #[tokio::test]
    async fn test_rpc_restore_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::RestoreRequest {
            backup_id: "backup1".into(),
        });
        let result = grpc.restore(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.backup_id, "backup1");
        assert_eq!(resp.file, "test.txt");
        assert_eq!(resp.deltas_restored, 3);
    }

    #[tokio::test]
    async fn test_rpc_gc_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let result = grpc.gc(Request::new(stratum_proto::Empty {})).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.removed_checkpoints, 2);
        assert_eq!(resp.removed_snapshots, 5);
        assert_eq!(resp.freed_bytes, 1024);
        assert!(!resp.delta_chain_depth_triggered);
    }

    #[tokio::test]
    async fn test_rpc_push_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::PushRequest {
            remote: Some("origin".into()),
            git_repo: "/tmp/repo".into(),
            message: Some("sync".into()),
        });
        let result = grpc.push(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.remote, "origin");
        assert_eq!(resp.git_commit_hash, "abc123");
    }

    #[tokio::test]
    async fn test_rpc_pull_delegates_to_service() {
        let svc = Arc::new(SuccessMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::PullRequest {
            remote: Some("origin".into()),
            git_repo: "/tmp/repo".into(),
            git_ref: Some("main".into()),
        });
        let result = grpc.pull(req).await;
        assert!(result.is_ok());
        let resp = result.unwrap().into_inner();
        assert_eq!(resp.remote, "origin");
        assert_eq!(resp.git_ref, "main");
    }

    #[tokio::test]
    async fn test_rpc_init_error_maps_to_status() {
        let svc = Arc::new(ErrorMockService);
        let grpc = StratumGrpc::new(svc);
        let req = Request::new(stratum_proto::InitRequest {
            db_path: None,
            git_repo: None,
            git_ref: None,
        });
        let result = grpc.init(req).await;
        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::NotFound);
    }

    // ── Mock services ──

    struct SuccessMockService;

    impl ApiService for SuccessMockService {
        fn init(&self, _req: InitRequest) -> ApiResult<InitResponse> {
            Ok(InitResponse {
                db_path: ".stratum/test.db".into(),
                manual_partition_id: "manual-1".into(),
                staged_partition_id: "staged-1".into(),
                branch: "main".into(),
            })
        }
        fn status(&self) -> ApiResult<StatusResponse> {
            Ok(StatusResponse {
                partitions: vec![PartitionInfo {
                    layer: "manual_edit".into(),
                    name: "manual".into(),
                    current_snapshot: "abc123".into(),
                    history_len: 1,
                }],
            })
        }
        fn edit(&self, _req: EditRequest) -> ApiResult<EditResponse> {
            Ok(EditResponse {
                snapshot_id: "snap1".into(),
                staged_snapshot_id: Some("staged1".into()),
            })
        }
        fn agent_edit(&self, _req: AgentEditRequest) -> ApiResult<EditResponse> {
            Ok(EditResponse {
                snapshot_id: "agent-snap1".into(),
                staged_snapshot_id: None,
            })
        }
        fn agent_submit(&self, _req: AgentSubmitRequest) -> ApiResult<SubmitResponse> {
            Ok(SubmitResponse {
                snapshot_id: "submit1".into(),
            })
        }
        fn approve(&self, _req: ApproveRequest) -> ApiResult<ApproveResponse> {
            Ok(ApproveResponse {
                integrated_snapshot_id: "int1".into(),
                staged_snapshot_id: "staged1".into(),
            })
        }
        fn commit(&self, _req: CommitRequest) -> ApiResult<CommitResponse> {
            Ok(CommitResponse {
                checkpoint_id: "cp1".into(),
                message: "test".into(),
            })
        }
        fn log(&self, _req: LogRequest) -> ApiResult<LogResponse> {
            Ok(LogResponse {
                checkpoints: vec![],
                total: 0,
            })
        }
        fn branch_create(&self, _req: BranchCreateRequest) -> ApiResult<BranchCreateResponse> {
            Ok(BranchCreateResponse {
                name: "feature".into(),
                head: "cp1".into(),
            })
        }
        fn branch_switch(&self, _req: BranchSwitchRequest) -> ApiResult<BranchSwitchResponse> {
            Ok(BranchSwitchResponse {
                name: "feature".into(),
                checkpoint_id: "cp1".into(),
            })
        }
        fn branch_list(&self) -> ApiResult<BranchListResponse> {
            Ok(BranchListResponse {
                branches: vec![BranchInfo {
                    name: "main".into(),
                    head: "cp1".into(),
                    updated_at: "2026-01-01".into(),
                    is_current: true,
                }],
                current: Some("main".into()),
            })
        }
        fn merge(&self, _req: MergeRequest) -> ApiResult<MergeResponse> {
            Ok(MergeResponse {
                checkpoint_id: "merge1".into(),
                source_branch: "feature".into(),
                target_branch: "main".into(),
            })
        }
        fn backup(&self, _req: BackupRequest) -> ApiResult<BackupResponse> {
            Ok(BackupResponse {
                backup_id: "backup1".into(),
                source_snapshot_id: "snap1".into(),
                label: Some("label".into()),
            })
        }
        fn restore(&self, _req: RestoreRequest) -> ApiResult<RestoreResponse> {
            Ok(RestoreResponse {
                backup_id: "backup1".into(),
                file: "test.txt".into(),
                deltas_restored: 3,
            })
        }
        fn gc(&self, _req: GcRequest) -> ApiResult<GcResponse> {
            Ok(GcResponse {
                removed_checkpoints: 2,
                removed_snapshots: 5,
                freed_bytes: 1024,
                delta_chain_depth_triggered: false,
            })
        }
        fn compact(&self, _req: CompactRequest) -> ApiResult<CompactResponse> {
            Ok(CompactResponse {
                wal_checkpointed: true,
                freelist_before: 100,
                total_pages: 200,
                freelist_after: 50,
                vacuum_performed: false,
                message: "ok".into(),
            })
        }
        fn push(&self, _req: PushRequest) -> ApiResult<PushResponse> {
            Ok(PushResponse {
                remote: "origin".into(),
                git_commit_hash: "abc123".into(),
            })
        }
        fn pull(&self, _req: PullRequest) -> ApiResult<PullResponse> {
            Ok(PullResponse {
                remote: "origin".into(),
                git_ref: "main".into(),
            })
        }
        fn show(&self, _req: ShowRequest) -> ApiResult<ShowResponse> {
            Ok(ShowResponse {
                target: "staged".into(),
                diffs: vec![],
            })
        }
        fn list_pending_approvals(&self) -> ApiResult<ListPendingApprovalsResponse> {
            Ok(ListPendingApprovalsResponse {
                approvals: vec![],
                total: 0,
            })
        }
        fn approve_agent(&self, _req: ApproveAgentRequest) -> ApiResult<ApproveAgentResponse> {
            Ok(ApproveAgentResponse {
                agent_id: "agent-1".into(),
                integrated_snapshot_id: "int1".into(),
            })
        }
        fn reject_agent(&self, _req: RejectAgentRequest) -> ApiResult<RejectAgentResponse> {
            Ok(RejectAgentResponse {
                agent_id: "agent-1".into(),
                baseline_snapshot_id: "base1".into(),
            })
        }
        fn merge_to_unified(
            &self,
            _req: MergeToUnifiedRequest,
        ) -> ApiResult<MergeToUnifiedResponse> {
            Ok(MergeToUnifiedResponse {
                unified_snapshot_id: "uni1".into(),
                merged_count: 2,
            })
        }
        fn merge_to_staged(
            &self,
            _req: MergeToStagedRequest,
        ) -> ApiResult<MergeToStagedResponse> {
            Ok(MergeToStagedResponse {
                staged_snapshot_id: "staged1".into(),
            })
        }
    }

    struct ErrorMockService;

    impl ApiService for ErrorMockService {
        fn init(&self, _req: InitRequest) -> ApiResult<InitResponse> {
            Err(ApiError::not_found("database"))
        }
        fn status(&self) -> ApiResult<StatusResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn edit(&self, _: EditRequest) -> ApiResult<EditResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn agent_edit(&self, _: AgentEditRequest) -> ApiResult<EditResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn agent_submit(&self, _: AgentSubmitRequest) -> ApiResult<SubmitResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn approve(&self, _: ApproveRequest) -> ApiResult<ApproveResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn commit(&self, _: CommitRequest) -> ApiResult<CommitResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn log(&self, _: LogRequest) -> ApiResult<LogResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn branch_create(&self, _: BranchCreateRequest) -> ApiResult<BranchCreateResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn branch_switch(&self, _: BranchSwitchRequest) -> ApiResult<BranchSwitchResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn branch_list(&self) -> ApiResult<BranchListResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn merge(&self, _: MergeRequest) -> ApiResult<MergeResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn backup(&self, _: BackupRequest) -> ApiResult<BackupResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn restore(&self, _: RestoreRequest) -> ApiResult<RestoreResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn gc(&self, _: GcRequest) -> ApiResult<GcResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn compact(&self, _: CompactRequest) -> ApiResult<CompactResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn push(&self, _: PushRequest) -> ApiResult<PushResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn pull(&self, _: PullRequest) -> ApiResult<PullResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn show(&self, _: ShowRequest) -> ApiResult<ShowResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn list_pending_approvals(&self) -> ApiResult<ListPendingApprovalsResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn approve_agent(&self, _: ApproveAgentRequest) -> ApiResult<ApproveAgentResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn reject_agent(&self, _: RejectAgentRequest) -> ApiResult<RejectAgentResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn merge_to_unified(
            &self,
            _: MergeToUnifiedRequest,
        ) -> ApiResult<MergeToUnifiedResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn merge_to_staged(
            &self,
            _: MergeToStagedRequest,
        ) -> ApiResult<MergeToStagedResponse> {
            Err(ApiError::internal("not impl"))
        }
    }
}
