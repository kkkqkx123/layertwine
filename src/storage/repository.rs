use crate::checkpoint::checkpoint::Checkpoint;
use crate::checkpoint::branch::Branch;
use crate::checkpoint::dag::CheckpointDag;
use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{CheckpointId, DeltaId, PartitionId, SnapshotId};
use crate::StorageResult;

/// Snapshot storage trait
pub trait SnapshotStore {
    /// Storage Snapshot
    fn store_snapshot(&self, snapshot: &Snapshot, content: &[u8]) -> StorageResult<()>;
    /// Getting a Snapshot
    fn get_snapshot(&self, id: &SnapshotId) -> StorageResult<Snapshot>;
    /// Query Snapshots by Path
    fn find_snapshots_by_file(&self, file_path: &str) -> StorageResult<Vec<Snapshot>>;
    /// Query Snapshots by Partition Type
    fn find_snapshots_by_partition(&self, partition_type: &str) -> StorageResult<Vec<Snapshot>>;
    /// Determining if a snapshot exists
    fn snapshot_exists(&self, id: &SnapshotId) -> StorageResult<bool>;
}

/// Delta storage trait
pub trait DeltaStore {
    /// Storage Delta
    fn store_delta(&self, delta: &Delta) -> StorageResult<()>;
    /// Get Delta
    fn get_delta(&self, id: &DeltaId) -> StorageResult<Delta>;
    /// Batch acquisition of Delta
    fn get_deltas(&self, ids: &[DeltaId]) -> StorageResult<Vec<Delta>>;
    /// Determine if Delta exists
    fn delta_exists(&self, id: &DeltaId) -> StorageResult<bool>;
}

/// Partition storage trait
pub trait PartitionStore {
    /// Creating Partitions
    fn create_partition(&self, partition: &Partition) -> StorageResult<()>;
    /// Updating the partition pointer
    fn update_pointer(&self, partition_id: &PartitionId, snapshot_id: &SnapshotId) -> StorageResult<()>;
    /// Get Partition
    fn get_partition(&self, id: &PartitionId) -> StorageResult<Partition>;
    /// Get partitions by name
    fn get_partition_by_name(&self, name: &str) -> StorageResult<Partition>;
    /// List all partitions
    fn list_partitions(&self) -> StorageResult<Vec<Partition>>;
}

/// File node storage trait
pub trait FileNodeStore {
    /// Storage file node
    fn store_file_node(&self, file_node: &FileNode, content: &[u8]) -> StorageResult<()>;
    /// Get the content corresponding to the file node
    fn get_file_content(&self, file_node: &FileNode) -> StorageResult<Vec<u8>>;
    /// Determine if a file node exists
    fn file_node_exists(&self, file_node: &FileNode) -> StorageResult<bool>;
}

/// Combined storage trait (full storage interface)
pub trait Repository: SnapshotStore + DeltaStore + PartitionStore + FileNodeStore {}

/// Checkpoint storage trait
pub trait CheckpointStore {
    /// Store Checkpoint
    fn store_checkpoint(&self, checkpoint: &Checkpoint) -> StorageResult<()>;
    /// Get Checkpoint
    fn get_checkpoint(&self, id: &CheckpointId) -> StorageResult<Checkpoint>;
    /// Checkpoint exists
    fn checkpoint_exists(&self, id: &CheckpointId) -> StorageResult<bool>;
    /// List all checkpoints
    fn list_checkpoints(&self) -> StorageResult<Vec<Checkpoint>>;
    /// Delete Checkpoint
    fn delete_checkpoint(&self, id: &CheckpointId) -> StorageResult<()>;
}

/// Branch storage trait
pub trait BranchStore {
    /// Store Branch
    fn store_branch(&self, branch: &Branch) -> StorageResult<()>;
    /// Get Branch by name
    fn get_branch(&self, name: &str) -> StorageResult<Branch>;
    /// Update Branch head
    fn update_branch_head(&self, name: &str, head: &CheckpointId) -> StorageResult<()>;
    /// List all branches
    fn list_branches(&self) -> StorageResult<Vec<Branch>>;
    /// Delete Branch
    fn delete_branch(&self, name: &str) -> StorageResult<()>;
}

/// DAG storage trait
pub trait DagStore {
    /// Store DAG
    fn store_dag(&self, dag: &CheckpointDag) -> StorageResult<()>;
    /// Load DAG
    fn load_dag(&self) -> StorageResult<CheckpointDag>;
}
