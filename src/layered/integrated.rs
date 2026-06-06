//! integrated / unified layer operations
//!
//! Manages the Integrated (named) and Unified partitions within the approval layer.
//! Agent changes flow approval_agent → integrated → unified, eventually merging into staged.

use crate::core::delta::Delta;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{AgentInstanceId, PartitionId, PartitionType, SnapshotId, SourceType};
use crate::engine::diff::diff_to_line_diff;
use crate::error::{Result, StratumError};
use crate::storage::repository::{DeltaStore, FileNodeStore, PartitionStore, SnapshotStore};

pub use crate::engine::merge::MergeConflict;

/// Result of merging multiple features into unified
#[derive(Debug, Clone)]
pub struct UnifiedMergeResult {
    pub snapshot_id: SnapshotId,
    pub conflicts: Vec<crate::engine::merge::MergeConflict>,
}

impl UnifiedMergeResult {
    /// Check if merge has conflicts
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }

    /// Get conflict count
    pub fn conflict_count(&self) -> usize {
        self.conflicts.len()
    }

    /// Format all conflicts as Git-style markers
    pub fn format_conflicts(&self) -> String {
        if self.conflicts.is_empty() {
            return String::new();
        }
        let mut result = String::new();
        for (i, conflict) in self.conflicts.iter().enumerate() {
            result.push_str(&format!("Conflict #{} (line {}):\n", i + 1, conflict.start_line));
            result.push_str(&conflict.to_conflict_marker());
            result.push('\n');
        }
        result
    }
}

// Partition ID generation -

/// ID of the Integrated partition for the given name via UUIDv5
pub fn integrated_partition_id(name: &str) -> PartitionId {
    let namespace = uuid::Uuid::from_u128(0x4000_0000_0000_0000_0000_0000_0000_0000);
    uuid::Uuid::new_v5(&namespace, name.as_bytes())
}

/// Fixed ID of the Unified partition
pub fn unified_partition_id() -> PartitionId {
    uuid::Uuid::from_u128(0x5000_0000_0000_0000_0000_0000_0000_0000)
}

// Partition creation -

/// Get or create an Integrated partition
pub fn ensure_integrated_partition<S: PartitionStore>(
    storage: &S,
    name: &str,
    initial_snapshot_id: SnapshotId,
) -> Result<Partition> {
    let pid = integrated_partition_id(name);
    match storage.get_partition(&pid) {
        Ok(p) => Ok(p),
        Err(_) => {
            let partition = Partition {
                id: pid,
                name: format!("integrated/{}", name),
                current_snapshot: initial_snapshot_id,
                history: vec![initial_snapshot_id],
                partition_type: PartitionType::Integrated(name.to_string()),
            };
            storage
                .create_partition(&partition)
                .map_err(StratumError::Storage)?;
            Ok(partition)
        }
    }
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

/// Create a new feature branch with explicit baseline
/// This makes the baseline concept clear for feature development
pub fn create_feature_branch<S: PartitionStore>(
    storage: &S,
    name: &str,
    baseline_snapshot_id: SnapshotId,
) -> Result<Partition> {
    let pid = integrated_partition_id(name);
    let partition = Partition {
        id: pid,
        name: format!("feature/{}", name),
        current_snapshot: baseline_snapshot_id,
        history: vec![baseline_snapshot_id],
        partition_type: PartitionType::Integrated(name.to_string()),
    };
    storage
        .create_partition(&partition)
        .map_err(StratumError::Storage)?;
    Ok(partition)
}

/// Get the baseline snapshot for a feature branch
/// The baseline is the first snapshot in the feature's history
pub fn get_feature_baseline<S: SnapshotStore + PartitionStore>(
    storage: &S,
    feature_name: &str,
) -> Result<Snapshot> {
    let pid = integrated_partition_id(feature_name);
    let part = storage.get_partition(&pid).map_err(|_| {
        StratumError::NotFound(format!("integrated partition {} not found", feature_name))
    })?;
    let baseline_id = &part.history[0];
    storage.get_snapshot(baseline_id).map_err(StratumError::Storage)
}

// Forward migration operations -

/// Result of merging an agent into a feature
#[derive(Debug, Clone)]
pub struct FeatureMergeResult {
    pub snapshot_id: SnapshotId,
    pub conflicts: Vec<crate::engine::merge::MergeConflict>,
}

impl FeatureMergeResult {
    /// Check if merge has conflicts
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }

    /// Get conflict count
    pub fn conflict_count(&self) -> usize {
        self.conflicts.len()
    }

    /// Format all conflicts as Git-style markers
    pub fn format_conflicts(&self) -> String {
        if self.conflicts.is_empty() {
            return String::new();
        }
        let mut result = String::new();
        for (i, conflict) in self.conflicts.iter().enumerate() {
            result.push_str(&format!("Conflict #{} (line {}):\n", i + 1, conflict.start_line));
            result.push_str(&conflict.to_conflict_marker());
            result.push('\n');
        }
        result
    }
}

