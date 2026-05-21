use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{DeltaId, PartitionId, SnapshotId};
use crate::StorageResult;

/// Snapshot 存储 trait
pub trait SnapshotStore {
    /// 存储快照
    fn store_snapshot(&self, snapshot: &Snapshot, content: &[u8]) -> StorageResult<()>;
    /// 获取快照
    fn get_snapshot(&self, id: &SnapshotId) -> StorageResult<Snapshot>;
    /// 按路径查询快照
    fn find_snapshots_by_file(&self, file_path: &str) -> StorageResult<Vec<Snapshot>>;
    /// 按分区类型查询快照
    fn find_snapshots_by_partition(&self, partition_type: &str) -> StorageResult<Vec<Snapshot>>;
    /// 判断快照是否存在
    fn snapshot_exists(&self, id: &SnapshotId) -> StorageResult<bool>;
}

/// Delta 存储 trait
pub trait DeltaStore {
    /// 存储 Delta
    fn store_delta(&self, delta: &Delta) -> StorageResult<()>;
    /// 获取 Delta
    fn get_delta(&self, id: &DeltaId) -> StorageResult<Delta>;
    /// 批量获取 Delta
    fn get_deltas(&self, ids: &[DeltaId]) -> StorageResult<Vec<Delta>>;
    /// 判断 Delta 是否存在
    fn delta_exists(&self, id: &DeltaId) -> StorageResult<bool>;
}

/// Partition 存储 trait
pub trait PartitionStore {
    /// 创建分区
    fn create_partition(&self, partition: &Partition) -> StorageResult<()>;
    /// 更新分区指针
    fn update_pointer(&self, partition_id: &PartitionId, snapshot_id: &SnapshotId) -> StorageResult<()>;
    /// 获取分区
    fn get_partition(&self, id: &PartitionId) -> StorageResult<Partition>;
    /// 按名称获取分区
    fn get_partition_by_name(&self, name: &str) -> StorageResult<Partition>;
    /// 列出所有分区
    fn list_partitions(&self) -> StorageResult<Vec<Partition>>;
}

/// 文件节点存储 trait
pub trait FileNodeStore {
    /// 存储文件节点
    fn store_file_node(&self, file_node: &FileNode, content: &[u8]) -> StorageResult<()>;
    /// 获取文件节点对应的内容
    fn get_file_content(&self, file_node: &FileNode) -> StorageResult<Vec<u8>>;
    /// 判断文件节点是否存在
    fn file_node_exists(&self, file_node: &FileNode) -> StorageResult<bool>;
}

/// 组合存储 trait（完整存储接口）
pub trait Repository: SnapshotStore + DeltaStore + PartitionStore + FileNodeStore {}
