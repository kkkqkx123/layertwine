use serde::{Deserialize, Serialize};
use std::fmt;

/// Agent 实例 ID 类型
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentInstanceId(pub String);

impl fmt::Display for AgentInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for AgentInstanceId {
    fn from(s: &str) -> Self {
        AgentInstanceId(s.to_string())
    }
}

impl From<String> for AgentInstanceId {
    fn from(s: String) -> Self {
        AgentInstanceId(s)
    }
}

/// 内容 ID — Blake3 哈希包装类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentId(pub [u8; 32]);

impl ContentId {
    /// 从字节数据计算内容 ID
    pub fn from_content(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        ContentId(*hash.as_bytes())
    }

    /// 返回 16 进制字符串表示
    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }

    /// 从 16 进制字符串解析
    pub fn from_hex(s: &str) -> Option<Self> {
        let bytes = hex_decode(s)?;
        if bytes.len() != 32 {
            return None;
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Some(ContentId(arr))
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// 类型别名
pub type SnapshotId = ContentId;
pub type DeltaId = ContentId;
pub type CheckpointId = ContentId;
pub type BackupId = ContentId;
pub type PartitionId = uuid::Uuid;

/// 来源类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    Manual,
    Agent(AgentInstanceId),
    Backup,
}

/// 分层类型
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerType {
    ManualEdit,
    AgentEdit,
    Approval,
    Staged,
}

impl LayerType {
    pub fn name(&self) -> &str {
        match self {
            LayerType::ManualEdit => "manual_edit",
            LayerType::AgentEdit => "agent_edit",
            LayerType::Approval => "approval",
            LayerType::Staged => "staged",
        }
    }
}

/// 分区类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartitionType {
    /// manual_edit 唯一分区
    Manual,
    /// agent_edit 按 Agent ID 分区
    Agent(AgentInstanceId),
    /// approval 按实例分区
    Approval(AgentInstanceId),
    /// INTEGRATED 合并区
    Integrated(String),
    /// UNIFIED 汇总区
    Unified,
    /// staged 唯一分区
    Staged,
}

/// Diff 操作类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffOp {
    /// 保留 (相同行)
    Equal { count: u32 },
    /// 删除
    Delete { old_start: u32, count: u32 },
    /// 插入
    Insert { new_start: u32, lines: Vec<String> },
    /// 替换
    Replace {
        old_start: u32,
        old_count: u32,
        new_start: u32,
        lines: Vec<String>,
    },
}

/// Hunk — 连续的差异块
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    pub ops: Vec<DiffOp>,
}

/// 行级差异
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineDiff {
    pub hunks: Vec<Hunk>,
}

// ── 辅助函数 ──

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_id_from_content() {
        let data = b"hello world";
        let id1 = ContentId::from_content(data);
        let id2 = ContentId::from_content(data);
        assert_eq!(id1, id2);

        let data2 = b"hello world!";
        let id3 = ContentId::from_content(data2);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_content_id_hex_roundtrip() {
        let data = b"test data";
        let id1 = ContentId::from_content(data);
        let hex = id1.to_hex();
        let id2 = ContentId::from_hex(&hex).unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_content_id_display() {
        let data = b"hello";
        let id = ContentId::from_content(data);
        assert_eq!(id.to_string(), id.to_hex());
    }
}
