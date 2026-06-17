use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::types::{BackupId, ContentId, SnapshotId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSnapshot {
    pub id: BackupId,
    pub source_snapshot: SnapshotId,
    pub file: FileNode,
    pub deltas: Vec<Delta>,
    pub label: Option<String>,
    pub backed_at: i64,
    pub metadata: HashMap<String, String>,
    pub agent_id: Option<String>,
    pub source_type: Option<String>,
    pub file_content: Vec<u8>,
}

impl BackupSnapshot {
    pub fn new(
        source_snapshot: SnapshotId,
        file: FileNode,
        deltas: Vec<Delta>,
        label: Option<String>,
        file_content: Vec<u8>,
    ) -> Self {
        Self::with_options(
            source_snapshot,
            file,
            deltas,
            label,
            None,
            None,
            file_content,
        )
    }

    pub fn with_options(
        source_snapshot: SnapshotId,
        file: FileNode,
        deltas: Vec<Delta>,
        label: Option<String>,
        agent_id: Option<String>,
        source_type: Option<String>,
        file_content: Vec<u8>,
    ) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        let mut bs = BackupSnapshot {
            id: ContentId([0u8; 32]),
            source_snapshot,
            file,
            deltas,
            label,
            backed_at: now,
            metadata: HashMap::new(),
            agent_id,
            source_type,
            file_content,
        };
        bs.id = bs.compute_id();
        bs
    }

    pub fn compute_id(&self) -> BackupId {
        let mut clone = self.clone();
        clone.backed_at = 0; // Exclude timestamp from content hash
        let json = serde_json::to_vec(&clone).unwrap_or_default();
        BackupId::from_content(&json)
    }

    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self.id = self.compute_id();
        self
    }

    pub fn with_agent_id(mut self, agent_id: &str) -> Self {
        self.agent_id = Some(agent_id.to_string());
        self.id = self.compute_id();
        self
    }

    pub fn with_source_type(mut self, source_type: &str) -> Self {
        self.source_type = Some(source_type.to_string());
        self.id = self.compute_id();
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct BackupFilter {
    pub source_snapshot: Option<SnapshotId>,
    pub time_range: Option<(i64, i64)>,
    pub label: Option<String>,
    pub metadata_key: Option<String>,
    pub metadata_value: Option<String>,
    pub agent_id: Option<String>,
    pub source_type: Option<String>,
}

impl BackupFilter {
    pub fn new() -> Self {
        BackupFilter::default()
    }

    pub fn with_source(mut self, id: SnapshotId) -> Self {
        self.source_snapshot = Some(id);
        self
    }

    pub fn with_time_range(mut self, start: i64, end: i64) -> Self {
        self.time_range = Some((start, end));
        self
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.label = Some(label.to_string());
        self
    }

    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata_key = Some(key.to_string());
        self.metadata_value = Some(value.to_string());
        self
    }

    pub fn with_agent_id(mut self, agent_id: &str) -> Self {
        self.agent_id = Some(agent_id.to_string());
        self
    }

    pub fn with_source_type(mut self, source_type: &str) -> Self {
        self.source_type = Some(source_type.to_string());
        self
    }
}
