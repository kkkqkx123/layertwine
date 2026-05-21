use serde::{Deserialize, Serialize};
use crate::core::types::{ContentId, DeltaId, SnapshotId};
use crate::core::file_node::FileNode;

/// Snapshot - Immutable state snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Unique ID (content addressing)
    pub id: SnapshotId,
    /// Linked document benchmarks
    pub file: FileNode,
    /// Incremental list (in order of application)
    pub deltas: Vec<DeltaId>,
    /// List of parent snapshot IDs (single parent = normal, multiple parents = merged)
    pub parents: Vec<SnapshotId>,
    /// Attributed partition type
    pub partition_type: String,
    /// Creating timestamps (Unix milliseconds)
    pub created_at: i64,
}

impl Snapshot {
    /// Creating the initial snapshot (first version)
    pub fn new_initial(file: FileNode, delta_id: DeltaId) -> Self {
        let snapshot = Snapshot {
            id: ContentId([0u8; 32]), // occupy a position
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

    /// Creating a new snapshot based on a parent snapshot
    pub fn from_parent(
        parent: &Snapshot,
        delta_id: DeltaId,
        partition_type: String,
    ) -> Self {
        let mut deltas = parent.deltas.clone();
        deltas.push(delta_id);

        let snapshot = Snapshot {
            id: ContentId([0u8; 32]), // occupy a position
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

    /// Apply an increment on the current snapshot to generate a new snapshot
    ///
    /// Equivalent to Snapshot::from_parent - appends a new Delta at the end of an incremental chain of existing snapshots.
    /// Returns a sub-snapshot containing the new Delta.
    pub fn apply_delta(&self, delta_id: DeltaId) -> Self {
        Snapshot::from_parent(self, delta_id, self.partition_type.clone())
    }

    /// Merge snapshots (multiple parents)
    pub fn merge(parents: Vec<&Snapshot>, delta_id: DeltaId, partition_type: String) -> Self {
        let file = parents[0].file.clone();
        let deltas = vec![delta_id];

        let snapshot = Snapshot {
            id: ContentId([0u8; 32]), // occupy a position
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

    /// Calculate ID based on content
    pub fn compute_id(&self) -> SnapshotId {
        let json = serde_json::to_vec(self).unwrap_or_default();
        SnapshotId::from_content(&json)
    }
}

/// Snapshot builder (chaining construction)
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
            id: ContentId([0u8; 32]), // occupy a position
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
