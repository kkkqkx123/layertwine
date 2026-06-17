//! HTTP transport layer for Stratum API
//!
//! Provides an axum-based REST/JSON server that wraps the ApiService trait.
//! Enabled with `feature = "http"`.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use crate::api::service::ApiService;
use crate::api::types::*;

/// Shared application state
struct AppState {
    service: Arc<dyn ApiService>,
}

/// Start the HTTP server
///
/// ```no_run
/// use std::sync::Arc;
/// use stratum::api::service::{ApiService, ApiServiceImpl, ServiceConfig};
/// use stratum::api::http;
///
/// # async fn example() {
/// let service = ApiServiceImpl::open(ServiceConfig::default()).unwrap();
/// http::serve(Arc::new(service), "127.0.0.1:8080".parse().unwrap()).await.unwrap();
/// # }
/// ```
pub async fn serve(
    service: Arc<dyn ApiService>,
    addr: SocketAddr,
) -> Result<(), crate::error::StratumError> {
    let state = Arc::new(AppState { service });

    let app = Router::new()
        // Repository lifecycle
        .route("/api/v1/init", post(handle_init))
        .route("/api/v1/status", get(handle_status))
        // Edit operations
        .route("/api/v1/edit", post(handle_edit))
        .route("/api/v1/agent/{id}/edit", post(handle_agent_edit))
        .route("/api/v1/agent/{id}/submit", post(handle_agent_submit))
        .route("/api/v1/approve/{agent_id}", post(handle_approve))
        // Grant and approval operations
        .route("/api/v1/approvals", get(handle_list_pending_approvals))
        .route("/api/v1/approve-agent", post(handle_approve_agent))
        .route("/api/v1/reject-agent", post(handle_reject_agent))
        .route("/api/v1/merge-to-unified", post(handle_merge_to_unified))
        .route("/api/v1/merge-to-staged", post(handle_merge_to_staged))
        // Checkpoint operations
        .route("/api/v1/commit", post(handle_commit))
        .route("/api/v1/log", get(handle_log))
        // Branch operations
        .route("/api/v1/branches", get(handle_branch_list))
        .route("/api/v1/branches", post(handle_branch_create))
        .route("/api/v1/branches/{name}/switch", post(handle_branch_switch))
        .route("/api/v1/merge", post(handle_merge))
        // Backup operations
        .route("/api/v1/backup", post(handle_backup))
        .route("/api/v1/restore", post(handle_restore))
        // Maintenance
        .route("/api/v1/gc", post(handle_gc))
        .route("/api/v1/compact", post(handle_compact))
        .route("/api/v1/push", post(handle_push))
        .route("/api/v1/pull", post(handle_pull))
        .route("/api/v1/show", get(handle_show))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| crate::error::StratumError::General(format!("failed to bind: {}", e)))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| crate::error::StratumError::General(format!("server error: {}", e)))
}

// ── Unified response wrapper ──

#[derive(serde::Serialize)]
struct ApiEnvelope<T: serde::Serialize> {
    success: bool,
    data: Option<T>,
    error: Option<ApiError>,
}

fn ok_response<T: serde::Serialize>(data: T) -> Json<ApiEnvelope<T>> {
    Json(ApiEnvelope {
        success: true,
        data: Some(data),
        error: None,
    })
}

fn err_response<T: serde::Serialize>(e: ApiError) -> (StatusCode, Json<ApiEnvelope<T>>) {
    let code = match e.code.as_str() {
        "NOT_FOUND" => StatusCode::NOT_FOUND,
        "INVALID_PARAMS" => StatusCode::BAD_REQUEST,
        "ALREADY_EXISTS" => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        code,
        Json(ApiEnvelope {
            success: false,
            data: None,
            error: Some(e),
        }),
    )
}

// ── Handler functions ──

async fn handle_init(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InitRequest>,
) -> impl IntoResponse {
    match state.service.init(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<InitResponse>(e).into_response(),
    }
}

async fn handle_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.service.status() {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<StatusResponse>(e).into_response(),
    }
}

async fn handle_edit(
    State(state): State<Arc<AppState>>,
    Json(req): Json<EditRequest>,
) -> impl IntoResponse {
    match state.service.edit(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<EditResponse>(e).into_response(),
    }
}

async fn handle_agent_edit(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<EditRequest>,
) -> impl IntoResponse {
    let agent_req = AgentEditRequest {
        agent_id: id,
        file: req.file,
        content: req.content,
    };
    match state.service.agent_edit(agent_req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<EditResponse>(e).into_response(),
    }
}

async fn handle_agent_submit(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let req = AgentSubmitRequest { agent_id: id };
    match state.service.agent_submit(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<SubmitResponse>(e).into_response(),
    }
}

async fn handle_approve(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let req = ApproveRequest { agent_id };
    match state.service.approve(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<ApproveResponse>(e).into_response(),
    }
}

