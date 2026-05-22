//! approval Layer Operation
//!
//! Agent changes can be migrated from an Agent Approval partition to an Integrated partition after they have been reviewed.
//! Multiple Integrated partitions can be merged into a Unified partition. Bi-directional pointer switching between partitions is also supported.

use crate::core::delta::Delta;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{
    AgentInstanceId, PartitionId, PartitionType, SnapshotId, SourceType,
};
use crate::engine::diff::diff_to_line_diff;
use crate::error::{Result, StratumError};
use crate::storage::repository::{DeltaStore, PartitionStore, SnapshotStore};
use crate::storage::sqlite_storage::SqliteStorage;

// Partition ID generation -

/// ID of the Agent partition in the approval layer
pub fn approval_agent_partition_id(agent_id: &AgentInstanceId) -> PartitionId {
    let uuid = uuid::Uuid::from_u128(0x3000_0000_0000_0000_0000_0000_0000_0000);
    let bytes = uuid.as_bytes();
    let agent_bytes = agent_id.0.as_bytes();
    let mut new_bytes = *bytes;
    for (i, b) in agent_bytes.iter().enumerate().take(16) {
        new_bytes[i] = new_bytes[i].wrapping_add(*b);
    }
    uuid::Uuid::from_bytes(new_bytes)
}

/// ID of the Integrated partition
pub fn integrated_partition_id(name: &str) -> PartitionId {
    let uuid = uuid::Uuid::from_u128(0x4000_0000_0000_0000_0000_0000_0000_0000);
    let bytes = uuid.as_bytes();
    let name_bytes = name.as_bytes();
    let mut new_bytes = *bytes;
    for (i, b) in name_bytes.iter().enumerate().take(16) {
        new_bytes[i] = new_bytes[i].wrapping_add(*b);
    }
    uuid::Uuid::from_bytes(new_bytes)
}

/// Fixed ID of the Unified partition
pub fn unified_partition_id() -> PartitionId {
    uuid::Uuid::from_u128(0x5000_0000_0000_0000_0000_0000_0000_0000)
}

// Partition creation -

/// Get or create an Agent partition at the approval level
pub fn ensure_approval_agent_partition(
    storage: &SqliteStorage,
    agent_id: &AgentInstanceId,
    initial_snapshot_id: SnapshotId,
) -> Result<Partition> {
    let pid = approval_agent_partition_id(agent_id);
    match storage.get_partition(&pid) {
        Ok(p) => Ok(p),
        Err(_) => {
            let partition = Partition {
                id: pid,
                name: format!("approval/{}", agent_id),
                current_snapshot: initial_snapshot_id,
                history: vec![initial_snapshot_id],
                partition_type: PartitionType::Approval(agent_id.clone()),
            };
            storage
                .create_partition(&partition)
                .map_err(|e| StratumError::Storage(e.into()))?;
            Ok(partition)
        }
    }
}

/// Getting or Creating Integrated Partitions
pub fn ensure_integrated_partition(
    storage: &SqliteStorage,
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
                .map_err(|e| StratumError::Storage(e.into()))?;
            Ok(partition)
        }
    }
}

/// Getting or Creating a Unified Partition
pub fn ensure_unified_partition(
    storage: &SqliteStorage,
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
                .map_err(|e| StratumError::Storage(e.into()))?;
            Ok(partition)
        }
    }
}

// -Forward migration operations -

