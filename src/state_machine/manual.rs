//Manual_edit level operations manual_edit Layer Operation
//!
//! Manual edits are grouped into manual_edit layers, which can be merged into staged layers via merge.

use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{
    PartitionId, PartitionType, SnapshotId, SourceType,
};
use crate::engine::diff::diff_to_line_diff;
use crate::engine::merge::apply_deltas;
use crate::error::{Result, StratumError};
use crate::storage::repository::{DeltaStore, FileNodeStore, PartitionStore, SnapshotStore};
use crate::storage::sqlite_storage::SqliteStorage;
use std::path::PathBuf;
/// Get the partition ID of the manual_edit level
pub fn manual_partition_id() -> PartitionId {
    uuid::Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0001)
}

/// Get or create manual_edit partition
pub fn ensure_manual_partition(storage: &SqliteStorage, initial_snapshot_id: SnapshotId) -> Result<Partition> {
    let pid = manual_partition_id();
    match storage.get_partition(&pid) {
        Ok(p) => Ok(p),
        Err(_) => {
            let partition = Partition::new(
                "manual_edit".to_string(),
                PartitionType::Manual,
                initial_snapshot_id,
            );
            storage
                .create_partition(&partition)
                .map_err(|e| StratumError::Storage(e.into()))?;
            Ok(partition)
        }
    }
}

