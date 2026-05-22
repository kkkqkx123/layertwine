//! Hierarchical State Machine Module (Phase 3)
//!
//! Management manual_edit → agent_edit → approval → staged Four levels of pure state machine complete
//! Forward flow and reverse fallback to ensure hierarchical segregation and the principle of immutability.

pub mod manual;
pub mod agent;
pub mod approval;
pub mod staged;
pub mod transition;

use crate::core::layer::Layer;
use crate::core::partition::Partition;
use crate::core::types::{CheckpointId, LayerType, PartitionId, PartitionType, SnapshotId};
use crate::error::{Result, StratumError};
use crate::storage::repository::{BranchStore, CheckpointStore, PartitionStore};
use crate::storage::sqlite_storage::SqliteStorage;
use std::sync::Arc;

/// Hierarchical State Machine - Unified Operations Portal
///
/// Holds storage tier references and provides partition access and state flow interfaces for each tier.
pub struct StateMachine {
    storage: Arc<SqliteStorage>,
}

impl StateMachine {
    /// Creating a new state machine instance
    pub fn new(storage: Arc<SqliteStorage>) -> Self {
        StateMachine { storage }
    }

    /// Getting Storage Layer References
    pub fn storage(&self) -> &SqliteStorage {
        &self.storage
    }

    // Partition access methods -

    /// Get the specified partition of the specified layer (read-only)
    pub fn get_partition(&self, _layer: &LayerType, partition_id: &PartitionId) -> Result<Partition> {
        self.storage
            .get_partition(partition_id)
            .map_err(|e| StratumError::Storage(e.into()))
    }

    /// Get the specified partition of the specified layer (variable)
    pub fn get_partition_mut(&self, _layer: &LayerType, partition_id: &PartitionId) -> Result<Partition> {
        self.storage
            .get_partition(partition_id)
            .map_err(|e| StratumError::Storage(e.into()))
    }

    /// Getting or creating partitions
    pub fn get_or_create_partition(
        &self,
        _layer: &LayerType,
        partition_id: &PartitionId,
        _name: &str,
        partition: &Partition,
    ) -> Result<Partition> {
        // First try to get
        match self.storage.get_partition(partition_id) {
            Ok(p) => Ok(p),
            Err(_) => {
                self.storage
                    .create_partition(partition)
                    .map_err(|e| StratumError::Storage(e.into()))?;
                Ok(partition.clone())
            }
        }
    }

    /// Updating the partition pointer
    pub fn update_partition_pointer(
        &self,
        partition_id: &PartitionId,
        snapshot_id: &SnapshotId,
    ) -> Result<()> {
        self.storage
            .update_pointer(partition_id, snapshot_id)
            .map_err(|e| StratumError::Storage(e.into()))
    }

    // Layer management -

    /// Creating a Default Layer
    pub fn create_layer(&self, layer_type: &LayerType) -> Layer {
        Layer::new(layer_type.clone())
    }

    /// Switch branches and synchronize the layer status
    ///
    /// 1. Get the head checkpoint for the target branch
    /// 2. Reset the staged partition to the first snapshot in the checkpoint
    /// 3. Clear other layer states (approval, agent_edit) if needed
    pub fn switch_branch(&self, branch_name: &str) -> Result<CheckpointId> {
        let branch = self
            .storage
            .get_branch(branch_name)
            .map_err(|e| StratumError::Storage(e.into()))?;
        let head_cp = self
            .storage
            .get_checkpoint(&branch.head)
            .map_err(|e| StratumError::Storage(e.into()))?;
        if head_cp.baseline_snapshots.is_empty() {
             return Err(StratumError::Checkpoint(
                 "branch head checkpoint has no snapshots".into(),
             ));
         }
         let base_snapshot = head_cp.baseline_snapshots[0];

         // Reset staged partition to the branch's base snapshot
         let staged_pid = crate::state_machine::staged::staged_partition_id();
         match self.storage.get_partition(&staged_pid) {
             Ok(_) => {
                 self.storage
                     .update_pointer(&staged_pid, &base_snapshot)
                     .map_err(|e| StratumError::Storage(e.into()))?;
             }
             Err(_) => {
                 let partition = Partition::new(
                     "staged".to_string(),
                     PartitionType::Staged,
                     base_snapshot,
                 );
                 self.storage
                     .create_partition(&partition)
                     .map_err(|e| StratumError::Storage(e.into()))?;
             }
         }

         Ok(branch.head)
    }

