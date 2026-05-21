use serde::{Deserialize, Serialize};
use crate::core::types::{PartitionId, PartitionType, SnapshotId};

/// Partition — 分区（可变指针）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Partition {
    /// 分区 ID
    pub id: PartitionId,
    /// 分区名
    pub name: String,
    /// 当前生效快照 ID（指针）
    pub current_snapshot: SnapshotId,
    /// 历史快照 ID 列表（全量保留）
    pub history: Vec<SnapshotId>,
    /// 分区类型
    pub partition_type: PartitionType,
}

impl Partition {
    pub fn new(name: String, partition_type: PartitionType, initial_snapshot: SnapshotId) -> Self {
        Partition {
            id: uuid::Uuid::new_v4(),
            name,
            current_snapshot: initial_snapshot,
            history: vec![initial_snapshot],
            partition_type,
        }
    }

    /// 更新当前快照指针（保留历史）
    pub fn advance(&mut self, new_snapshot: SnapshotId) {
        self.current_snapshot = new_snapshot;
        self.history.push(new_snapshot);
    }

    /// 回退到历史中指定 ID（仅切换指针，不动数据）
    pub fn rollback_to(&mut self, target_snapshot: &SnapshotId) -> bool {
        if let Some(pos) = self.history.iter().position(|s| s == target_snapshot) {
            self.current_snapshot = *target_snapshot;
            self.history.truncate(pos + 1);
            true
        } else {
            false
        }
    }

    /// 回退一步
    pub fn rollback_one(&mut self) -> Option<SnapshotId> {
        if self.history.len() > 1 {
            let prev = self.history[self.history.len() - 2];
            self.current_snapshot = prev;
            self.history.pop();
            Some(prev)
        } else {
            None
        }
    }
}
