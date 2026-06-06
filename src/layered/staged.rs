//! staged layer operation
//!
//! The Staged layer is the last layer before commit submission.
//! It serves as the final preparation area for checkpoint commits.
//!
//! Responsibility:
//! 1. Accept merge results from unified layer (unique entry point)
//! 2. Support final validation before checkpoint submission
//! 3. Provide checkpoint commit functionality

use crate::checkpoint::checkpoint::{Checkpoint, CheckpointMetadata};
use crate::core::delta::Delta;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{CheckpointId, PartitionId, PartitionType, SnapshotId, SourceType};
use crate::engine::diff::diff_to_line_diff;
use crate::error::{Result, StratumError};
use crate::storage::repository::{
    BranchStore, CheckpointStore, DagStore, DeltaStore, FileNodeStore, PartitionStore,
    SnapshotStore,
};

/// Validation result for staged before commit
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationResult {
    /// Staged is ready to commit
    Ready,
    /// Staged has unresolved conflicts
    HasConflicts(Vec<String>),
    /// Staged has other unresolved problems
    HasUnresolvedProblems(Vec<String>),
}

impl ValidationResult {
    /// Check if staged is ready to commit
    pub fn is_ready(&self) -> bool {
        matches!(self, ValidationResult::Ready)
    }

    /// Get error messages if validation failed
    pub fn get_errors(&self) -> Vec<String> {
        match self {
            ValidationResult::Ready => vec![],
            ValidationResult::HasConflicts(errors) => errors.clone(),
            ValidationResult::HasUnresolvedProblems(errors) => errors.clone(),
        }
    }
}

/// Fixed ID of the staged partition
pub fn staged_partition_id() -> PartitionId {
    uuid::Uuid::from_u128(0x6000_0000_0000_0000_0000_0000_0000_0000)
}

/// Getting or creating staged partitions
pub fn ensure_staged_partition<S: PartitionStore>(
    storage: &S,
    initial_snapshot_id: SnapshotId,
) -> Result<Partition> {
    let pid = staged_partition_id();
    match storage.get_partition(&pid) {
        Ok(p) => Ok(p),
        Err(_) => {
            let partition = Partition {
                id: pid,
                name: "staged".to_string(),
                current_snapshot: initial_snapshot_id,
                history: vec![initial_snapshot_id],
                partition_type: PartitionType::Staged,
            };
            storage
                .create_partition(&partition)
                .map_err(StratumError::Storage)?;
            Ok(partition)
        }
    }
}

