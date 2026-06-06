//! approval Layer Operation
//!
//! Manages the Agent-level approval partitions.
//! Agent changes flow: agent_edit → approval_agent → integrated (see integrated.rs).

use crate::core::partition::Partition;
use crate::core::types::{AgentInstanceId, PartitionId, PartitionType, SnapshotId};
use crate::error::{Result, StratumError};
use crate::storage::repository::PartitionStore;

/// ID of the Agent partition in the approval layer via UUIDv5
pub fn approval_agent_partition_id(agent_id: &AgentInstanceId) -> PartitionId {
    let namespace = uuid::Uuid::from_u128(0x3000_0000_0000_0000_0000_0000_0000_0000);
    uuid::Uuid::new_v5(&namespace, agent_id.0.as_bytes())
}

/// Get or create an Agent partition at the approval level
pub fn ensure_approval_agent_partition<S: PartitionStore>(
    storage: &S,
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
                .map_err(StratumError::Storage)?;
            Ok(partition)
        }
    }
}

/// List all approval-type partitions (regardless of status)
pub fn list_approval_partitions<S: PartitionStore>(storage: &S) -> Result<Vec<Partition>> {
    let all = storage.list_partitions().map_err(StratumError::Storage)?;
    Ok(all
        .into_iter()
        .filter(|p| matches!(p.partition_type, PartitionType::Approval(_)))
        .collect())
}

/// List pending approval partitions — those that have more than 1 history entry
/// (indicating the agent has submitted changes that haven't been processed yet).
///
/// A "pending" approval partition has been updated by `move_agent_to_approval`
/// but not yet approved (merged into integrated) or rejected (rolled back).
pub fn list_pending_approvals<S: PartitionStore>(storage: &S) -> Result<Vec<Partition>> {
    let all = list_approval_partitions(storage)?;
    Ok(all
        .into_iter()
        .filter(|p| p.history.len() > 1)
        .collect())
}

/// Reject an agent's approval submission by rolling back to the baseline snapshot.
///
/// This undoes the agent's contribution by restoring the approval partition pointer
/// to the first snapshot in its history (the base state before agent edits were merged).
pub fn reject_approval<S: PartitionStore>(storage: &S, agent_id: &AgentInstanceId) -> Result<SnapshotId> {
    let pid = approval_agent_partition_id(agent_id);
    let partition = storage
        .get_partition(&pid)
        .map_err(|_| StratumError::NotFound(format!("approval partition for agent '{}' not found", agent_id)))?;

    let base_snapshot = partition.history.first().ok_or_else(|| {
        StratumError::StateMachine("approval partition has empty history".into())
    })?;

    storage
        .update_pointer(&pid, base_snapshot)
        .map_err(StratumError::Storage)?;

    Ok(*base_snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::Delta;
    use crate::core::file_node::FileNode;
    use crate::core::snapshot::Snapshot;
    use crate::core::types::SourceType;
    use crate::storage::repository::{DeltaStore, FileNodeStore, SnapshotStore};
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
    fn test_approval_agent_partition_id() {
        let agent_a = AgentInstanceId("agent-a".into());
        let agent_b = AgentInstanceId("agent-b".into());

        let aa = approval_agent_partition_id(&agent_a);
        let ab = approval_agent_partition_id(&agent_b);
        assert_ne!(
            aa, ab,
            "different agents should have different approval partition ids"
        );
    }

    #[test]
    fn test_list_approval_partitions() {
        let storage = setup_storage();
        let base_id = create_initial_snapshot(&storage, "base\n");

        let agent_a = AgentInstanceId("agent-a".into());
        let agent_b = AgentInstanceId("agent-b".into());

        ensure_approval_agent_partition(&storage, &agent_a, base_id).unwrap();
        ensure_approval_agent_partition(&storage, &agent_b, base_id).unwrap();

        let list = list_approval_partitions(&storage).unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_list_pending_approvals_empty() {
        let storage = setup_storage();
        let base_id = create_initial_snapshot(&storage, "base\n");

        let agent = AgentInstanceId("agent".into());
        ensure_approval_agent_partition(&storage, &agent, base_id).unwrap();

        // Partition has 1 history entry → not pending
        let pending = list_pending_approvals(&storage).unwrap();
        assert!(pending.is_empty());
    }

    #[test]
    fn test_reject_approval() {
        let storage = setup_storage();
        let base_id = create_initial_snapshot(&storage, "base\n");

        let agent = AgentInstanceId("reject-agent".into());

        // Create approval partition with base_id
        let mut p = ensure_approval_agent_partition(&storage, &agent, base_id).unwrap();

        // Simulate move_agent_to_approval by advancing the pointer
        let merged_id = create_initial_snapshot(&storage, "agent changes\n");
        p.advance(merged_id);
        storage
            .update_pointer(&p.id, &merged_id)
            .map_err(|e| StratumError::Storage(e))
            .unwrap();

        // Verify partition now has 2 history entries
        let before = storage.get_partition(&p.id).unwrap();
        assert_eq!(before.history.len(), 2);

        // Reject
        let rolled_back = reject_approval(&storage, &agent).unwrap();
        assert_eq!(rolled_back, base_id);

        // Verify pointer is back to base
        let after = storage.get_partition(&p.id).unwrap();
        assert_eq!(after.current_snapshot, base_id);
    }

    #[test]
    fn test_reject_approval_nonexistent_agent() {
        let storage = setup_storage();
        let agent = AgentInstanceId("ghost".into());
        let result = reject_approval(&storage, &agent);
        assert!(result.is_err());
    }
}