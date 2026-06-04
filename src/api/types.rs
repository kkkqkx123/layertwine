use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── ApiError ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub suggestion: Option<String>,
    pub details: Option<Value>,
}

impl ApiError {
    pub fn not_found(entity: impl Into<String>) -> Self {
        ApiError {
            code: "NOT_FOUND".into(),
            message: format!("{} not found", entity.into()),
            suggestion: Some("check that the name or ID is correct".into()),
            details: None,
        }
    }

    pub fn invalid_params(msg: impl Into<String>) -> Self {
        ApiError {
            code: "INVALID_PARAMS".into(),
            message: msg.into(),
            suggestion: Some("check the provided parameters".into()),
            details: None,
        }
    }

    pub fn storage(msg: impl Into<String>) -> Self {
        ApiError {
            code: "STORAGE_ERROR".into(),
            message: msg.into(),
            suggestion: Some("check database integrity and permissions".into()),
            details: None,
        }
    }

    pub fn engine(msg: impl Into<String>) -> Self {
        ApiError {
            code: "ENGINE_ERROR".into(),
            message: msg.into(),
            details: None,
            suggestion: None,
        }
    }

    pub fn state_machine(msg: impl Into<String>) -> Self {
        ApiError {
            code: "STATE_MACHINE_ERROR".into(),
            message: msg.into(),
            details: None,
            suggestion: None,
        }
    }

    pub fn checkpoint(msg: impl Into<String>) -> Self {
        ApiError {
            code: "CHECKPOINT_ERROR".into(),
            message: msg.into(),
            details: None,
            suggestion: None,
        }
    }

    pub fn git_sync(msg: impl Into<String>) -> Self {
        ApiError {
            code: "GIT_SYNC_ERROR".into(),
            message: msg.into(),
            details: None,
            suggestion: None,
        }
    }

    pub fn gc(msg: impl Into<String>) -> Self {
        ApiError {
            code: "GC_ERROR".into(),
            message: msg.into(),
            details: None,
            suggestion: None,
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        ApiError {
            code: "INTERNAL_ERROR".into(),
            message: msg.into(),
            suggestion: None,
            details: None,
        }
    }

    pub fn general(msg: impl Into<String>) -> Self {
        ApiError {
            code: "ERROR".into(),
            message: msg.into(),
            suggestion: None,
            details: None,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

// ── Request types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitRequest {
    pub db_path: Option<String>,
    pub git_repo: Option<String>,
    pub git_ref: Option<String>,
}

impl Default for InitRequest {
    fn default() -> Self {
        InitRequest {
            db_path: Some(".stratum/stratum.db".into()),
            git_repo: None,
            git_ref: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditRequest {
    pub file: String,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEditRequest {
    pub agent_id: String,
    pub file: String,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSubmitRequest {
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveRequest {
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitRequest {
    pub message: String,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRequest {
    pub count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCreateRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSwitchRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeRequest {
    pub branch: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRequest {
    pub snapshot_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreRequest {
    pub backup_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    pub remote: Option<String>,
    pub git_repo: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub remote: Option<String>,
    pub git_repo: String,
    pub git_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowRequest {
    pub show_what: String,
    pub target_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowResponse {
    pub target: String,
    pub diffs: Vec<FileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub file_path: String,
    pub unified_diff: String,
    pub inserts: usize,
    pub deletes: usize,
}

// ── Response types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitResponse {
    pub db_path: String,
    pub manual_partition_id: String,
    pub staged_partition_id: String,
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub partitions: Vec<PartitionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionInfo {
    pub layer: String,
    pub name: String,
    pub current_snapshot: String,
    pub history_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditResponse {
    pub snapshot_id: String,
    pub staged_snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitResponse {
    pub snapshot_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveResponse {
    pub integrated_snapshot_id: String,
    pub staged_snapshot_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitResponse {
    pub checkpoint_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogResponse {
    pub checkpoints: Vec<CheckpointInfo>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointInfo {
    pub id: String,
    pub author: String,
    pub message: String,
    pub parents: Vec<String>,
    pub snapshots: Vec<String>,
    pub created_at: i64,
    pub git_anchor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCreateResponse {
    pub name: String,
    pub head: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSwitchResponse {
    pub name: String,
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchListResponse {
    pub branches: Vec<BranchInfo>,
    pub current: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub head: String,
    pub updated_at: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResponse {
    pub checkpoint_id: String,
    pub source_branch: String,
    pub target_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupResponse {
    pub backup_id: String,
    pub source_snapshot_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreResponse {
    pub backup_id: String,
    pub file: String,
    pub deltas_restored: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcResponse {
    pub removed_checkpoints: usize,
    pub removed_snapshots: usize,
    pub freed_bytes: u64,
    pub delta_chain_depth_triggered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResponse {
    pub remote: String,
    pub git_commit_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullResponse {
    pub remote: String,
    pub git_ref: String,
}
