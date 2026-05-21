use serde::{Deserialize, Serialize};
use crate::core::types::{ContentId, DeltaId, DiffOp, SourceType};
use crate::core::file_node::FileNode;

/// Delta — 最小不可变增量
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delta {
    /// 唯一 ID（内容寻址）
    pub id: DeltaId,
    /// 关联的文件基准
    pub file: FileNode,
    /// 行级差异
    pub diff: LineDiff,
    /// 来源
    pub source: SourceType,
    /// 创建时间戳（Unix 毫秒）
    pub timestamp: i64,
}

impl Delta {
    /// 创建新 Delta（自动计算 ID）
    pub fn new(file: FileNode, diff: LineDiff, source: SourceType) -> Self {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let mut delta = Delta {
            id: ContentId([0u8; 32]), // 占位
            file,
            diff,
            source,
            timestamp,
        };
        delta.id = delta.compute_id();
        delta
    }

    /// 根据内容计算 ID
    pub fn compute_id(&self) -> DeltaId {
        let json = serde_json::to_vec(self).unwrap_or_default();
        ContentId::from_content(&json)
    }
}

/// 行级差异
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineDiff {
    pub hunks: Vec<super::types::Hunk>,
}

impl LineDiff {
    pub fn new(hunks: Vec<super::types::Hunk>) -> Self {
        LineDiff { hunks }
    }

    /// 判断是否为空 diff
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }
}

/// Application summary: how many lines changed
pub struct DeltaSummary {
    pub inserts: usize,
    pub deletes: usize,
    pub replaces: usize,
    pub total_hunks: usize,
}

impl Delta {
    /// 统计变化量
    pub fn summary(&self) -> DeltaSummary {
        let mut inserts = 0;
        let mut deletes = 0;
        let mut replaces = 0;

        for hunk in &self.diff.hunks {
            for op in &hunk.ops {
                match op {
                    DiffOp::Insert { lines, .. } => inserts += lines.len(),
                    DiffOp::Delete { count, .. } => deletes += *count as usize,
                    DiffOp::Replace { lines, .. } => replaces += lines.len(),
                    DiffOp::Equal { .. } => {}
                }
            }
        }

        DeltaSummary {
            inserts,
            deletes,
            replaces,
            total_hunks: self.diff.hunks.len(),
        }
    }
}
