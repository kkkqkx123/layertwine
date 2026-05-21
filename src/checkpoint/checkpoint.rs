//! Checkpoint Entity — 自研提交单元
//!
//! 基于不可变快照体系实现的轻量版本控制提交单元。
//! 参考 architecture/05-检查点仓库与分支管理.md §5.2

use serde::{Deserialize, Serialize};
use crate::core::types::{CheckpointId, ContentId, SnapshotId};

/// Checkpoint 元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointMetadata {
    /// 提交人 / Agent ID
    pub author: String,
    /// 提交信息
    pub message: String,
    /// Git 同步锚点（可选）
    pub git_anchor: Option<String>,
}

impl CheckpointMetadata {
    pub fn new(author: &str, message: &str) -> Self {
        CheckpointMetadata {
            author: author.to_string(),
            message: message.to_string(),
            git_anchor: None,
        }
    }
}

/// Checkpoint — 自研提交单元
///
/// 不可变、轻量（仅存 Delta 引用，不存全量文件）。
/// 内容寻址 ID（由内容哈希决定）。
/// 单父 = 线性提交，多父 = 分支合并。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// 唯一哈希 ID（内容寻址）
    pub id: CheckpointId,
    /// 父提交（单父 = 普通，多父 = 合并）
    pub parents: Vec<CheckpointId>,
    /// 提交时的文件基线快照
    pub baseline_snapshot: SnapshotId,
    /// 元数据
    pub metadata: CheckpointMetadata,
    /// 创建时间（Unix 毫秒）
    pub created_at: i64,
}

impl Checkpoint {
    /// 创建新的 Checkpoint（自动计算 ID）
    pub fn new(
        baseline_snapshot: SnapshotId,
        parents: Vec<CheckpointId>,
        metadata: CheckpointMetadata,
    ) -> Self {
        let mut cp = Checkpoint {
            id: ContentId([0u8; 32]),
            parents,
            baseline_snapshot,
            metadata,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        cp.id = cp.compute_id();
        cp
    }

    /// 基于内容计算 ID（内容寻址）
    pub fn compute_id(&self) -> CheckpointId {
        let json = serde_json::to_vec(self).unwrap_or_default();
        CheckpointId::from_content(&json)
    }
}

/// Checkpoint 链式构造器（参考 jj CommitBuilder）
#[derive(Debug, Clone)]
pub struct CheckpointBuilder {
    parents: Vec<CheckpointId>,
    baseline_snapshot: Option<SnapshotId>,
    author: String,
    message: String,
    git_anchor: Option<String>,
}

impl CheckpointBuilder {
    pub fn new() -> Self {
        CheckpointBuilder {
            parents: vec![],
            baseline_snapshot: None,
            author: "unknown".to_string(),
            message: String::new(),
            git_anchor: None,
        }
    }

    /// 添加父提交
    pub fn parent(mut self, parent_id: CheckpointId) -> Self {
        self.parents.push(parent_id);
        self
    }

    /// 设置多个父提交
    pub fn parents(mut self, parents: Vec<CheckpointId>) -> Self {
        self.parents = parents;
        self
    }

    /// 设置基线快照
    pub fn baseline_snapshot(mut self, snapshot_id: SnapshotId) -> Self {
        self.baseline_snapshot = Some(snapshot_id);
        self
    }

    /// 设置作者
    pub fn author(mut self, author: &str) -> Self {
        self.author = author.to_string();
        self
    }

    /// 设置提交信息
    pub fn message(mut self, message: &str) -> Self {
        self.message = message.to_string();
        self
    }

    /// 设置 Git 锚点
    pub fn git_anchor(mut self, anchor: &str) -> Self {
        self.git_anchor = Some(anchor.to_string());
        self
    }

    /// 构建 Checkpoint
    pub fn build(self) -> Result<Checkpoint, &'static str> {
        let snapshot_id = self.baseline_snapshot.ok_or("baseline_snapshot is required")?;
        let metadata = CheckpointMetadata {
            author: self.author,
            message: self.message,
            git_anchor: self.git_anchor,
        };
        Ok(Checkpoint::new(snapshot_id, self.parents, metadata))
    }
}

impl Default for CheckpointBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{ContentId, SnapshotId};

    fn dummy_snapshot_id() -> SnapshotId {
        ContentId::from_content(b"dummy-snapshot")
    }

    #[test]
    fn test_checkpoint_creation() {
        let snap_id = dummy_snapshot_id();
        let metadata = CheckpointMetadata::new("test-user", "initial commit");
        let cp = Checkpoint::new(snap_id, vec![], metadata);
        assert_eq!(cp.parents.len(), 0);
        assert_eq!(cp.metadata.author, "test-user");
        assert_eq!(cp.metadata.message, "initial commit");
    }

    #[test]
    fn test_checkpoint_content_addressing() {
        let snap_id = dummy_snapshot_id();
        let cp1 = Checkpoint::new(
            snap_id,
            vec![],
            CheckpointMetadata::new("user", "message"),
        );
        let cp2 = Checkpoint::new(
            snap_id,
            vec![],
            CheckpointMetadata::new("user", "message"),
        );
        assert_eq!(cp1.id, cp2.id, "same content = same id");

        let cp3 = Checkpoint::new(
            snap_id,
            vec![],
            CheckpointMetadata::new("other", "message"),
        );
        assert_ne!(cp1.id, cp3.id, "different author = different id");
    }

    #[test]
    fn test_checkpoint_builder() {
        let snap_id = dummy_snapshot_id();
        let parent_id = CheckpointId::from_content(b"parent");

        let cp = CheckpointBuilder::new()
            .baseline_snapshot(snap_id)
            .author("builder-user")
            .message("built checkpoint")
            .parent(parent_id)
            .build()
            .unwrap();

        assert_eq!(cp.metadata.author, "builder-user");
        assert_eq!(cp.metadata.message, "built checkpoint");
        assert_eq!(cp.parents, vec![parent_id]);
        assert_eq!(cp.baseline_snapshot, snap_id);
    }

    #[test]
    fn test_multi_parent_merge() {
        let snap_id = dummy_snapshot_id();
        let p1 = CheckpointId::from_content(b"parent1");
        let p2 = CheckpointId::from_content(b"parent2");

        let cp = CheckpointBuilder::new()
            .baseline_snapshot(snap_id)
            .author("merger")
            .message("merge branches")
            .parents(vec![p1, p2])
            .build()
            .unwrap();

        assert_eq!(cp.parents.len(), 2);
    }
}
