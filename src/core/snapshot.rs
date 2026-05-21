use serde::{Deserialize, Serialize};
use crate::core::types::{ContentId, DeltaId, SnapshotId};
use crate::core::file_node::FileNode;

/// Snapshot — 不可变状态快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// 唯一 ID（内容寻址）
    pub id: SnapshotId,
    /// 关联的文件基准
    pub file: FileNode,
    /// 增量列表（按应用顺序）
    pub deltas: Vec<DeltaId>,
    /// 父快照 ID 列表（单父=普通，多父=合并）
    pub parents: Vec<SnapshotId>,
    /// 归属分区类型
    pub partition_type: String,
    /// 创建时间戳（Unix 毫秒）
    pub created_at: i64,
}

impl Snapshot {
    /// 创建初始快照（第一个版本）
    pub fn new_initial(file: FileNode, delta_id: DeltaId) -> Self {
        let snapshot = Snapshot {
            id: ContentId([0u8; 32]), // 占位
            file,
            deltas: vec![delta_id],
            parents: vec![],
            partition_type: String::new(),
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        let mut s = snapshot;
        s.id = s.compute_id();
        s
    }

    /// 基于父快照创建新快照
    pub fn from_parent(
        parent: &Snapshot,
        delta_id: DeltaId,
        partition_type: String,
    ) -> Self {
        let mut deltas = parent.deltas.clone();
        deltas.push(delta_id);

        let snapshot = Snapshot {
            id: ContentId([0u8; 32]), // 占位
            file: parent.file.clone(),
            deltas,
            parents: vec![parent.id],
            partition_type,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        let mut s = snapshot;
        s.id = s.compute_id();
        s
    }

    /// 在当前快照上应用增量，生成新快照
    ///
    /// 等价于 Snapshot::from_parent — 在现有快照的增量链末尾追加新 Delta。
    /// 返回包含新 Delta 的子快照。
    pub fn apply_delta(&self, delta_id: DeltaId) -> Self {
        Snapshot::from_parent(self, delta_id, self.partition_type.clone())
    }

    /// 合并快照（多父）
    pub fn merge(parents: Vec<&Snapshot>, delta_id: DeltaId, partition_type: String) -> Self {
        let file = parents[0].file.clone();
        let deltas = vec![delta_id];

        let snapshot = Snapshot {
            id: ContentId([0u8; 32]), // 占位
            file,
            deltas,
            parents: parents.iter().map(|p| p.id).collect(),
            partition_type,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        let mut s = snapshot;
        s.id = s.compute_id();
        s
    }

    /// 根据内容计算 ID
    pub fn compute_id(&self) -> SnapshotId {
        let json = serde_json::to_vec(self).unwrap_or_default();
        SnapshotId::from_content(&json)
    }
}

/// Snapshot 构建器（链式构造）
#[derive(Debug, Clone)]
pub struct SnapshotBuilder {
    file: Option<FileNode>,
    deltas: Vec<DeltaId>,
    parents: Vec<SnapshotId>,
    partition_type: String,
}

impl SnapshotBuilder {
    pub fn new() -> Self {
        SnapshotBuilder {
            file: None,
            deltas: vec![],
            parents: vec![],
            partition_type: String::new(),
        }
    }

    pub fn file(mut self, file: FileNode) -> Self {
        self.file = Some(file);
        self
    }

    pub fn add_delta(mut self, delta_id: DeltaId) -> Self {
        self.deltas.push(delta_id);
        self
    }

    pub fn with_parent(mut self, parent: SnapshotId) -> Self {
        self.parents.push(parent);
        self
    }

    pub fn with_partition_type(mut self, partition_type: String) -> Self {
        self.partition_type = partition_type;
        self
    }

    pub fn build(self) -> Result<Snapshot, &'static str> {
        let file = self.file.ok_or("file is required")?;
        let snapshot = Snapshot {
            id: ContentId([0u8; 32]), // 占位
            file,
            deltas: self.deltas,
            parents: self.parents,
            partition_type: self.partition_type,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        let mut s = snapshot;
        s.id = s.compute_id();
        Ok(s)
    }
}

impl Default for SnapshotBuilder {
    fn default() -> Self {
        Self::new()
    }
}