/// Apply manual editing to specified files
///
/// 1. read old_content (from file_node or empty string)
/// 2. Calculate old ↔ new Delta
/// 3. Create a new Snapshot to append to the manual_edit partition
/// 4. Return the new Snapshot ID
pub fn apply_manual_edit(
    storage: &SqliteStorage,
    file_path: &str,
    new_content: &str,
) -> Result<SnapshotId> {
    // Get the current snapshot of the manual_edit partition
    let pid = manual_partition_id();
    let partition = storage
        .get_partition(&pid)
        .map_err(|_| StratumError::NotFound("manual_edit partition not found, call ensure_manual_partition first".into()))?;

    let current_snapshot = storage
        .get_snapshot(&partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    // Read old content
    let old_content = {
        let deltas = storage
            .get_deltas(&current_snapshot.deltas)
            .map_err(|e| StratumError::Storage(e.into()))?;
        let content_str = String::from_utf8_lossy(
            &storage
                .get_file_content(&current_snapshot.file)
                .map_err(|e| StratumError::Storage(e.into()))?,
        )
        .to_string();
        apply_deltas(&content_str, &deltas)
            .map_err(|e| StratumError::Engine(e.to_string()))?
    };

    // Calculate diff
    let line_diff = diff_to_line_diff(&old_content, new_content);
    if line_diff.is_empty() {
        return Ok(partition.current_snapshot); // No change, return current snapshot
    }

    // Create Delta
    let file_node = FileNode::new(PathBuf::from(file_path), old_content.as_bytes());
    let delta = Delta::new(file_node.clone(), line_diff, SourceType::Manual);
    storage
        .store_file_node(&file_node, old_content.as_bytes())
        .map_err(|e| StratumError::Storage(e.into()))?;
    storage
        .store_delta(&delta)
        .map_err(|e| StratumError::Storage(e.into()))?;
    storage
        .store_file_node(&file_node, new_content.as_bytes())
        .map_err(|e| StratumError::Storage(e.into()))?;

    // Creating a New Snapshot
    let new_snapshot = Snapshot::from_parent(
        &current_snapshot,
        delta.id,
        PartitionType::Manual.name().to_string(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(|e| StratumError::Storage(e.into()))?;

    // Updating the partition pointer
    storage
        .update_pointer(&pid, &new_snapshot.id)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(new_snapshot.id)
}

/// Merge the current snapshot of the manual_edit tier into staged
///
/// Take the current Snapshot from manual_edit and staged and merge it to create a new Snapshot to push into the staged history.
pub fn merge_manual_to_staged(
    storage: &SqliteStorage,
) -> Result<SnapshotId> {
    let manual_pid = manual_partition_id();
    let staged_pid = crate::state_machine::staged::staged_partition_id();

    // Get manual and staged partitions
    let manual_partition = storage
        .get_partition(&manual_pid)
        .map_err(|_| StratumError::NotFound("manual_edit partition not found".into()))?;
    let staged_partition = storage
        .get_partition(&staged_pid)
        .map_err(|_| StratumError::NotFound("staged partition not found".into()))?;

    let manual_snapshot = storage
        .get_snapshot(&manual_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;
    let staged_snapshot = storage
        .get_snapshot(&staged_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    // Reconstructing text content
    let manual_text = crate::state_machine::transition::reconstruct_text(storage, &manual_snapshot)?;
    let staged_text = crate::state_machine::transition::reconstruct_text(storage, &staged_snapshot)?;

    // Incorporate manual changes using staged as a baseline.
    // Calculate the diff of manual_text relative to staged_text
    let merge_diff = diff_to_line_diff(&staged_text, &manual_text);
    if merge_diff.is_empty() {
        return Ok(staged_partition.current_snapshot); // no change
    }

    let merge_delta = Delta::new(
        staged_snapshot.file.clone(),
        merge_diff,
        SourceType::Manual,
    );
    storage
        .store_delta(&merge_delta)
        .map_err(|e| StratumError::Storage(e.into()))?;

    // Create merge snapshot (dual parent)
    let new_snapshot = Snapshot::merge(
        vec![&staged_snapshot, &manual_snapshot],
        merge_delta.id,
        PartitionType::Staged.name().to_string(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(|e| StratumError::Storage(e.into()))?;

    // Update staged pointer
    storage
        .update_pointer(&staged_pid, &new_snapshot.id)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(new_snapshot.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::SourceType;
    use crate::storage::sqlite_storage::SqliteStorage;
    use std::sync::Arc;

    fn setup_storage() -> Arc<SqliteStorage> {
        Arc::new(SqliteStorage::new_in_memory().unwrap())
    }

    fn create_initial_snapshot(storage: &SqliteStorage, content: &str) -> SnapshotId {
        let file_path = "test.txt";
        let file_node = FileNode::new(std::path::PathBuf::from(file_path), content.as_bytes());
        storage.store_file_node(&file_node, content.as_bytes()).unwrap();

        let empty_diff = crate::core::delta::LineDiff::new(vec![]);
        let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();

        let snapshot = Snapshot::new_initial(file_node, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();
        snapshot.id
    }

    #[test]
    fn test_apply_manual_edit() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "hello\nworld\n");
        ensure_manual_partition(&storage, initial_id).unwrap();

        let new_id = apply_manual_edit(&storage, "test.txt", "hello\nrust\n").unwrap();
        assert_ne!(new_id, initial_id);

        // Validate Snapshot Chain
        let snapshot = storage.get_snapshot(&new_id).unwrap();
        assert_eq!(snapshot.parents.len(), 1);
        assert_eq!(snapshot.parents[0], initial_id);
    }

    #[test]
    fn test_merge_manual_to_staged() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\ncontent\n");

        // Create manual and staged partitions that point to the same initial snapshot
        ensure_manual_partition(&storage, initial_id).unwrap();
        crate::state_machine::staged::ensure_staged_partition(&storage, initial_id).unwrap();

        // Apply edits to the manual layer
        apply_manual_edit(&storage, "test.txt", "base\nmodified\n").unwrap();

        // Merge to staged
        let merged_id = merge_manual_to_staged(&storage).unwrap();
        let staged = storage.get_partition(&crate::state_machine::staged::staged_partition_id()).unwrap();
        assert_eq!(staged.current_snapshot, merged_id);

        // Verify the dual parent of the merge snapshot
        let merged = storage.get_snapshot(&merged_id).unwrap();
        assert_eq!(merged.parents.len(), 2);
    }
}
