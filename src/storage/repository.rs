use crate::checkpoint::branch::Branch;
use crate::checkpoint::types::Checkpoint;
use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::layer::Layer;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{
    CheckpointId, DeltaId, LayerType, PartitionId, PartitionType, SnapshotId,
};
use crate::StorageResult;

/// Snapshot storage trait
pub trait SnapshotStore {
    /// Storage Snapshot
    fn store_snapshot(&self, snapshot: &Snapshot, content: &[u8]) -> StorageResult<()>;

    /// Batch store snapshots with atomic guarantee
    ///
    /// Default implementation stores snapshots sequentially.
    /// Implementations can override this for better performance using transactions.
    fn store_snapshots_batch(&self, snapshots: &[(&Snapshot, &[u8])]) -> StorageResult<()> {
        for (snapshot, content) in snapshots {
            self.store_snapshot(snapshot, content)?;
        }
        Ok(())
    }

    /// Getting a Snapshot
    fn get_snapshot(&self, id: &SnapshotId) -> StorageResult<Snapshot>;
    /// Query Snapshots by Path
    fn find_snapshots_by_file(&self, file_path: &str) -> StorageResult<Vec<Snapshot>>;
    /// Query Snapshots by Partition Type
    fn find_snapshots_by_partition(
        &self,
        partition_type: &PartitionType,
    ) -> StorageResult<Vec<Snapshot>>;
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
    fn update_pointer(
        &self,
        partition_id: &PartitionId,
        snapshot_id: &SnapshotId,
    ) -> StorageResult<()>;
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
    /// Get file content by path and base hash
    fn get_file_content(&self, file_path: &str, base_hash: &[u8; 32]) -> StorageResult<Vec<u8>>;
    /// Determine if a file node exists
    fn file_node_exists(&self, file_path: &str, base_hash: &[u8; 32]) -> StorageResult<bool>;
}

/// Layer storage trait
pub trait LayerStore {
    /// Store or update a layer
    fn store_layer(&self, layer: &Layer) -> StorageResult<()>;
    /// Get a layer by its type
    fn get_layer(&self, layer_type: &LayerType) -> StorageResult<Layer>;
    /// List all layer types
    fn list_layer_types(&self) -> StorageResult<Vec<LayerType>>;
    /// Delete a layer
    fn delete_layer(&self, layer_type: &LayerType) -> StorageResult<()>;
}

/// Atomic operations trait for transactional guarantees
pub trait AtomicOps {
    /// Execute the given closure with atomic (transactional) guarantees.
    ///
    /// Default implementation: no-op wrapping (caller manages atomicity).
    /// SQLite backend overrides this with SAVEPOINT-based transactions.
    fn with_atomic<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&Self) -> StorageResult<T>,
    {
        f(self)
    }
}

/// Combined storage trait (full storage interface)
pub trait Repository:
    SnapshotStore
    + DeltaStore
    + PartitionStore
    + FileNodeStore
    + CheckpointStore
    + BranchStore
    + LayerStore
    + MetadataStore
    + AtomicOps
{
}

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

/// Metadata storage trait
///
/// Used for storing repository-wide metadata such as current branch name.
pub trait MetadataStore {
    /// Store arbitrary metadata key-value pair
    fn store_metadata(&self, key: &str, value: &str) -> StorageResult<()>;
    /// Load metadata value by key
    fn load_metadata(&self, key: &str) -> StorageResult<Option<String>>;
}

/// Combined checkpoint persistence trait (for auto-persist in CheckpointRepo)
pub trait CheckpointPersist:
    CheckpointStore + BranchStore + MetadataStore + SnapshotStore + Send + Sync
{
}
impl<T: CheckpointStore + BranchStore + MetadataStore + SnapshotStore + Send + Sync>
    CheckpointPersist for T
{
}