/// Merge the contents of the approval Agent partition into the staged
///
/// Takes the current snapshot of the specified Agent partition from the approval level and merges it into staged.
pub fn merge_approval_to_staged<S>(
    storage: &S,
    approval_partition_id: &PartitionId,
) -> Result<SnapshotId>
where
    S: SnapshotStore + DeltaStore + FileNodeStore + PartitionStore,
{
    let staged_pid = staged_partition_id();

    let approval_partition = storage
        .get_partition(approval_partition_id)
        .map_err(|_| StratumError::NotFound("approval partition not found".into()))?;
    let staged_partition = storage.get_partition(&staged_pid).map_err(|_| {
        StratumError::NotFound(
            "staged partition not found, call ensure_staged_partition first".into(),
        )
    })?;

    let approval_snapshot = storage
        .get_snapshot(&approval_partition.current_snapshot)
        .map_err(StratumError::Storage)?;
    let staged_snapshot = storage
        .get_snapshot(&staged_partition.current_snapshot)
        .map_err(StratumError::Storage)?;

    let approval_text = crate::layered::transition::reconstruct_text(storage, &approval_snapshot)?;
    let staged_text = crate::layered::transition::reconstruct_text(storage, &staged_snapshot)?;

    let merge_diff = diff_to_line_diff(&staged_text, &approval_text);
    if merge_diff.is_empty() {
        return Ok(staged_partition.current_snapshot);
    }

    let merge_delta = Delta::new(staged_snapshot.file.clone(), merge_diff, SourceType::Manual);
    storage
        .store_delta(&merge_delta)
        .map_err(StratumError::Storage)?;

    let new_snapshot = Snapshot::merge(
        vec![&staged_snapshot, &approval_snapshot],
        merge_delta.id,
        PartitionType::Staged.name(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(StratumError::Storage)?;

    storage
        .update_pointer(&staged_pid, &new_snapshot.id)
        .map_err(StratumError::Storage)?;

    Ok(new_snapshot.id)
}

/// Merge the contents of the Unified partition into the staged
/// 
/// This is the UNIQUE entry point for merging into staged.
/// All content should flow through the unified layer before reaching staged.
/// This ensures proper three-way merging and conflict detection.
///
/// This is the final step of the approval pipeline: approval_agent → integrated → unified → staged.
pub fn merge_unified_to_staged<S>(storage: &S) -> Result<SnapshotId>
where
    S: SnapshotStore + DeltaStore + FileNodeStore + PartitionStore,
{
    let staged_pid = staged_partition_id();
    let unified_pid = crate::layered::integrated::unified_partition_id();

    let unified_partition = storage.get_partition(&unified_pid).map_err(|_| {
        StratumError::NotFound(
            "unified partition not found, call ensure_unified_partition first".into(),
        )
    })?;
    let staged_partition = storage.get_partition(&staged_pid).map_err(|_| {
        StratumError::NotFound(
            "staged partition not found, call ensure_staged_partition first".into(),
        )
    })?;

    // If unified and staged point to the same snapshot, no merge needed
    if unified_partition.current_snapshot == staged_partition.current_snapshot {
        return Ok(staged_partition.current_snapshot);
    }

    let unified_snapshot = storage
        .get_snapshot(&unified_partition.current_snapshot)
        .map_err(StratumError::Storage)?;
    let staged_snapshot = storage
        .get_snapshot(&staged_partition.current_snapshot)
        .map_err(StratumError::Storage)?;

    let unified_text = crate::layered::transition::reconstruct_text(storage, &unified_snapshot)?;
    let staged_text = crate::layered::transition::reconstruct_text(storage, &staged_snapshot)?;

    let merge_diff = diff_to_line_diff(&staged_text, &unified_text);
    if merge_diff.is_empty() {
        return Ok(staged_partition.current_snapshot);
    }

    let merge_delta = Delta::new(staged_snapshot.file.clone(), merge_diff, SourceType::Manual);
    storage
        .store_delta(&merge_delta)
        .map_err(StratumError::Storage)?;

    let new_snapshot = Snapshot::merge(
        vec![&staged_snapshot, &unified_snapshot],
        merge_delta.id,
        PartitionType::Staged.name(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(StratumError::Storage)?;

    storage
        .update_pointer(&staged_pid, &new_snapshot.id)
        .map_err(StratumError::Storage)?;

    Ok(new_snapshot.id)
}

/// Validate staged before commit
///
/// Checks if staged is ready to commit by:
/// 1. Verifying staged contains all content from unified
/// 2. Checking for unresolved conflicts
/// 3. Checking for other problems
pub fn validate_staged_for_commit<S>(storage: &S) -> Result<ValidationResult>
where
    S: SnapshotStore + PartitionStore,
{
    let unified_pid = crate::layered::integrated::unified_partition_id();
    let staged_pid = staged_partition_id();

    let unified = storage.get_partition(&unified_pid).map_err(|_| {
        StratumError::NotFound("unified partition not found".into())
    })?;
    let staged = storage.get_partition(&staged_pid).map_err(|_| {
        StratumError::NotFound("staged partition not found".into())
    })?;

    // Check if staged contains unified content
    if staged.current_snapshot == unified.current_snapshot {
        Ok(ValidationResult::Ready)
    } else {
        // Check if there are unresolved conflicts or other problems
        // TODO: Need to record conflict status during merge
        // For now, we assume if staged != unified, there might be pending changes
        Ok(ValidationResult::HasUnresolvedProblems(vec![
            "Staged does not contain all unified content. Call merge_unified_to_staged first.".to_string(),
        ]))
    }
}

/// Submit staged as Checkpoint
///
/// 1. Get staged partition current snapshot
/// 2. Get current branch head from BranchStore
/// 3. Build a Checkpoint with the snapshot as baseline
/// 4. Store the checkpoint via CheckpointStore
/// 5. Store updated DAG via DagStore
/// 6. Update branch head via BranchStore
/// 7. Return the new CheckpointId
pub fn commit_staged_to_checkpoint<S>(
    storage: &S,
    branch_name: &str,
    message: &str,
    author: &str,
) -> Result<CheckpointId>
where
    S: SnapshotStore + PartitionStore + CheckpointStore + BranchStore + DagStore,
{
    // 1. Get staged partition
    let staged_pid = staged_partition_id();
    let staged_partition = storage
        .get_partition(&staged_pid)
        .map_err(|_| StratumError::NotFound("staged partition not found".into()))?;
    let current_snapshot_id = staged_partition.current_snapshot;

    // 2. Get or create branch
    let branch_head = match storage.get_branch(branch_name) {
        Ok(b) => b.head,
        Err(_) => {
            // First commit: create initial branch pointing to the staged snapshot
            let branch = crate::checkpoint::branch::Branch::new(branch_name, current_snapshot_id);
            storage
                .store_branch(&branch)
                .map_err(StratumError::Storage)?;
            current_snapshot_id
        }
    };

    // 3. Build Checkpoint
    let metadata = CheckpointMetadata::new(author, message);
    let cp = Checkpoint::new(vec![current_snapshot_id], vec![branch_head], metadata);
    let cp_id = cp.id;

    // 4. Store checkpoint
    storage
        .store_checkpoint(&cp)
        .map_err(StratumError::Storage)?;

    // 5. Update DAG (load, add edge, store)
    let mut dag = storage.load_dag().map_err(StratumError::Storage)?;
    dag.add_node(cp_id);
    dag.add_edge(branch_head, cp_id);
    storage.store_dag(&dag).map_err(StratumError::Storage)?;

    // 6. Update branch head
    storage
        .update_branch_head(branch_name, &cp_id)
        .map_err(StratumError::Storage)?;

    Ok(cp_id)
}

/// Empty staged partition (reset to initial state)
pub fn reset_staged<S: PartitionStore>(storage: &S, base_snapshot_id: SnapshotId) -> Result<()> {
    let pid = staged_partition_id();
    storage
        .update_pointer(&pid, &base_snapshot_id)
        .map_err(StratumError::Storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::Delta;
    use crate::core::file_node::FileNode;
    use crate::core::snapshot::Snapshot;
    use crate::core::types::{AgentInstanceId, PartitionType, SourceType};
    use crate::engine::diff::diff_to_line_diff;
    use crate::storage::repository::FileNodeStore;
    use crate::storage::repository::{DeltaStore, SnapshotStore};
    use crate::storage::SqliteStorage;

    fn setup_storage() -> SqliteStorage {
        let storage = SqliteStorage::new_in_memory().unwrap();
        storage
            .with_conn(|conn| crate::storage::migrations::initialize_full(conn))
            .unwrap();
        storage
    }

    fn create_initial_snapshot(
        storage: &SqliteStorage,
        content: &str,
    ) -> crate::core::types::SnapshotId {
        let file_node = FileNode::new(std::path::PathBuf::from("test.txt"), content.as_bytes());
        storage
            .store_file_node(&file_node, content.as_bytes())
            .unwrap();
        let empty_diff = crate::core::types::LineDiff::new(vec![]);
        let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        let snapshot = Snapshot::new_initial(file_node, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();
        snapshot.id
    }

    fn create_snapshot_with_content(
        storage: &SqliteStorage,
        parent_id: &crate::core::types::SnapshotId,
        content: &str,
        partition_type: &str,
    ) -> crate::core::types::SnapshotId {
        let parent = storage.get_snapshot(parent_id).unwrap();
        let file_node = FileNode::new(std::path::PathBuf::from("test.txt"), content.as_bytes());
        storage
            .store_file_node(&file_node, content.as_bytes())
            .unwrap();

        let parent_text = crate::layered::transition::reconstruct_text(storage, &parent).unwrap();
        let diff = diff_to_line_diff(&parent_text, content);
        let delta = Delta::new(file_node, diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();

        let snap = Snapshot::from_parent(&parent, delta.id, partition_type.to_string());
        storage.store_snapshot(&snap, b"").unwrap();
        snap.id
    }

    fn create_approval_partition(storage: &SqliteStorage, content: &str) -> PartitionId {
        let file_path = "test.txt";
        let agent_id = AgentInstanceId("test-agent".into());
        let initial_id = create_initial_snapshot(storage, "base\n");
        let pid = crate::layered::approval::approval_agent_partition_id(&agent_id);
        let partition = Partition {
            id: pid,
            name: format!("approval/{}", agent_id),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Approval(agent_id.clone()),
        };
        storage.create_partition(&partition).unwrap();

        let file_node = FileNode::new(std::path::PathBuf::from(file_path), content.as_bytes());
        storage
            .store_file_node(&file_node, content.as_bytes())
            .unwrap();
        let diff = diff_to_line_diff("base\n", content);
        let delta = Delta::new(file_node, diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        let snap = storage.get_snapshot(&initial_id).unwrap();
        let new_snap = Snapshot::from_parent(
            &snap,
            delta.id,
            PartitionType::Approval(AgentInstanceId("test-agent".into())).name(),
        );
        storage.store_snapshot(&new_snap, b"").unwrap();
        storage.update_pointer(&pid, &new_snap.id).unwrap();
        pid
    }

    #[test]
    fn test_merge_approval_to_staged() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");
        ensure_staged_partition(&storage, initial_id).unwrap();

        let approval_pid = create_approval_partition(&storage, "base\nmodified\n");
        let merged_id = merge_approval_to_staged(&storage, &approval_pid).unwrap();

        let staged = storage.get_partition(&staged_partition_id()).unwrap();
        assert_eq!(staged.current_snapshot, merged_id);

        let merged = storage.get_snapshot(&merged_id).unwrap();
        assert_eq!(merged.parents.len(), 2);
    }

    #[test]
    fn test_ensure_staged_partition() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        let p1 = ensure_staged_partition(&storage, initial_id).unwrap();
        let p2 = ensure_staged_partition(&storage, initial_id).unwrap();
        assert_eq!(p1.id, p2.id);
    }

    #[test]
    fn test_merge_approval_to_staged_no_changes() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");
        ensure_staged_partition(&storage, initial_id).unwrap();

        let approval_pid = create_approval_partition(&storage, "base\n");
        let result = merge_approval_to_staged(&storage, &approval_pid);
        assert!(result.is_ok());

        let staged = storage.get_partition(&staged_partition_id()).unwrap();
        assert_eq!(staged.current_snapshot, initial_id);
    }

    #[test]
    fn test_merge_unified_to_staged() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");
        ensure_staged_partition(&storage, initial_id).unwrap();

        let unified_pid = crate::layered::integrated::unified_partition_id();
        let unified_part = Partition {
            id: unified_pid,
            name: "unified".to_string(),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Unified,
        };
        storage.create_partition(&unified_part).unwrap();

        let modified_id =
            create_snapshot_with_content(&storage, &initial_id, "base\nmodified\n", "unified");
        storage.update_pointer(&unified_pid, &modified_id).unwrap();

        let merged_id = merge_unified_to_staged(&storage).unwrap();
        assert!(
            merged_id != initial_id,
            "should create new snapshot when there are changes"
        );

        let staged = storage.get_partition(&staged_partition_id()).unwrap();
        assert_eq!(staged.current_snapshot, merged_id);

        let merged = storage.get_snapshot(&merged_id).unwrap();
        assert_eq!(merged.parents.len(), 2);
    }

    #[test]
    fn test_merge_unified_to_staged_no_changes() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");
        ensure_staged_partition(&storage, initial_id).unwrap();

        let unified_pid = crate::layered::integrated::unified_partition_id();
        let unified_part = Partition {
            id: unified_pid,
            name: "unified".to_string(),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Unified,
        };
        storage.create_partition(&unified_part).unwrap();

        let result = merge_unified_to_staged(&storage).unwrap();
        assert_eq!(
            result, initial_id,
            "should return initial id when no changes"
        );
    }

    #[test]
    fn test_commit_staged_to_checkpoint() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");
        ensure_staged_partition(&storage, initial_id).unwrap();

        let cp_id = commit_staged_to_checkpoint(&storage, "main", "test commit", "test-author").unwrap();

        let checkpoint = storage.get_checkpoint(&cp_id).unwrap();
        assert_eq!(checkpoint.baseline_snapshots.len(), 1);
        assert_eq!(checkpoint.baseline_snapshots[0], initial_id);

        let branch = storage.get_branch("main").unwrap();
        assert_eq!(branch.head, cp_id);
    }

    #[test]
    fn test_commit_staged_to_checkpoint_multiple() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");
        ensure_staged_partition(&storage, initial_id).unwrap();

        let cp_id1 = commit_staged_to_checkpoint(&storage, "main", "first commit", "test-author").unwrap();
        let cp_id2 = commit_staged_to_checkpoint(&storage, "main", "second commit", "test-author").unwrap();

        assert_ne!(
            cp_id1, cp_id2,
            "different commits should have different IDs"
        );

        let branch = storage.get_branch("main").unwrap();
        assert_eq!(
            branch.head, cp_id2,
            "branch head should point to latest commit"
        );

        let dag = storage.load_dag().unwrap();
        assert!(
            dag.is_ancestor(&cp_id1, &cp_id2),
            "first commit should be ancestor of second"
        );
    }

    #[test]
    fn test_reset_staged() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");
        ensure_staged_partition(&storage, initial_id).unwrap();

        let file_node = FileNode::new(std::path::PathBuf::from("test.txt"), b"base\nmodified\n");
        storage
            .store_file_node(&file_node, b"base\nmodified\n")
            .unwrap();
        let diff = diff_to_line_diff("base\n", "base\nmodified\n");
        let delta = Delta::new(file_node, diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        let snap = storage.get_snapshot(&initial_id).unwrap();
        let new_snap = Snapshot::from_parent(&snap, delta.id, PartitionType::Staged.name());
        storage.store_snapshot(&new_snap, b"").unwrap();
        let staged_pid = staged_partition_id();
        storage.update_pointer(&staged_pid, &new_snap.id).unwrap();

        let staged = storage.get_partition(&staged_pid).unwrap();
        assert_ne!(staged.current_snapshot, initial_id);

        reset_staged(&storage, initial_id).unwrap();
        let staged = storage.get_partition(&staged_pid).unwrap();
        assert_eq!(staged.current_snapshot, initial_id);
    }

    #[test]
    fn test_reset_staged_at_base() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");
        ensure_staged_partition(&storage, initial_id).unwrap();

        reset_staged(&storage, initial_id).unwrap();
        let staged = storage.get_partition(&staged_partition_id()).unwrap();
        assert_eq!(staged.current_snapshot, initial_id);
    }
}
