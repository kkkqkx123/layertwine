use serde::{Deserialize, Serialize};
use crate::core::types::{LayerType, PartitionId};

/// Layer — 分层（可变容器）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    /// 分层类型
    pub layer_type: LayerType,
    /// 归属本层的所有分区 ID
    pub partitions: Vec<PartitionId>,
}

impl Layer {
    pub fn new(layer_type: LayerType) -> Self {
        Layer {
            layer_type,
            partitions: vec![],
        }
    }

    pub fn add_partition(&mut self, partition_id: PartitionId) {
        if !self.partitions.contains(&partition_id) {
            self.partitions.push(partition_id);
        }
    }

    pub fn remove_partition(&mut self, partition_id: &PartitionId) {
        self.partitions.retain(|p| p != partition_id);
    }

    pub fn has_partition(&self, partition_id: &PartitionId) -> bool {
        self.partitions.contains(partition_id)
    }
}
