use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::backup::backup_repo::BackupRepo;
use crate::checkpoint::branch::Branch;
use crate::checkpoint::checkpoint::Checkpoint;
use crate::checkpoint::repo::CheckpointRepo;
use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::snapshot::Snapshot;
use crate::core::types::LineDiff;
use crate::core::types::{
    AgentInstanceId, ContentId, DiffOp, PartitionType, SnapshotId, SourceType,
};
use crate::error::{Result as StratumResult, StratumError};
use crate::git_sync::gc::collect_garbage;
use crate::git_sync::git_bridge::GitBridge;
use crate::layered::StateMachine;
use crate::storage::repository::{
    BranchStore, CheckpointPersist, CheckpointStore, DeltaStore, FileNodeStore, PartitionStore,
    SnapshotStore,
};
use crate::storage::SqliteStorage;

use super::types::*;

/// Unified service configuration
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub db_path: String,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        ServiceConfig {
            db_path: ".stratum/stratum.db".into(),
        }
    }
}

/// Unified API service trait
///
/// All operations are synchronous (the underlying storage layer is synchronous).
/// HTTP/gRPC transport layers should wrap calls in `tokio::task::spawn_blocking`.
pub trait ApiService: Send + Sync {
    fn init(&self, req: InitRequest) -> ApiResult<InitResponse>;
    fn status(&self) -> ApiResult<StatusResponse>;
    fn edit(&self, req: EditRequest) -> ApiResult<EditResponse>;
    fn agent_edit(&self, req: AgentEditRequest) -> ApiResult<EditResponse>;
    fn agent_submit(&self, req: AgentSubmitRequest) -> ApiResult<SubmitResponse>;
    fn approve(&self, req: ApproveRequest) -> ApiResult<ApproveResponse>;
    fn commit(&self, req: CommitRequest) -> ApiResult<CommitResponse>;
    fn log(&self, req: LogRequest) -> ApiResult<LogResponse>;
    fn branch_create(&self, req: BranchCreateRequest) -> ApiResult<BranchCreateResponse>;
    fn branch_switch(&self, req: BranchSwitchRequest) -> ApiResult<BranchSwitchResponse>;
    fn branch_list(&self) -> ApiResult<BranchListResponse>;
    fn merge(&self, req: MergeRequest) -> ApiResult<MergeResponse>;
    fn backup(&self, req: BackupRequest) -> ApiResult<BackupResponse>;
    fn restore(&self, req: RestoreRequest) -> ApiResult<RestoreResponse>;
    fn gc(&self, _req: GcRequest) -> ApiResult<GcResponse>;
    fn push(&self, req: PushRequest) -> ApiResult<PushResponse>;
    fn pull(&self, req: PullRequest) -> ApiResult<PullResponse>;
    fn show(&self, req: ShowRequest) -> ApiResult<ShowResponse>;
}

// ── Helpers ──

fn open_storage(db_path: &str) -> StratumResult<Arc<SqliteStorage>> {
    let path = Path::new(db_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| StratumError::General(format!("failed to create db directory: {}", e)))?;
    }
    let storage = SqliteStorage::new_full(path).map_err(StratumError::Storage)?;
    Ok(Arc::new(storage))
}

fn load_checkpoint_repo(storage: &SqliteStorage) -> StratumResult<CheckpointRepo> {
    let persist: Box<dyn CheckpointPersist> = Box::new(storage.share());
    CheckpointRepo::load(persist)
}

fn map_error(e: StratumError) -> ApiError {
    match e {
        StratumError::Storage(se) => ApiError::storage(se.to_string()),
        StratumError::Engine(s) => ApiError::engine(s),
        StratumError::StateMachine(s) => ApiError::state_machine(s),
        StratumError::Checkpoint(s) => ApiError::checkpoint(s),
        StratumError::GitSync(s) => ApiError::git_sync(s),
        StratumError::Gc(s) => ApiError::gc(s),
        StratumError::NotFound(s) => ApiError::not_found(s),
        StratumError::Cli {
            context,
            suggestion,
        } => ApiError {
            code: "CLI_ERROR".into(),
            message: context,
            suggestion,
            details: None,
        },
        StratumError::Serialization(s) => ApiError::internal(format!("serialization: {}", s)),
        StratumError::General(s) => ApiError::general(s),
    }
}

