//! approval Layer Operation
//!
//! Manages the Agent-level approval partitions.
//! Agent changes flow: agent_edit → approval_agent → integrated (see integrated.rs).

use crate::core::partition::Partition;
use crate::core::types::{
    AgentInstanceId, PartitionId, PartitionType, SnapshotId,
};
use crate::error::{Result, StratumError};
use crate::storage::repository::PartitionStore;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::file_node::FileNode;
    use crate::core::delta::Delta;
    use crate::core::types::SourceType;
    use crate::core::snapshot::Snapshot;
    use crate::storage::repository::{SnapshotStore, FileNodeStore, DeltaStore};
    use crate::storage::sqlite_storage::SqliteStorage;

    fn setup_storage() -> SqliteStorage {
        SqliteStorage::new_in_memory().unwrap()
    }

    fn create_initial_snapshot(storage: &SqliteStorage, content: &str) -> SnapshotId {
        let file_node = FileNode::new(std::path::PathBuf::from("test.txt"), content.as_bytes());
        storage.store_file_node(&file_node, content.as_bytes()).unwrap();
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
        assert_ne!(aa, ab, "different agents should have different approval partition ids");
    }
}