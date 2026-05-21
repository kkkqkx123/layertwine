use serde::{Deserialize, Serialize};
use crate::core::types::{ContentId, DeltaId, DiffOp, SourceType};
use crate::core::file_node::FileNode;

/// Delta - minimum non-variable increment
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delta {
    /// Unique ID (content addressing)
    pub id: DeltaId,
    /// Linked document benchmarks
    pub file: FileNode,
    /// Row level differences
    pub diff: LineDiff,
    /// source (of information etc)
    pub source: SourceType,
    /// Creating timestamps (Unix milliseconds)
    pub timestamp: i64,
}

impl Delta {
    /// Creating a new Delta (automatic ID calculation)
    pub fn new(file: FileNode, diff: LineDiff, source: SourceType) -> Self {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let mut delta = Delta {
            id: ContentId([0u8; 32]), // occupy a position
            file,
            diff,
            source,
            timestamp,
        };
        delta.id = delta.compute_id();
        delta
    }

    /// Calculate ID based on content
    pub fn compute_id(&self) -> DeltaId {
        let json = serde_json::to_vec(self).unwrap_or_default();
        ContentId::from_content(&json)
    }
}

/// Row level differences
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineDiff {
    pub hunks: Vec<super::types::Hunk>,
}

impl LineDiff {
    pub fn new(hunks: Vec<super::types::Hunk>) -> Self {
        LineDiff { hunks }
    }

    /// Determine if it is empty diff
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
    /// Volume of statistical change
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