fn snapshot_id_to_hex(id: &SnapshotId) -> String {
    id.to_hex()
}

fn checkpoint_to_info(cp: &Checkpoint) -> CheckpointInfo {
    CheckpointInfo {
        id: cp.id.to_hex(),
        author: cp.metadata.author.clone(),
        message: cp.metadata.message.clone(),
        parents: cp.parents.iter().map(|p| p.to_hex()).collect(),
        snapshots: cp.baseline_snapshots.iter().map(|s| s.to_hex()).collect(),
        created_at: cp.created_at,
        git_anchor: cp.metadata.git_anchor.clone(),
    }
}

// ── ApiServiceImpl ──

/// Default implementation of ApiService
///
/// Wraps StateMachine and SqliteStorage, providing a structured API
/// that all transport layers (CLI, HTTP, gRPC) can use.
pub struct ApiServiceImpl {
    storage: Arc<SqliteStorage>,
    state_machine: StateMachine<SqliteStorage>,
    db_path: String,
}

impl ApiServiceImpl {
    /// Open an existing stratum repository
    pub fn open(config: ServiceConfig) -> ApiResult<Self> {
        let storage = open_storage(&config.db_path).map_err(map_error)?;
        let state_machine = StateMachine::new(storage.clone());
        Ok(ApiServiceImpl {
            storage,
            state_machine,
            db_path: config.db_path,
        })
    }

    /// Reconstruct text from a snapshot by its ID
    fn reconstruct_text_from_id(&self, snapshot_id: &SnapshotId) -> ApiResult<String> {
        let snapshot = self
            .storage
            .get_snapshot(snapshot_id)
            .map_err(|e| map_error(StratumError::Storage(e)))?;
        crate::layered::transition::reconstruct_text(self.storage.as_ref(), &snapshot)
            .map_err(map_error)
    }