async fn handle_commit(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CommitRequest>,
) -> impl IntoResponse {
    match state.service.commit(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<CommitResponse>(e).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct LogQuery {
    count: Option<usize>,
}

#[derive(serde::Deserialize)]
struct ShowQuery {
    show_what: String,
    target_id: Option<String>,
}

async fn handle_log(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LogQuery>,
) -> impl IntoResponse {
    let req = LogRequest { count: query.count };
    match state.service.log(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<LogResponse>(e).into_response(),
    }
}

async fn handle_branch_list(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.service.branch_list() {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<BranchListResponse>(e).into_response(),
    }
}

async fn handle_branch_create(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BranchCreateRequest>,
) -> impl IntoResponse {
    match state.service.branch_create(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<BranchCreateResponse>(e).into_response(),
    }
}

async fn handle_branch_switch(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let req = BranchSwitchRequest { name };
    match state.service.branch_switch(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<BranchSwitchResponse>(e).into_response(),
    }
}

async fn handle_merge(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MergeRequest>,
) -> impl IntoResponse {
    match state.service.merge(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<MergeResponse>(e).into_response(),
    }
}

async fn handle_backup(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BackupRequest>,
) -> impl IntoResponse {
    match state.service.backup(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<BackupResponse>(e).into_response(),
    }
}

async fn handle_restore(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RestoreRequest>,
) -> impl IntoResponse {
    match state.service.restore(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<RestoreResponse>(e).into_response(),
    }
}

async fn handle_gc(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let req = GcRequest {};
    match state.service.gc(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<GcResponse>(e).into_response(),
    }
}

async fn handle_push(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PushRequest>,
) -> impl IntoResponse {
    match state.service.push(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<PushResponse>(e).into_response(),
    }
}

async fn handle_pull(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PullRequest>,
) -> impl IntoResponse {
    match state.service.pull(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<PullResponse>(e).into_response(),
    }
}

async fn handle_show(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ShowQuery>,
) -> impl IntoResponse {
    let req = ShowRequest {
        show_what: query.show_what,
        target_id: query.target_id,
    };
    match state.service.show(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<ShowResponse>(e).into_response(),
    }
}

async fn handle_list_pending_approvals(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.service.list_pending_approvals() {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<ListPendingApprovalsResponse>(e).into_response(),
    }
}

async fn handle_approve_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ApproveAgentRequest>,
) -> impl IntoResponse {
    match state.service.approve_agent(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<ApproveAgentResponse>(e).into_response(),
    }
}

async fn handle_reject_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RejectAgentRequest>,
) -> impl IntoResponse {
    match state.service.reject_agent(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<RejectAgentResponse>(e).into_response(),
    }
}

async fn handle_merge_to_unified(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MergeToUnifiedRequest>,
) -> impl IntoResponse {
    match state.service.merge_to_unified(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<MergeToUnifiedResponse>(e).into_response(),
    }
}

async fn handle_merge_to_staged(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MergeToStagedRequest>,
) -> impl IntoResponse {
    match state.service.merge_to_staged(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<MergeToStagedResponse>(e).into_response(),
    }
}