/// Merge an Agent's approval into a feature branch using three-way merge
/// This supports multiple agents collaborating on the same feature
pub fn merge_agent_to_feature<S>(
    storage: &S,
    agent_id: &AgentInstanceId,
    feature_name: &str,
) -> Result<FeatureMergeResult>
where
    S: SnapshotStore + DeltaStore + FileNodeStore + PartitionStore,
{
    let approval_pid = crate::layered::approval::approval_agent_partition_id(agent_id);
    let integrated_pid = integrated_partition_id(feature_name);

    let approval_partition = storage.get_partition(&approval_pid).map_err(|_| {
        StratumError::NotFound(format!("approval agent partition {} not found", agent_id))
    })?;
    let approval_snapshot = storage
        .get_snapshot(&approval_partition.current_snapshot)
        .map_err(StratumError::Storage)?;

    let integrated_partition = ensure_integrated_partition(
        storage,
        feature_name,
        approval_partition.current_snapshot,
    )?;

    if integrated_partition.current_snapshot == approval_partition.current_snapshot {
        return Ok(FeatureMergeResult {
            snapshot_id: integrated_partition.current_snapshot,
            conflicts: vec![],
        });
    }

    let baseline_snapshot = get_feature_baseline(storage, feature_name)?;
    let integrated_snapshot = storage
        .get_snapshot(&integrated_partition.current_snapshot)
        .map_err(StratumError::Storage)?;

    let baseline_text = crate::layered::transition::reconstruct_text(storage, &baseline_snapshot)?;
    let approval_text = crate::layered::transition::reconstruct_text(storage, &approval_snapshot)?;
    let integrated_text =
        crate::layered::transition::reconstruct_text(storage, &integrated_snapshot)?;

    let (merged_text, conflicts) = crate::engine::merge::merge_texts(
        &baseline_text,
        &approval_text,
        &integrated_text,
    );

    let merge_diff = diff_to_line_diff(&baseline_text, &merged_text);
    if merge_diff.is_empty() {
        return Ok(FeatureMergeResult {
            snapshot_id: integrated_partition.current_snapshot,
            conflicts,
        });
    }

    let merge_delta = Delta::new(
        baseline_snapshot.file.clone(),
        merge_diff,
        SourceType::Agent(agent_id.clone()),
    );
    storage
        .store_delta(&merge_delta)
        .map_err(StratumError::Storage)?;

    let new_snapshot = Snapshot::merge(
        vec![&baseline_snapshot, &approval_snapshot, &integrated_snapshot],
        merge_delta.id,
        PartitionType::Integrated(feature_name.to_string()).name(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(StratumError::Storage)?;

    storage
        .update_pointer(&integrated_pid, &new_snapshot.id)
        .map_err(StratumError::Storage)?;

    Ok(FeatureMergeResult {
        snapshot_id: new_snapshot.id,
        conflicts,
    })
}

/// Migrate Agent approval content into an Integrated partition
/// This is a simplified version that uses sequential merge (legacy, kept for backward compatibility)
/// For new code, use merge_agent_to_feature instead
pub fn move_approval_to_integrated<S>(
    storage: &S,
    agent_id: &AgentInstanceId,
    integrated_name: &str,
) -> Result<SnapshotId>
where
    S: SnapshotStore + DeltaStore + FileNodeStore + PartitionStore,
{
    let result = merge_agent_to_feature(storage, agent_id, integrated_name)?;
    Ok(result.snapshot_id)
}

/// Get the common baseline for multiple features
fn get_common_baseline<S: SnapshotStore + PartitionStore>(
    storage: &S,
    feature_names: &[String],
) -> Result<Snapshot> {
    if feature_names.is_empty() {
        return Err(StratumError::General("至少需要一个feature".into()));
    }
    
    let first_pid = integrated_partition_id(&feature_names[0]);
    let first_part = storage.get_partition(&first_pid).map_err(|_| {
        StratumError::NotFound(format!(
            "integrated partition {} not found",
            feature_names[0]
        ))
    })?;
    let baseline_id = &first_part.history[0];
    storage.get_snapshot(baseline_id).map_err(StratumError::Storage)
}

/// Merge all named Integrated partitions into the Unified partition
/// Uses three-way merge based on the common baseline instead of sequential merge
pub fn move_integrated_to_unified<S>(
    storage: &S,
    integrated_names: &[String],
) -> Result<SnapshotId>
where
    S: SnapshotStore + DeltaStore + FileNodeStore + PartitionStore,
{
    let result = merge_features_to_unified(storage, integrated_names)?;
    Ok(result.snapshot_id)
}

/// Merge multiple feature branches into unified
/// Uses three-way merge engine, all features merge based on the original baseline
pub fn merge_features_to_unified<S>(
    storage: &S,
    feature_names: &[String],
) -> Result<UnifiedMergeResult>
where
    S: SnapshotStore + DeltaStore + FileNodeStore + PartitionStore,
{
    if feature_names.is_empty() {
        return Err(StratumError::General("至少需要一个feature".into()));
    }

    let unified_pid = unified_partition_id();

    let common_baseline = get_common_baseline(storage, feature_names)?;

    let unified_partition = ensure_unified_partition(storage, common_baseline.id)?;

    let baseline_text = crate::layered::transition::reconstruct_text(storage, &common_baseline)?;
    let mut current_text = baseline_text.clone();
    let mut all_conflicts: Vec<crate::engine::merge::MergeConflict> = Vec::new();
    let mut all_parents: Vec<Snapshot> = vec![common_baseline.clone()];

    for name in feature_names {
        let pid = integrated_partition_id(name);
        let part = storage.get_partition(&pid).map_err(|_| {
            StratumError::NotFound(format!("integrated partition {} not found", name))
        })?;
        let snap = storage
            .get_snapshot(&part.current_snapshot)
            .map_err(StratumError::Storage)?;
        let feature_text = crate::layered::transition::reconstruct_text(storage, &snap)?;

        let (merged_text, conflicts) = crate::engine::merge::merge_texts(
            &baseline_text,
            &current_text,
            &feature_text,
        );

        current_text = merged_text;
        all_conflicts.extend(conflicts);
        all_parents.push(snap);
    }

    let unified_snapshot = storage
        .get_snapshot(&unified_partition.current_snapshot)
        .map_err(StratumError::Storage)?;

    let final_diff = diff_to_line_diff(
        &crate::layered::transition::reconstruct_text(storage, &unified_snapshot)?,
        &current_text,
    );
    
    if final_diff.is_empty() {
        return Ok(UnifiedMergeResult {
            snapshot_id: unified_snapshot.id,
            conflicts: all_conflicts,
        });
    }

    let merge_delta = Delta::new(
        unified_snapshot.file.clone(),
        final_diff,
        SourceType::Manual,
    );
    storage
        .store_delta(&merge_delta)
        .map_err(StratumError::Storage)?;

    let mut parent_refs: Vec<&Snapshot> = all_parents.iter().collect();
    parent_refs.insert(0, &unified_snapshot);

    let new_snapshot = Snapshot::merge(parent_refs, merge_delta.id, PartitionType::Unified.name());
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(StratumError::Storage)?;

    storage
        .update_pointer(&unified_pid, &new_snapshot.id)
        .map_err(StratumError::Storage)?;

    Ok(UnifiedMergeResult {
        snapshot_id: new_snapshot.id,
        conflicts: all_conflicts,
    })
}

/// Copy current_snapshot pointer from one partition to another (pointer-only, no data writes)
pub fn migrate_between_partitions<S: PartitionStore>(
    storage: &S,
    from_partition_id: &PartitionId,
    to_partition_id: &PartitionId,
) -> Result<()> {
    let from_partition = storage
        .get_partition(from_partition_id)
        .map_err(|_| StratumError::NotFound("source partition not found".into()))?;

    storage
        .update_pointer(to_partition_id, &from_partition.current_snapshot)
        .map_err(StratumError::Storage)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::Delta;
    use crate::core::file_node::FileNode;
    use crate::core::types::SourceType;
    use crate::engine::diff::diff_to_line_diff;
    use crate::layered::transition::reconstruct_text;
    use crate::storage::repository::{FileNodeStore, SnapshotStore};
    use crate::storage::SqliteStorage;

    fn setup_storage() -> SqliteStorage {
        SqliteStorage::new_in_memory().unwrap()
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

        let parent_text = reconstruct_text(storage, &parent).unwrap();
        let diff = diff_to_line_diff(&parent_text, content);
        let delta = Delta::new(file_node, diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();

        let snap = Snapshot::from_parent(&parent, delta.id, partition_type.to_string());
        storage.store_snapshot(&snap, b"").unwrap();
        snap.id
    }

    #[test]
    fn test_migrate_between_partitions() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        let agent_id = AgentInstanceId("test-agent".into());
        let approval_pid = crate::layered::approval::approval_agent_partition_id(&agent_id);
        let integrated_name = "integrated-1";
        let integrated_pid = integrated_partition_id(integrated_name);

        let approval_part = Partition {
            id: approval_pid,
            name: "approval/test-agent".into(),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Approval(agent_id.clone()),
        };
        storage.create_partition(&approval_part).unwrap();

        let integrated_part = Partition {
            id: integrated_pid,
            name: format!("integrated/{}", integrated_name),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Integrated(integrated_name.to_string()),
        };
        storage.create_partition(&integrated_part).unwrap();

        let new_snap = {
            let snap = storage.get_snapshot(&initial_id).unwrap();
            let s = Snapshot::from_parent(
                &snap,
                Delta::new(
                    FileNode::new(std::path::PathBuf::from("test.txt"), b"base\n"),
                    crate::core::types::LineDiff::new(vec![]),
                    SourceType::Manual,
                )
                .id,
                PartitionType::Approval(agent_id.clone()).name(),
            );
            storage.store_snapshot(&s, b"").unwrap();
            s
        };
        storage.update_pointer(&approval_pid, &new_snap.id).unwrap();

        migrate_between_partitions(&storage, &approval_pid, &integrated_pid).unwrap();
        let integrated = storage.get_partition(&integrated_pid).unwrap();
        assert_eq!(integrated.current_snapshot, new_snap.id);
    }

    #[test]
    fn test_ensure_integrated_partition() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        let p1 = ensure_integrated_partition(&storage, "feat-1", initial_id).unwrap();
        let p2 = ensure_integrated_partition(&storage, "feat-1", initial_id).unwrap();
        assert_eq!(p1.id, p2.id);

        let p3 = ensure_integrated_partition(&storage, "feat-2", initial_id).unwrap();
        assert_ne!(
            p1.id, p3.id,
            "different names should produce different partition ids"
        );
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
    fn test_move_approval_to_integrated() {
        let storage = setup_storage();
        let agent_id = AgentInstanceId("test-agent".into());
        let initial_id = create_initial_snapshot(&storage, "base\n");

        let approval_pid = crate::layered::approval::approval_agent_partition_id(&agent_id);
        let approval_part = Partition {
            id: approval_pid,
            name: format!("approval/{}", agent_id),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Approval(agent_id.clone()),
        };
        storage.create_partition(&approval_part).unwrap();

        let integrated_name = "feat-integrated";
        let integrated_pid = integrated_partition_id(integrated_name);
        let integrated_part = Partition {
            id: integrated_pid,
            name: format!("integrated/{}", integrated_name),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Integrated(integrated_name.to_string()),
        };
        storage.create_partition(&integrated_part).unwrap();

        let approval_new_id = create_snapshot_with_content(
            &storage,
            &initial_id,
            "base\nmodified\n",
            "approval/test-agent",
        );
        storage
            .update_pointer(&approval_pid, &approval_new_id)
            .unwrap();

        let result = move_approval_to_integrated(&storage, &agent_id, integrated_name);
        assert!(result.is_ok());

        let integrated = storage.get_partition(&integrated_pid).unwrap();
        assert_ne!(integrated.current_snapshot, initial_id);
    }

    #[test]
    fn test_move_integrated_to_unified() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        ensure_unified_partition(&storage, initial_id).unwrap();

        let integrated_names = vec!["int-a".to_string(), "int-b".to_string()];

        for name in &integrated_names {
            let pid = integrated_partition_id(name);
            let part = Partition {
                id: pid,
                name: format!("integrated/{}", name),
                current_snapshot: initial_id,
                history: vec![initial_id],
                partition_type: PartitionType::Integrated(name.clone()),
            };
            storage.create_partition(&part).unwrap();

            let new_id = create_snapshot_with_content(
                &storage,
                &initial_id,
                &format!("base\nfrom-{}\n", name),
                &format!("integrated/{}", name),
            );
            storage.update_pointer(&pid, &new_id).unwrap();
        }

        let result = move_integrated_to_unified(&storage, &integrated_names);
        assert!(result.is_ok());

        let unified = storage.get_partition(&unified_partition_id()).unwrap();
        assert_ne!(unified.current_snapshot, initial_id);
    }

    #[test]
    fn test_partition_id_uniqueness() {
        let agent_a = AgentInstanceId("agent-a".into());
        let agent_b = AgentInstanceId("agent-b".into());

        let aa = crate::layered::approval::approval_agent_partition_id(&agent_a);
        let ab = crate::layered::approval::approval_agent_partition_id(&agent_b);
        assert_ne!(
            aa, ab,
            "different agents should have different approval partition ids"
        );

        let ia = integrated_partition_id("feat-a");
        let ib = integrated_partition_id("feat-b");
        assert_ne!(
            ia, ib,
            "different integrations should have different partition ids"
        );

        assert_ne!(
            aa, ia,
            "approval and integrated partition ids should differ"
        );
    }
}