    /// Get the "before" text from the last checkpoint (or empty string if none)
    fn last_checkpoint_text(&self) -> String {
        match self.storage.list_checkpoints() {
            Ok(cps) if !cps.is_empty() => {
                let cp = &cps[0];
                cp.baseline_snapshots
                    .first()
                    .and_then(|sid| self.reconstruct_text_from_id(sid).ok())
                    .unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    /// Compute diff stats from a LineDiff
    fn diff_stats(diff: &LineDiff) -> (usize, usize) {
        let mut inserts = 0usize;
        let mut deletes = 0usize;
        for hunk in &diff.hunks {
            for op in &hunk.ops {
                match op {
                    DiffOp::Insert { lines, .. } => inserts += lines.len(),
                    DiffOp::Delete { count, .. } => deletes += *count as usize,
                    DiffOp::Replace {
                        old_count, lines, ..
                    } => {
                        deletes += *old_count as usize;
                        inserts += lines.len();
                    }
                    DiffOp::Equal { .. } => {}
                }
            }
        }
        (inserts, deletes)
    }

    /// Show staged changes vs last committed checkpoint
    fn show_staged(&self) -> ApiResult<ShowResponse> {
        let staged_pid = crate::layered::staged::staged_partition_id();
        let staged_partition = self
            .storage
            .get_partition(&staged_pid)
            .map_err(|e| map_error(StratumError::Storage(e)))?;
        let staged_snapshot = self
            .storage
            .get_snapshot(&staged_partition.current_snapshot)
            .map_err(|e| map_error(StratumError::Storage(e)))?;

        let new_text = self.reconstruct_text_from_id(&staged_partition.current_snapshot)?;
        let old_text = self.last_checkpoint_text();
        let file_path = staged_snapshot.file.path_str().to_string();

        let unified_diff = crate::engine::diff::format_unified_diff(&old_text, &new_text, 3);
        let line_diff = crate::engine::diff::diff_to_line_diff(&old_text, &new_text);
        let (inserts, deletes) = Self::diff_stats(&line_diff);

        Ok(ShowResponse {
            target: "staged".into(),
            diffs: vec![FileDiff {
                file_path,
                unified_diff,
                inserts,
                deletes,
            }],
        })
    }

    /// Show diff for a checkpoint vs its parent
    fn show_checkpoint(&self, id_str: &str) -> ApiResult<ShowResponse> {
        let cp_id = ContentId::from_hex(id_str).ok_or_else(|| {
            ApiError::invalid_params(format!("invalid checkpoint ID '{}'", id_str))
        })?;
        let checkpoint = self
            .storage
            .get_checkpoint(&cp_id)
            .map_err(|e| map_error(StratumError::Storage(e)))?;

        // "After" text from the checkpoint's baseline snapshot
        let new_snapshot_id = checkpoint
            .baseline_snapshots
            .first()
            .ok_or_else(|| ApiError::internal("checkpoint has no baseline snapshots"))?;
        let new_snapshot = self
            .storage
            .get_snapshot(new_snapshot_id)
            .map_err(|e| map_error(StratumError::Storage(e)))?;
        let new_text = self.reconstruct_text_from_id(new_snapshot_id)?;
        let file_path = new_snapshot.file.path_str().to_string();

        // "Before" text from the parent checkpoint's baseline snapshot
        let old_text = match checkpoint.parents.first() {
            Some(parent_id) => match self.storage.get_checkpoint(parent_id) {
                Ok(parent_cp) => parent_cp
                    .baseline_snapshots
                    .first()
                    .and_then(|sid| self.reconstruct_text_from_id(sid).ok())
                    .unwrap_or_default(),
                Err(_) => String::new(),
            },
            None => String::new(),
        };

        let unified_diff = crate::engine::diff::format_unified_diff(&old_text, &new_text, 3);
        let line_diff = crate::engine::diff::diff_to_line_diff(&old_text, &new_text);
        let (inserts, deletes) = Self::diff_stats(&line_diff);

        Ok(ShowResponse {
            target: format!("checkpoint:{}", id_str),
            diffs: vec![FileDiff {
                file_path,
                unified_diff,
                inserts,
                deletes,
            }],
        })
    }

    /// Show diff for a partition vs last checkpoint
    fn show_partition(&self, name: &str) -> ApiResult<ShowResponse> {
        let partition = self
            .storage
            .get_partition_by_name(name)
            .map_err(|_| ApiError::not_found(format!("partition '{}'", name)))?;

        let snapshot = self
            .storage
            .get_snapshot(&partition.current_snapshot)
            .map_err(|e| map_error(StratumError::Storage(e)))?;
        let new_text = self.reconstruct_text_from_id(&partition.current_snapshot)?;
        let file_path = snapshot.file.path_str().to_string();
        let old_text = self.last_checkpoint_text();

        let unified_diff = crate::engine::diff::format_unified_diff(&old_text, &new_text, 3);
        let line_diff = crate::engine::diff::diff_to_line_diff(&old_text, &new_text);
        let (inserts, deletes) = Self::diff_stats(&line_diff);

        Ok(ShowResponse {
            target: format!("partition:{}", name),
            diffs: vec![FileDiff {
                file_path,
                unified_diff,
                inserts,
                deletes,
            }],
        })
    }
}

impl ApiService for ApiServiceImpl {
    fn init(&self, req: InitRequest) -> ApiResult<InitResponse> {
        let db_path = req.db_path.clone().unwrap_or_else(|| self.db_path.clone());
        let storage = open_storage(&db_path).map_err(map_error)?;

        if let Some(git_repo_path) = &req.git_repo {
            let git_path = Path::new(git_repo_path);
            let ref_name = req.git_ref.as_deref().unwrap_or("HEAD");
            let persist: Box<dyn CheckpointPersist> = Box::new(storage.share());
            let mut checkpoint_repo = CheckpointRepo::load(persist).map_err(map_error)?;

            GitBridge::init_from_git(git_path, &*storage, &mut checkpoint_repo, ref_name)
                .map_err(map_error)?;

            // Auto-persist any metadata changes made inside init_from_git (e.g. git_anchor)
            checkpoint_repo.sync_all().map_err(map_error)?;

            Ok(InitResponse {
                db_path: db_path.clone(),
                manual_partition_id: String::new(),
                staged_partition_id: String::new(),
                branch: "main".into(),
            })
        } else {
            let file_node = FileNode::new(PathBuf::from(".stratum/init"), b"");
            storage
                .store_file_node(&file_node, b"")
                .map_err(|e| map_error(StratumError::Storage(e)))?;
            let empty_diff = Delta::new(
                file_node.clone(),
                crate::core::types::LineDiff::new(vec![]),
                SourceType::Manual,
            );
            storage
                .store_delta(&empty_diff)
                .map_err(|e| map_error(StratumError::Storage(e)))?;
            let initial_snapshot = Snapshot::new_initial(file_node, empty_diff.id);
            storage
                .store_snapshot(&initial_snapshot, b"")
                .map_err(|e| map_error(StratumError::Storage(e)))?;

            let manual_partition = crate::layered::manual::ensure_manual_partition(
                storage.as_ref(),
                initial_snapshot.id,
            )
            .map_err(map_error)?;
            let staged_partition = crate::layered::staged::ensure_staged_partition(
                storage.as_ref(),
                initial_snapshot.id,
            )
            .map_err(map_error)?;

            let persist: Box<dyn CheckpointPersist> = Box::new(storage.share());
            let mut _checkpoint_repo = CheckpointRepo::load(persist).map_err(map_error)?;

            Ok(InitResponse {
                db_path: db_path.clone(),
                manual_partition_id: manual_partition.id.to_string(),
                staged_partition_id: staged_partition.id.to_string(),
                branch: "main".into(),
            })
        }
    }

    fn status(&self) -> ApiResult<StatusResponse> {
        let partitions = self
            .storage
            .list_partitions()
            .map_err(|e| map_error(StratumError::Storage(e)))?;
        let infos = partitions
            .iter()
            .map(|p| {
                let layer = match &p.partition_type {
                    PartitionType::Manual => "manual_edit",
                    PartitionType::Agent(_) => "agent_edit",
                    PartitionType::Approval(_) => "approval",
                    PartitionType::Integrated(_) => "approval",
                    PartitionType::Unified => "approval",
                    PartitionType::Staged => "staged",
                };
                PartitionInfo {
                    layer: layer.into(),
                    name: p.name.clone(),
                    current_snapshot: p.current_snapshot.to_hex(),
                    history_len: p.history.len(),
                }
            })
            .collect();
        Ok(StatusResponse { partitions: infos })
    }

    fn edit(&self, req: EditRequest) -> ApiResult<EditResponse> {
        let content = req.content.as_deref().ok_or_else(|| {
            ApiError::invalid_params(
                "edit content is required (provide via -c/--content or pipe via stdin)",
            )
        })?;
        let snapshot_id =
            crate::layered::manual::apply_manual_edit(self.storage.as_ref(), &req.file, content)
                .map_err(map_error)?;

        let staged_snapshot_id =
            crate::layered::manual::merge_manual_to_staged(self.storage.as_ref())
                .map_err(map_error)
                .ok();

        Ok(EditResponse {
            snapshot_id: snapshot_id_to_hex(&snapshot_id),
            staged_snapshot_id: staged_snapshot_id.map(|id| snapshot_id_to_hex(&id)),
        })
    }

    fn agent_edit(&self, req: AgentEditRequest) -> ApiResult<EditResponse> {
        let agent_instance = AgentInstanceId(req.agent_id.clone());
        let content = req.content.as_deref().ok_or_else(|| {
            ApiError::invalid_params(
                "edit content is required (provide via -c/--content or pipe via stdin)",
            )
        })?;

        let staged_pid = crate::layered::staged::staged_partition_id();
        let initial_snapshot = match self.storage.get_partition(&staged_pid) {
            Ok(p) => p.current_snapshot,
            Err(_) => {
                let file_node = FileNode::new(PathBuf::from(&req.file), content.as_bytes());
                self.storage
                    .store_file_node(&file_node, content.as_bytes())
                    .map_err(|e| map_error(StratumError::Storage(e)))?;
                let delta = Delta::new(
                    file_node,
                    crate::core::types::LineDiff::new(vec![]),
                    SourceType::Agent(agent_instance.clone()),
                );
                self.storage
                    .store_delta(&delta)
                    .map_err(|e| map_error(StratumError::Storage(e)))?;
                let snapshot = Snapshot::new_initial(
                    FileNode::new(PathBuf::from(&req.file), content.as_bytes()),
                    delta.id,
                );
                self.storage
                    .store_snapshot(&snapshot, content.as_bytes())
                    .map_err(|e| map_error(StratumError::Storage(e)))?;
                snapshot.id
            }
        };

        let _ = crate::layered::agent::ensure_agent_partition(
            self.storage.as_ref(),
            &agent_instance,
            initial_snapshot,
        )
        .map_err(map_error)?;

        let snapshot_id = crate::layered::agent::apply_agent_edit(
            self.storage.as_ref(),
            &agent_instance,
            &req.file,
            content,
        )
        .map_err(map_error)?;

        Ok(EditResponse {
            snapshot_id: snapshot_id_to_hex(&snapshot_id),
            staged_snapshot_id: None,
        })
    }

    fn agent_submit(&self, req: AgentSubmitRequest) -> ApiResult<SubmitResponse> {
        let agent_instance = AgentInstanceId(req.agent_id.clone());

        let staged_pid = crate::layered::staged::staged_partition_id();
        let base_snapshot = self
            .storage
            .get_partition(&staged_pid)
            .map_err(|_| ApiError::invalid_params("no staged partition found. Make edits first."))?
            .current_snapshot;

        let _ = crate::layered::approval::ensure_approval_agent_partition(
            self.storage.as_ref(),
            &agent_instance,
            base_snapshot,
        )
        .map_err(map_error)?;

        let snapshot_id =
            crate::layered::agent::move_agent_to_approval(self.storage.as_ref(), &agent_instance)
                .map_err(map_error)?;

        Ok(SubmitResponse {
            snapshot_id: snapshot_id_to_hex(&snapshot_id),
        })
    }

    fn approve(&self, req: ApproveRequest) -> ApiResult<ApproveResponse> {
        let agent_instance = AgentInstanceId(req.agent_id.clone());

        let integrated_id = crate::layered::integrated::move_approval_to_integrated(
            self.storage.as_ref(),
            &agent_instance,
            &req.agent_id,
        )
        .map_err(map_error)?;

        let integration_names: Vec<String> = self
            .storage
            .list_partitions()
            .map_err(|e| map_error(StratumError::Storage(e)))?
            .into_iter()
            .filter_map(|p| match &p.partition_type {
                PartitionType::Integrated(name) => Some(name.clone()),
                _ => None,
            })
            .collect();

        if !integration_names.is_empty() {
            crate::layered::integrated::move_integrated_to_unified(
                self.storage.as_ref(),
                &integration_names,
            )
            .map_err(map_error)?;
        }

        let staged_id = crate::layered::staged::merge_unified_to_staged(self.storage.as_ref())
            .map_err(map_error)?;

        Ok(ApproveResponse {
            integrated_snapshot_id: snapshot_id_to_hex(&integrated_id),
            staged_snapshot_id: snapshot_id_to_hex(&staged_id),
        })
    }

    fn commit(&self, req: CommitRequest) -> ApiResult<CommitResponse> {
        let author = req.author.as_deref().unwrap_or("user");
        let cp_id = crate::layered::staged::commit_staged_to_checkpoint(
            self.storage.as_ref(),
            &req.message,
            author,
        )
        .map_err(map_error)?;

        Ok(CommitResponse {
            checkpoint_id: cp_id.to_hex(),
            message: req.message.clone(),
        })
    }

    fn log(&self, req: LogRequest) -> ApiResult<LogResponse> {
        let count = req.count.unwrap_or(20);
        let mut checkpoints = self
            .storage
            .list_checkpoints()
            .map_err(|e| map_error(StratumError::Storage(e)))?;
        checkpoints.truncate(count);
        let total = checkpoints.len();
        Ok(LogResponse {
            checkpoints: checkpoints.iter().map(checkpoint_to_info).collect(),
            total,
        })
    }

    fn branch_create(&self, req: BranchCreateRequest) -> ApiResult<BranchCreateResponse> {
        let branches = self
            .storage
            .list_branches()
            .map_err(|e| map_error(StratumError::Storage(e)))?;
        if branches.iter().any(|b| b.name == req.name) {
            return Err(ApiError::invalid_params(format!(
                "branch '{}' already exists",
                req.name
            )));
        }
        let head = match self.storage.list_checkpoints() {
            Ok(cps) if !cps.is_empty() => cps[0].id,
            _ => {
                return Err(ApiError::invalid_params(
                    "no checkpoints yet. Make a commit first.",
                ))
            }
        };
        let branch = Branch::new(&req.name, head);
        self.storage
            .store_branch(&branch)
            .map_err(|e| map_error(StratumError::Storage(e)))?;
        Ok(BranchCreateResponse {
            name: req.name,
            head: head.to_hex(),
        })
    }

    fn branch_switch(&self, req: BranchSwitchRequest) -> ApiResult<BranchSwitchResponse> {
        let _ = self
            .storage
            .get_branch(&req.name)
            .map_err(|_| ApiError::not_found(format!("branch '{}'", req.name)))?;
        let cp_id = self
            .state_machine
            .switch_branch(&req.name)
            .map_err(map_error)?;
        Ok(BranchSwitchResponse {
            name: req.name,
            checkpoint_id: cp_id.to_hex(),
        })
    }

    fn branch_list(&self) -> ApiResult<BranchListResponse> {
        let branches = self
            .storage
            .list_branches()
            .map_err(|e| map_error(StratumError::Storage(e)))?;
        let infos = branches
            .iter()
            .map(|b| BranchInfo {
                name: b.name.clone(),
                head: b.head.to_hex(),
                updated_at: b.updated_at.to_string(),
                is_current: false,
            })
            .collect();
        Ok(BranchListResponse {
            branches: infos,
            current: None,
        })
    }

    fn merge(&self, req: MergeRequest) -> ApiResult<MergeResponse> {
        let mut repo = load_checkpoint_repo(self.storage.as_ref()).map_err(map_error)?;
        let current_name = repo.current_branch_name().to_string();

        let staged_pid = crate::layered::staged::staged_partition_id();
        let snapshot_ids = match self.storage.get_partition(&staged_pid) {
            Ok(p) => vec![p.current_snapshot],
            Err(_) => return Err(ApiError::invalid_params("staged partition not found")),
        };

        let msg = req.message.clone().unwrap_or_else(|| "merge".into());
        let cp_id = repo
            .merge_branches(&req.branch, snapshot_ids, &msg, "user")
            .map_err(map_error)?;

        // merge_branches auto-persists with embedded storage
        Ok(MergeResponse {
            checkpoint_id: cp_id.to_hex(),
            source_branch: req.branch,
            target_branch: current_name,
        })
    }

    fn backup(&self, req: BackupRequest) -> ApiResult<BackupResponse> {
        let snapshot_id = ContentId::from_hex(&req.snapshot_id).ok_or_else(|| {
            ApiError::invalid_params(format!("invalid snapshot ID '{}'", req.snapshot_id))
        })?;

        let backup_path = Path::new(&self.db_path).parent().unwrap_or(Path::new("."));
        let backup_db_path = backup_path.join("stratum-backup.db");
        let backup_repo =
            BackupRepo::new(&backup_db_path).map_err(|e| map_error(StratumError::Storage(e)))?;

        let backup_id = backup_repo
            .backup_snapshot(self.storage.as_ref(), snapshot_id, req.label.clone())
            .map_err(map_error)?;

        Ok(BackupResponse {
            backup_id: backup_id.to_hex(),
            source_snapshot_id: req.snapshot_id,
            label: req.label,
        })
    }

    fn restore(&self, req: RestoreRequest) -> ApiResult<RestoreResponse> {
        let backup_id = ContentId::from_hex(&req.backup_id).ok_or_else(|| {
            ApiError::invalid_params(format!("invalid backup ID '{}'", req.backup_id))
        })?;

        let backup_path = Path::new(&self.db_path).parent().unwrap_or(Path::new("."));
        let backup_db_path = backup_path.join("stratum-backup.db");
        let backup_repo =
            BackupRepo::new(&backup_db_path).map_err(|e| map_error(StratumError::Storage(e)))?;

        let backup = backup_repo.get_backup(&backup_id).map_err(map_error)?;

        let delta_count = backup.deltas.len();
        for delta in &backup.deltas {
            self.storage
                .store_delta(delta)
                .map_err(|e| map_error(StratumError::Storage(e)))?;
        }

        Ok(RestoreResponse {
            backup_id: req.backup_id,
            file: backup.file.path_str().to_string(),
            deltas_restored: delta_count,
        })
    }

    fn gc(&self, _req: GcRequest) -> ApiResult<GcResponse> {
        let mut repo = load_checkpoint_repo(self.storage.as_ref()).map_err(map_error)?;
        let stats = collect_garbage(&mut repo).map_err(map_error)?;
        // remove_checkpoint auto-persists; sync any remaining state
        repo.sync_all().map_err(map_error)?;
        Ok(GcResponse {
            removed_checkpoints: stats.removed_checkpoints as usize,
            removed_snapshots: stats.removed_snapshots as usize,
            freed_bytes: stats.freed_bytes,
            delta_chain_depth_triggered: stats.delta_chain_depth_triggered,
        })
    }

    fn push(&self, req: PushRequest) -> ApiResult<PushResponse> {
        let remote = req.remote.unwrap_or_else(|| "origin".into());
        let message = req.message.unwrap_or_else(|| "sync from stratum".into());

        let mut repo = load_checkpoint_repo(self.storage.as_ref()).map_err(map_error)?;
        let branch_name = repo.current_branch_name().to_string();
        let git_hash = GitBridge::push_to_remote(
            self.storage.as_ref(),
            Path::new(&req.git_repo),
            &mut repo,
            &branch_name,
            &remote,
            &message,
        )
        .map_err(map_error)?;

        // Persist git_anchor changes made inside push_to_remote
        repo.sync_all().map_err(map_error)?;

        Ok(PushResponse {
            remote,
            git_commit_hash: git_hash,
        })
    }

    fn pull(&self, req: PullRequest) -> ApiResult<PullResponse> {
        let remote = req.remote.unwrap_or_else(|| "origin".into());
        let git_ref = req.git_ref.unwrap_or_else(|| "HEAD".into());

        GitBridge::fetch_from_remote(Path::new(&req.git_repo), &remote).map_err(map_error)?;

        let mut repo = load_checkpoint_repo(self.storage.as_ref()).map_err(map_error)?;

        GitBridge::init_from_git(
            Path::new(&req.git_repo),
            self.storage.as_ref(),
            &mut repo,
            &git_ref,
        )
        .map_err(map_error)?;

        // Auto-persisted via embedded storage; sync any metadata changes
        repo.sync_all().map_err(map_error)?;

        Ok(PullResponse { remote, git_ref })
    }

    fn show(&self, req: ShowRequest) -> ApiResult<ShowResponse> {
        match req.show_what.as_str() {
            "staged" => self.show_staged(),
            "checkpoint" => {
                let id = req.target_id.as_deref().ok_or_else(|| {
                    ApiError::invalid_params("checkpoint ID required for 'checkpoint' target")
                })?;
                self.show_checkpoint(id)
            }
            "partition" => {
                let name = req.target_id.as_deref().ok_or_else(|| {
                    ApiError::invalid_params("partition name required for 'partition' target")
                })?;
                self.show_partition(name)
            }
            other => Err(ApiError::invalid_params(format!(
                "unknown show target '{}'. Use 'staged', 'checkpoint', or 'partition'",
                other
            ))),
        }
    }
}
