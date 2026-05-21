//! staged layer operation
//!
//The Staged layer is the last layer before submission. The Staged layer is the last layer before the commit, and the contents of the Approval layer are merged into this layer.
//! can be packaged as a Checkpoint via commit (placeholder, P4 implements specific logic).

use crate::core::delta::Delta;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{PartitionId, PartitionType, SnapshotId, SourceType};
use crate::engine::diff::diff_to_line_diff;
use crate::error::{Result, StratumError};
use crate::storage::repository::{DeltaStore, PartitionStore, SnapshotStore};
use crate::storage::sqlite_storage::SqliteStorage;

/// Fixed ID of the staged partition
pub fn staged_partition_id() -> PartitionId {
    uuid::Uuid::from_u128(0x6000_0000_0000_0000_0000_0000_0000_0000)
}

/// Getting or creating staged partitions
pub fn ensure_staged_partition(
    storage: &SqliteStorage,
    initial_snapshot_id: SnapshotId,
) -> Result<Partition> {
    let pid = staged_partition_id();
    match storage.get_partition(&pid) {
        Ok(p) => Ok(p),
        Err(_) => {
            let partition = Partition::new(
                "staged".to_string(),
                PartitionType::Staged,
                initial_snapshot_id,
            );
            storage
                .create_partition(&partition)
                .map_err(|e| StratumError::Storage(e.into()))?;
            Ok(partition)
        }
    }
}

/// Merge the contents of the approval Agent partition into the staged
///
/// Takes the current snapshot of the specified Agent partition from the approval level and merges it into staged.
pub fn merge_approval_to_staged(
    storage: &SqliteStorage,
    approval_partition_id: &PartitionId,
) -> Result<SnapshotId> {
    let staged_pid = staged_partition_id();

    let approval_partition = storage
        .get_partition(approval_partition_id)
        .map_err(|_| StratumError::NotFound("approval partition not found".into()))?;
    let staged_partition = storage
        .get_partition(&staged_pid)
        .map_err(|_| StratumError::NotFound("staged partition not found, call ensure_staged_partition first".into()))?;

    let approval_snapshot = storage
        .get_snapshot(&approval_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;
    let staged_snapshot = storage
        .get_snapshot(&staged_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    let approval_text =
        crate::state_machine::transition::reconstruct_text(storage, &approval_snapshot)?;
    let staged_text =
        crate::state_machine::transition::reconstruct_text(storage, &staged_snapshot)?;

    let merge_diff = diff_to_line_diff(&staged_text, &approval_text);
    if merge_diff.is_empty() {
        return Ok(staged_partition.current_snapshot);
    }

    let merge_delta = Delta::new(
        staged_snapshot.file.clone(),
        merge_diff,
        SourceType::Manual,
    );
    storage
        .store_delta(&merge_delta)
        .map_err(|e| StratumError::Storage(e.into()))?;

    let new_snapshot = Snapshot::merge(
        vec![&staged_snapshot, &approval_snapshot],
        merge_delta.id,
        PartitionType::Staged.name(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(|e| StratumError::Storage(e.into()))?;

    storage
        .update_pointer(&staged_pid, &new_snapshot.id)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(new_snapshot.id)
}

/// Submit staged as Checkpoint (placeholder, P4 implements specific logic)
///
/// Currently only the staged current snapshot ID and placeholder CheckpointId are returned.
/// The full Checkpoint submission logic will be implemented in P4.
pub fn commit_staged_to_checkpoint(
    _storage: &SqliteStorage,
    _message: &str,
) -> Result<()> {
    // P4 Implementation: Packaging the staged current snapshot as a Checkpoint
    // Add to DAG, update branch head, clear staged
    Err(StratumError::Checkpoint(
        "checkpoint commit not yet implemented in P3, see P4".into(),
    ))
}

/// Empty staged partition (reset to initial state)
pub fn reset_staged(
    storage: &SqliteStorage,
    base_snapshot_id: SnapshotId,
) -> Result<()> {
    let pid = staged_partition_id();
    storage
        .update_pointer(&pid, &base_snapshot_id)
        .map_err(|e| StratumError::Storage(e.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::file_node::FileNode;
    use crate::core::types::{AgentInstanceId, PartitionType, SourceType};
    use crate::storage::repository::FileNodeStore;
    use crate::storage::sqlite_storage::SqliteStorage;
    use std::sync::Arc;

    fn setup_storage() -> Arc<SqliteStorage> {
        Arc::new(SqliteStorage::new_in_memory().unwrap())
    }

    fn create_initial_snapshot(storage: &SqliteStorage, content: &str) -> SnapshotId {
        let file_node = FileNode::new(std::path::PathBuf::from("test.txt"), content.as_bytes());
        storage.store_file_node(&file_node, content.as_bytes()).unwrap();
        let empty_diff = crate::core::delta::LineDiff::new(vec![]);
        let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        let snapshot = Snapshot::new_initial(file_node, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();
        snapshot.id
    }

    fn create_approval_partition(
        storage: &SqliteStorage,
        content: &str,
    ) -> PartitionId {
        let file_path = "test.txt";
        let agent_id = AgentInstanceId("test-agent".into());
        let initial_id = create_initial_snapshot(storage, "base\n");
        let pid = crate::state_machine::approval::approval_agent_partition_id(&agent_id);
        let partition = Partition::new(
            format!("approval/{}", agent_id),
            PartitionType::Approval(agent_id),
            initial_id,
        );
        storage.create_partition(&partition).unwrap();

        // Creating a modified snapshot
        let file_node = FileNode::new(std::path::PathBuf::from(file_path), content.as_bytes());
        storage.store_file_node(&file_node, content.as_bytes()).unwrap();
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
}
