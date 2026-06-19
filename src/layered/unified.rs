//! Unified partition operations
//!
//! The Unified partition is the aggregation point between Integrated and Staged layers.
//! Individual feature (Integrated) partitions are merged here via three-way merge
//! using each feature's own baseline, before flowing into Staged.
//!
//! Flow: approval_agent → integrated → unified → staged

use crate::core::delta::Delta;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{PartitionId, PartitionType, SnapshotId, SourceType};
use crate::engine::diff::diff_to_line_diff;
use crate::error::{Result, StratumError};
use crate::layered::MergeResult;
use crate::storage::repository::{DeltaStore, FileNodeStore, PartitionStore, SnapshotStore};

/// Fixed ID of the Unified partition
pub fn unified_partition_id() -> PartitionId {
    uuid::Uuid::from_u128(0x5000_0000_0000_0000_0000_0000_0000_0000)
}

/// Get or create the Unified partition
pub fn ensure_unified_partition<S: PartitionStore>(
    storage: &S,
    initial_snapshot_id: SnapshotId,
) -> Result<Partition> {
    let pid = unified_partition_id();
    match storage.get_partition(&pid) {
        Ok(p) => Ok(p),
        Err(_) => {
            let partition = Partition {
                id: pid,
                name: "unified".to_string(),
                current_snapshot: initial_snapshot_id,
                history: vec![initial_snapshot_id],
                partition_type: PartitionType::Unified,
            };
            storage
                .create_partition(&partition)
                .map_err(StratumError::Storage)?;
            Ok(partition)
        }
    }
}