async fn handle_compact(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CompactRequest>,
) -> impl IntoResponse {
    match state.service.compact(req) {
        Ok(r) => ok_response(r).into_response(),
        Err(e) => err_response::<CompactResponse>(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    struct MockService;

    impl ApiService for MockService {
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

    fn test_app() -> Router {
        let state = Arc::new(AppState {
            service: Arc::new(MockService),
        });

        Router::new()
            .route("/api/v1/init", post(handle_init))
            .route("/api/v1/status", get(handle_status))
            .route("/api/v1/edit", post(handle_edit))
            .route("/api/v1/agent/{id}/edit", post(handle_agent_edit))
            .route("/api/v1/agent/{id}/submit", post(handle_agent_submit))
            .route("/api/v1/approve/{agent_id}", post(handle_approve))
            .route("/api/v1/approvals", get(handle_list_pending_approvals))
            .route("/api/v1/approve-agent", post(handle_approve_agent))
            .route("/api/v1/reject-agent", post(handle_reject_agent))
            .route("/api/v1/merge-to-unified", post(handle_merge_to_unified))
            .route("/api/v1/merge-to-staged", post(handle_merge_to_staged))
            .route("/api/v1/commit", post(handle_commit))
            .route("/api/v1/log", get(handle_log))
            .route("/api/v1/branches", get(handle_branch_list))
            .route("/api/v1/branches", post(handle_branch_create))
            .route("/api/v1/branches/{name}/switch", post(handle_branch_switch))
            .route("/api/v1/merge", post(handle_merge))
            .route("/api/v1/backup", post(handle_backup))
            .route("/api/v1/restore", post(handle_restore))
            .route("/api/v1/gc", post(handle_gc))
            .route("/api/v1/compact", post(handle_compact))
            .route("/api/v1/push", post(handle_push))
            .route("/api/v1/pull", post(handle_pull))
            .route("/api/v1/show", get(handle_show))
            .with_state(state)
    }

    // ── Envelope format tests ──

    #[tokio::test]
    async fn test_ok_response_envelope_format() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let envelope: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(envelope["success"], true);
        assert!(envelope["data"].is_object());
        assert!(envelope["error"].is_null());
    }

    #[tokio::test]
    async fn test_err_response_not_found_returns_404() {
        let state = Arc::new(AppState {
            service: Arc::new(NotFoundMockService),
        });
        let app = Router::new()
            .route("/api/v1/status", get(handle_status))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let envelope: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(envelope["success"], false);
        assert!(envelope["data"].is_null());
        assert_eq!(envelope["error"]["code"], "NOT_FOUND");
    }

    #[tokio::test]
    async fn test_err_response_invalid_params_returns_400() {
        let state = Arc::new(AppState {
            service: Arc::new(InvalidParamsMockService),
        });
        let app = Router::new()
            .route("/api/v1/edit", post(handle_edit))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/edit")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"file":"test.txt","content":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let envelope: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(envelope["error"]["code"], "INVALID_PARAMS");
    }

    #[tokio::test]
    async fn test_err_response_internal_error_returns_500() {
        let state = Arc::new(AppState {
            service: Arc::new(InternalErrorMockService),
        });
        let app = Router::new()
            .route("/api/v1/edit", post(handle_edit))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/edit")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"file":"test.txt","content":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_err_response_already_exists_returns_409() {
        let state = Arc::new(AppState {
            service: Arc::new(AlreadyExistsMockService),
        });
        let app = Router::new()
            .route("/api/v1/branches", post(handle_branch_create))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/branches")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"name":"feature"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let envelope: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(envelope["error"]["code"], "ALREADY_EXISTS");
    }

    // ── Route registration tests ──

    #[tokio::test]
    async fn test_init_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/init")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"db_path":".stratum/test.db"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_edit_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/edit")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"file":"test.txt","content":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_agent_edit_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/agent/agent-01/edit")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"file":"test.txt","content":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_agent_submit_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/agent/agent-01/submit")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_approve_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/approve/agent-01")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_commit_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/commit")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"message":"test commit"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_log_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/log?count=5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_branch_list_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/branches")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_branch_create_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/branches")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"name":"feature"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_branch_switch_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/branches/feature/switch")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_merge_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/merge")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"branch":"feature"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_backup_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/backup")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"snapshot_id":"snap1","label":"test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_restore_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/restore")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"backup_id":"backup1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_gc_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/gc")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_push_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/push")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"git_repo":"/tmp/repo"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_pull_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/pull")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"git_repo":"/tmp/repo"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_show_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/show?show_what=staged")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_compact_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/compact")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_pending_approvals_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/approvals")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_approve_agent_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/approve-agent")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"agent_id":"agent-1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_reject_agent_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/reject-agent")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"agent_id":"agent-1"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_merge_to_unified_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/merge-to-unified")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_merge_to_staged_route_success() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/merge-to-staged")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    // ── Error mock services ──

    struct NotFoundMockService;
    impl ApiService for NotFoundMockService {
        fn status(&self) -> ApiResult<StatusResponse> {
            Err(ApiError::not_found("partition 'test'"))
        }
        fn init(&self, _: InitRequest) -> ApiResult<InitResponse> {
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

    struct InvalidParamsMockService;
    impl ApiService for InvalidParamsMockService {
        fn edit(&self, _: EditRequest) -> ApiResult<EditResponse> {
            Err(ApiError::invalid_params("missing file path"))
        }
        fn init(&self, _: InitRequest) -> ApiResult<InitResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn status(&self) -> ApiResult<StatusResponse> {
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

    struct InternalErrorMockService;
    impl ApiService for InternalErrorMockService {
        fn edit(&self, _: EditRequest) -> ApiResult<EditResponse> {
            Err(ApiError::internal("db corrupted"))
        }
        fn init(&self, _: InitRequest) -> ApiResult<InitResponse> {
            Err(ApiError::internal("not impl"))
        }
        fn status(&self) -> ApiResult<StatusResponse> {
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

    struct AlreadyExistsMockService;
    impl ApiService for AlreadyExistsMockService {
        fn branch_create(&self, _req: BranchCreateRequest) -> ApiResult<BranchCreateResponse> {
            Err(ApiError {
                code: "ALREADY_EXISTS".into(),
                message: "branch 'feature' already exists".into(),
                suggestion: Some("use a different name".into()),
                details: None,
            })
        }
        fn init(&self, _: InitRequest) -> ApiResult<InitResponse> {
            Err(ApiError::internal("not impl"))
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