/// Migrating the contents of an Agent Approval partition to an Integrated partition
///
/// Takes the current snapshot from the Agent partition at the approval level and merges it into the Integrated partition with the specified name.
/// If the Integrated partition does not exist, it is created automatically using the approval snapshot as the initial state.
pub fn move_approval_to_integrated(
    storage: &SqliteStorage,
    agent_id: &AgentInstanceId,
    integrated_name: &str,
) -> Result<SnapshotId> {
    let approval_pid = approval_agent_partition_id(agent_id);
    let integrated_pid = integrated_partition_id(integrated_name);

    let approval_partition = storage
        .get_partition(&approval_pid)
        .map_err(|_| StratumError::NotFound(format!("approval agent partition {} not found", agent_id)))?;
    let approval_snapshot = storage
        .get_snapshot(&approval_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    // Get or create the integrated partition using the approval snapshot as initial state
    let integrated_partition = ensure_integrated_partition(
        storage,
        integrated_name,
        approval_partition.current_snapshot,
    )?;

    // If the integrated partition was just created (same snapshot as approval), no merge needed
    if integrated_partition.current_snapshot == approval_partition.current_snapshot {
        return Ok(integrated_partition.current_snapshot);
    }

    let integrated_snapshot = storage
        .get_snapshot(&integrated_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    // incorporate: diff from integrated to approval content
    let approval_text =
        crate::state_machine::transition::reconstruct_text(storage, &approval_snapshot)?;
    let integrated_text =
        crate::state_machine::transition::reconstruct_text(storage, &integrated_snapshot)?;

    let merge_diff = diff_to_line_diff(&integrated_text, &approval_text);
    if merge_diff.is_empty() {
        return Ok(integrated_partition.current_snapshot);
    }

    let merge_delta = Delta::new(
        approval_snapshot.file.clone(),
        merge_diff,
        SourceType::Agent(agent_id.clone()),
    );
    storage
        .store_delta(&merge_delta)
        .map_err(|e| StratumError::Storage(e.into()))?;

    let new_snapshot = Snapshot::merge(
        vec![&integrated_snapshot, &approval_snapshot],
        merge_delta.id,
        PartitionType::Integrated(integrated_name.to_string()).name(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(|e| StratumError::Storage(e.into()))?;

    storage
        .update_pointer(&integrated_pid, &new_snapshot.id)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(new_snapshot.id)
}

/// Merging Multiple Integrated Partitions into a Unified Partition
///
/// Merges the contents of the specified Integrated partitions into the Unified partition.
/// If the Unified partition does not exist, it is created automatically using the first
/// Integrated partition's current snapshot as the initial state.
pub fn move_integrated_to_unified(
    storage: &SqliteStorage,
    integrated_names: &[String],
) -> Result<SnapshotId> {
    let unified_pid = unified_partition_id();

    // Determine the initial snapshot for the unified partition
    // Use the first integrated partition's snapshot if available
    let initial_snapshot = if integrated_names.is_empty() {
        // No integrated partitions to merge, just ensure unified exists
        // Use a zero-content hash as placeholder initial snapshot
        let placeholder = SnapshotId::from_content(b"unified-placeholder");
        let _ = ensure_unified_partition(storage, placeholder)?;
        let unified = storage.get_partition(&unified_pid)
            .map_err(|_| StratumError::NotFound("unified partition not found".into()))?;
        return Ok(unified.current_snapshot);
    } else {
        let first_pid = integrated_partition_id(&integrated_names[0]);
        let first_part = storage
            .get_partition(&first_pid)
            .map_err(|_| StratumError::NotFound(format!("integrated partition {} not found", integrated_names[0])))?;
        first_part.current_snapshot
    };

    // Get or create unified partition
    let unified_partition = ensure_unified_partition(storage, initial_snapshot)?;
    let unified_snapshot = storage
        .get_snapshot(&unified_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    let unified_text =
        crate::state_machine::transition::reconstruct_text(storage, &unified_snapshot)?;
    let mut merged_text = unified_text.clone();
    let mut parent_snapshots_owned: Vec<Snapshot> = Vec::new();

    for name in integrated_names {
        let pid = integrated_partition_id(name);
        let part = storage
            .get_partition(&pid)
            .map_err(|_| StratumError::NotFound(format!("integrated partition {} not found", name)))?;
        let snap = storage
            .get_snapshot(&part.current_snapshot)
            .map_err(|e| StratumError::Storage(e.into()))?;
        let text = crate::state_machine::transition::reconstruct_text(storage, &snap)?;

        // cumulative merger
        let diff = diff_to_line_diff(&merged_text, &text);
        if !diff.is_empty() {
            // Apply diff to merged_text
            let delta = Delta::new(
                snap.file.clone(),
                diff,
                SourceType::Manual,
            );
            storage
                .store_delta(&delta)
                .map_err(|e| StratumError::Storage(e.into()))?;
            // Update merged_text with apply_deltas
            merged_text = crate::engine::merge::apply_deltas(&merged_text, &[delta])
                .map_err(|e| StratumError::Engine(e.to_string()))?;
        }
        parent_snapshots_owned.push(snap);
    }

    // If there are no changes, the current unified snapshot is returned
    if parent_snapshots_owned.is_empty() {
        return Ok(unified_partition.current_snapshot);
    }

    // Constructing reference collections for merge
    let mut all_parents: Vec<&Snapshot> = vec![&unified_snapshot];
    for snap in &parent_snapshots_owned {
        all_parents.push(snap);
    }

    // Creating the final merge diff
    let final_diff = diff_to_line_diff(
        &crate::state_machine::transition::reconstruct_text(storage, &unified_snapshot)?,
        &merged_text,
    );
    let merge_delta = Delta::new(
        unified_snapshot.file.clone(),
        final_diff,
        SourceType::Manual,
    );
    storage
        .store_delta(&merge_delta)
        .map_err(|e| StratumError::Storage(e.into()))?;

    let new_snapshot = Snapshot::merge(
        all_parents,
        merge_delta.id,
        PartitionType::Unified.name(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(|e| StratumError::Storage(e.into()))?;

    storage
        .update_pointer(&unified_pid, &new_snapshot.id)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(new_snapshot.id)
}

/// AGENT_RAW ↔ INTEGRATED ↔ UNIFIED Bidirectional migration (switching pointers only)
///
/// Copies the current_snapshot pointer from the from partition to the to partition.
pub fn migrate_between_partitions(
    storage: &SqliteStorage,
    from_partition_id: &PartitionId,
    to_partition_id: &PartitionId,
) -> Result<()> {
    let from_partition = storage
        .get_partition(from_partition_id)
        .map_err(|_| StratumError::NotFound("source partition not found".into()))?;

    storage
        .update_pointer(to_partition_id, &from_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::Delta;
    use crate::core::file_node::FileNode;
    use crate::core::types::SourceType;
    use crate::engine::diff::diff_to_line_diff;
    use crate::storage::repository::{FileNodeStore, SnapshotStore};
    use crate::storage::sqlite_storage::SqliteStorage;
    use crate::state_machine::transition::reconstruct_text;
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

    fn create_snapshot_with_content(
        storage: &SqliteStorage,
        parent_id: &SnapshotId,
        content: &str,
        partition_type: &str,
    ) -> SnapshotId {
        let parent = storage.get_snapshot(parent_id).unwrap();
        let file_node = FileNode::new(std::path::PathBuf::from("test.txt"), content.as_bytes());
        storage.store_file_node(&file_node, content.as_bytes()).unwrap();

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
        let approval_pid = approval_agent_partition_id(&agent_id);
        let integrated_name = "integrated-1";
        let integrated_pid = integrated_partition_id(integrated_name);

        // Creating Partitions
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

        // Creating a new snapshot for approval
        let new_snap = {
            let snap = storage.get_snapshot(&initial_id).unwrap();
            let s = Snapshot::from_parent(
                &snap,
                Delta::new(
                    FileNode::new(std::path::PathBuf::from("test.txt"), b"base\n"),
                    crate::core::delta::LineDiff::new(vec![]),
                    SourceType::Manual,
                )
                .id,
                PartitionType::Approval(agent_id.clone()).name(),
            );
            storage.store_snapshot(&s, b"").unwrap();
            s
        };
        storage.update_pointer(&approval_pid, &new_snap.id).unwrap();

        // migration pointer
        migrate_between_partitions(&storage, &approval_pid, &integrated_pid).unwrap();
        let integrated = storage.get_partition(&integrated_pid).unwrap();
        assert_eq!(integrated.current_snapshot, new_snap.id);
    }

    #[test]
    fn test_ensure_approval_agent_partition() {
        let storage = setup_storage();
        let agent_id = AgentInstanceId("test-agent".into());
        let initial_id = create_initial_snapshot(&storage, "base\n");

        let p1 = ensure_approval_agent_partition(&storage, &agent_id, initial_id).unwrap();
        let p2 = ensure_approval_agent_partition(&storage, &agent_id, initial_id).unwrap();
        assert_eq!(p1.id, p2.id, "second call should return existing partition");
    }

    #[test]
    fn test_ensure_integrated_partition() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        let p1 = ensure_integrated_partition(&storage, "feat-1", initial_id).unwrap();
        let p2 = ensure_integrated_partition(&storage, "feat-1", initial_id).unwrap();
        assert_eq!(p1.id, p2.id);

        let p3 = ensure_integrated_partition(&storage, "feat-2", initial_id).unwrap();
        assert_ne!(p1.id, p3.id, "different names should produce different partition ids");
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

        // Create approval partition
        let approval_pid = approval_agent_partition_id(&agent_id);
        let approval_part = Partition {
            id: approval_pid,
            name: format!("approval/{}", agent_id),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Approval(agent_id.clone()),
        };
        storage.create_partition(&approval_part).unwrap();

        // Create integrated partition
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

        // Advance approval with modified content
        let approval_new_id = create_snapshot_with_content(
            &storage,
            &initial_id,
            "base\nmodified\n",
            "approval/test-agent",
        );
        storage.update_pointer(&approval_pid, &approval_new_id).unwrap();

        // Move approval to integrated
        let result = move_approval_to_integrated(&storage, &agent_id, integrated_name);
        assert!(result.is_ok());

        let integrated = storage.get_partition(&integrated_pid).unwrap();
        assert_ne!(integrated.current_snapshot, initial_id);
    }

    #[test]
    fn test_move_integrated_to_unified() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        // Create unified partition
        ensure_unified_partition(&storage, initial_id).unwrap();

        // Create two integrated partitions
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

        // Move integrated to unified
        let result = move_integrated_to_unified(&storage, &integrated_names);
        assert!(result.is_ok());

        let unified = storage.get_partition(&unified_partition_id()).unwrap();
        assert_ne!(unified.current_snapshot, initial_id);
    }

    #[test]
    fn test_partition_id_uniqueness() {
        let agent_a = AgentInstanceId("agent-a".into());
        let agent_b = AgentInstanceId("agent-b".into());

        let aa = approval_agent_partition_id(&agent_a);
        let ab = approval_agent_partition_id(&agent_b);
        assert_ne!(aa, ab, "different agents should have different approval partition ids");

        let ia = integrated_partition_id("feat-a");
        let ib = integrated_partition_id("feat-b");
        assert_ne!(ia, ib, "different integrations should have different partition ids");

        // Different partition types should also differ
        assert_ne!(aa, ia, "approval and integrated partition ids should differ");
    }
}