/// Merge a single Integrated feature partition into Unified
///
/// Uses three-way merge with the feature's original baseline as the merge base:
///   merge_base = feature.history[0]
///   ours       = unified.current_snapshot
///   theirs     = feature.current_snapshot
///
/// This avoids the brittle "all features share the same baseline" assumption.
pub fn merge_feature_to_unified<S>(storage: &S, feature_name: &str) -> Result<MergeResult>
where
    S: SnapshotStore + DeltaStore + FileNodeStore + PartitionStore,
{
    let unified_pid = unified_partition_id();
    let integrated_pid = crate::layered::integrated::integrated_partition_id(feature_name);

    // Get the feature (Integrated) partition and its baseline
    let feature_part = storage.get_partition(&integrated_pid).map_err(|_| {
        StratumError::NotFound(format!("integrated partition '{}' not found", feature_name))
    })?;
    let baseline_id = feature_part.history.first().ok_or_else(|| {
        StratumError::StateMachine(format!(
            "integrated partition '{}' has empty history",
            feature_name
        ))
    })?;
    let baseline_snapshot = storage
        .get_snapshot(baseline_id)
        .map_err(StratumError::Storage)?;
    let feature_snapshot = storage
        .get_snapshot(&feature_part.current_snapshot)
        .map_err(StratumError::Storage)?;

    // Get or create Unified partition
    let unified_partition = ensure_unified_partition(storage, *baseline_id)?;

    // If unified already points to the same snapshot as the feature, no merge needed
    if unified_partition.current_snapshot == feature_part.current_snapshot {
        return Ok(MergeResult {
            snapshot_id: unified_partition.current_snapshot,
            conflicts: vec![],
        });
    }

    let unified_snapshot = storage
        .get_snapshot(&unified_partition.current_snapshot)
        .map_err(StratumError::Storage)?;

    // Reconstruct texts
    let baseline_text = crate::layered::transition::reconstruct_text(storage, &baseline_snapshot)?;
    let unified_text = crate::layered::transition::reconstruct_text(storage, &unified_snapshot)?;
    let feature_text = crate::layered::transition::reconstruct_text(storage, &feature_snapshot)?;

    // Three-way merge: baseline (base), unified (ours), feature (theirs)
    let (merged_text, conflicts) =
        crate::engine::merge::merge_texts(&baseline_text, &unified_text, &feature_text);

    let has_conflicts = !conflicts.is_empty();

    let merge_diff = diff_to_line_diff(&unified_text, &merged_text);
    if merge_diff.is_empty() {
        return Ok(MergeResult {
            snapshot_id: unified_partition.current_snapshot,
            conflicts,
        });
    }

    let feature_deltas = storage
        .get_deltas(&feature_snapshot.deltas)
        .map_err(StratumError::Storage)?;
    let merge_file = feature_deltas
        .last()
        .map(|d| d.file.clone())
        .unwrap_or_else(|| unified_snapshot.file.clone());
    let merge_delta = Delta::new(
        merge_file,
        merge_diff,
        SourceType::Manual,
    );
    storage
        .store_delta(&merge_delta)
        .map_err(StratumError::Storage)?;

    let new_snapshot = Snapshot::merge(
        vec![&unified_snapshot, &feature_snapshot],
        merge_delta.id,
        PartitionType::Unified.name(),
        has_conflicts,
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(StratumError::Storage)?;
    storage
        .update_pointer(&unified_pid, &new_snapshot.id)
        .map_err(StratumError::Storage)?;

    Ok(MergeResult {
        snapshot_id: new_snapshot.id,
        conflicts,
    })
}

/// Merge multiple Integrated feature partitions into Unified
///
/// Each feature is merged individually via `merge_feature_to_unified`.
/// Unlike the old approach, this does NOT assume all features share the same baseline.
/// Features are merged one at a time, each using its own baseline as merge base.
pub fn merge_features_to_unified<S>(storage: &S, feature_names: &[String]) -> Result<MergeResult>
where
    S: SnapshotStore + DeltaStore + FileNodeStore + PartitionStore,
{
    if feature_names.is_empty() {
        return Err(StratumError::General(
            "at least one feature required".into(),
        ));
    }

    let mut all_conflicts = Vec::new();
    let mut last_snapshot_id = None;

    for name in feature_names {
        let result = merge_feature_to_unified(storage, name)?;
        all_conflicts.extend(result.conflicts);
        last_snapshot_id = Some(result.snapshot_id);
    }

    Ok(MergeResult {
        snapshot_id: last_snapshot_id.unwrap(),
        conflicts: all_conflicts,
    })
}

/// Alias for `merge_features_to_unified` — maintained for backward compatibility
pub fn move_integrated_to_unified<S>(storage: &S, feature_names: &[String]) -> Result<SnapshotId>
where
    S: SnapshotStore + DeltaStore + FileNodeStore + PartitionStore,
{
    merge_features_to_unified(storage, feature_names).map(|r| r.snapshot_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::Delta;
    use crate::core::file_node::FileNode;
    use crate::core::types::SourceType;
    use crate::layered::integrated::integrated_partition_id;
    use crate::storage::repository::{FileNodeStore, SnapshotStore};
    use crate::storage::SqliteStorage;

    fn setup_storage() -> SqliteStorage {
        let storage = SqliteStorage::new_in_memory().unwrap();
        storage
            .with_conn(crate::storage::migrations::initialize_full)
            .unwrap();
        storage
    }

    fn create_initial_snapshot(storage: &SqliteStorage, content: &str) -> SnapshotId {
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
        parent_id: &SnapshotId,
        content: &str,
        partition_type: &str,
    ) -> SnapshotId {
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

    #[test]
    fn test_unified_partition_id() {
        let id1 = unified_partition_id();
        let id2 = unified_partition_id();
        assert_eq!(id1, id2, "unified partition ID must be deterministic");
    }

    #[test]
    fn test_ensure_unified_partition() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        let p1 = ensure_unified_partition(&storage, initial_id).unwrap();
        let p2 = ensure_unified_partition(&storage, initial_id).unwrap();
        assert_eq!(p1.id, p2.id, "unified partition should be singleton");
    }

    #[test]
    fn test_merge_feature_to_unified_no_changes() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        // Create feature partition at the same snapshot
        let feature_name = "test-feature";
        let integrated_pid = integrated_partition_id(feature_name);
        let integrated_part = Partition {
            id: integrated_pid,
            name: format!("integrated/{}", feature_name),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Integrated(feature_name.to_string()),
        };
        storage.create_partition(&integrated_part).unwrap();

        // Merging a feature that hasn't changed should be a no-op
        let result = merge_feature_to_unified(&storage, feature_name).unwrap();
        assert_eq!(result.snapshot_id, initial_id);
        assert!(!result.has_conflicts());
    }

    #[test]
    fn test_merge_feature_to_unified_with_changes() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        // Create feature partition with modified content
        let feature_name = "test-feature";
        let integrated_pid = integrated_partition_id(feature_name);
        let feature_snap_id = create_snapshot_with_content(
            &storage,
            &initial_id,
            "base\nfeature-added\n",
            "integrated/test-feature",
        );
        let integrated_part = Partition {
            id: integrated_pid,
            name: format!("integrated/{}", feature_name),
            current_snapshot: feature_snap_id,
            history: vec![initial_id, feature_snap_id],
            partition_type: PartitionType::Integrated(feature_name.to_string()),
        };
        storage.create_partition(&integrated_part).unwrap();

        let result = merge_feature_to_unified(&storage, feature_name).unwrap();
        assert!(
            result.snapshot_id != initial_id,
            "should create new snapshot when feature has changes"
        );
        assert!(!result.has_conflicts());
    }

    #[test]
    fn test_merge_features_to_unified_multiple() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\nline2\nline3\n");

        // Feature A modifies the second line
        let feat_a_snap = create_snapshot_with_content(
            &storage,
            &initial_id,
            "base\nmodified-by-A\nline3\n",
            "integrated/feat-a",
        );
        let pid_a = integrated_partition_id("feat-a");
        let part_a = Partition {
            id: pid_a,
            name: "integrated/feat-a".into(),
            current_snapshot: feat_a_snap,
            history: vec![initial_id],
            partition_type: PartitionType::Integrated("feat-a".into()),
        };
        storage.create_partition(&part_a).unwrap();

        // Feature B modifies the third line
        let feat_b_snap = create_snapshot_with_content(
            &storage,
            &initial_id,
            "base\nline2\nmodified-by-B\n",
            "integrated/feat-b",
        );
        let pid_b = integrated_partition_id("feat-b");
        let part_b = Partition {
            id: pid_b,
            name: "integrated/feat-b".into(),
            current_snapshot: feat_b_snap,
            history: vec![initial_id],
            partition_type: PartitionType::Integrated("feat-b".into()),
        };
        storage.create_partition(&part_b).unwrap();

        let result =
            merge_features_to_unified(&storage, &["feat-a".to_string(), "feat-b".to_string()])
                .unwrap();
        assert!(
            result.snapshot_id != initial_id,
            "should create new snapshot"
        );
        assert!(
            !result.has_conflicts(),
            "non-overlapping edits should not conflict"
        );
    }
}