    // Transaction support -

    /// enforcement service
    pub fn with_transaction<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&SqliteStorage) -> Result<T>,
    {
        self.storage
            .with_conn(|conn| {
                conn.execute_batch("BEGIN TRANSACTION;")
                    .map_err(|e| crate::StorageError::Database(e))?;
                // Within a transaction: f uses the storage method (locks itself), so it needs to release the lock first.
                // Transactions are managed internally by SQLite and do not rely on Rust Mutex to maintain the
                Ok(())
            })?;

        let result = f(&self.storage);

        // Commit or Rollback
        self.storage
            .with_conn(|conn| {
                match &result {
                    Ok(_) => conn
                        .execute_batch("COMMIT;")
                        .map_err(crate::StorageError::Database),
                    Err(_) => conn
                        .execute_batch("ROLLBACK;")
                        .map_err(crate::StorageError::Database),
                }
            })?;

        result
    }

    }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::Delta;
    use crate::core::file_node::FileNode;
    use crate::core::snapshot::Snapshot;
    use crate::core::types::SourceType;
    use crate::storage::repository::{FileNodeStore, SnapshotStore, DeltaStore, PartitionStore};
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

    #[test]
    fn test_state_machine_new() {
        let storage = setup_storage();
        let sm = StateMachine::new(storage.clone());
        let _ = sm.get_partition(&LayerType::ManualEdit, &uuid::Uuid::new_v4());
        // Should not panic - method returns error for non-existent partition
    }

    #[test]
    fn test_state_machine_create_layer() {
        let storage = setup_storage();
        let sm = StateMachine::new(storage);

        let manual_layer = sm.create_layer(&LayerType::ManualEdit);
        assert_eq!(manual_layer.layer_type, LayerType::ManualEdit);
        assert!(manual_layer.partitions.is_empty());

        let agent_layer = sm.create_layer(&LayerType::AgentEdit);
        assert_eq!(agent_layer.layer_type, LayerType::AgentEdit);

        let approval_layer = sm.create_layer(&LayerType::Approval);
        assert_eq!(approval_layer.layer_type, LayerType::Approval);

        let staged_layer = sm.create_layer(&LayerType::Staged);
        assert_eq!(staged_layer.layer_type, LayerType::Staged);
    }

    #[test]
    fn test_state_machine_update_partition_pointer() {
        let storage = setup_storage();
        let sm = StateMachine::new(storage.clone());

        let initial_id = create_initial_snapshot(&storage, "base\n");
        let pid = uuid::Uuid::new_v4();
        let partition = Partition {
            id: pid,
            name: "test".to_string(),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Manual,
        };
        storage.create_partition(&partition).unwrap();

        // Create a new snapshot
        let file_node = FileNode::new(std::path::PathBuf::from("test.txt"), b"base\nmodified\n");
        storage.store_file_node(&file_node, b"base\nmodified\n").unwrap();
        let diff = crate::engine::diff::diff_to_line_diff("base\n", "base\nmodified\n");
        let delta = Delta::new(file_node, diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        let snap = storage.get_snapshot(&initial_id).unwrap();
        let new_snap = Snapshot::from_parent(&snap, delta.id, "manual".to_string());
        storage.store_snapshot(&new_snap, b"").unwrap();

        // Update pointer
        let result = sm.update_partition_pointer(&pid, &new_snap.id);
        assert!(result.is_ok());

        let updated = storage.get_partition(&pid).unwrap();
        assert_eq!(updated.current_snapshot, new_snap.id);
    }

    #[test]
    fn test_state_machine_storage_accessor() {
        let storage = setup_storage();
        let sm = StateMachine::new(storage.clone());
        let retrieved = sm.storage();
        // Verify we can access the storage through the state machine
        let partitions = retrieved.list_partitions();
        assert!(partitions.is_ok());
    }
}
