use crate::core::file_node::FileNode;
use crate::core::types::{ContentId, DeltaId, SnapshotId};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct SnapshotForId<'a> {
    file: &'a FileNode,
    deltas: &'a Vec<DeltaId>,
    parents: &'a Vec<SnapshotId>,
    partition_type: &'a str,
    has_conflicts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub file: FileNode,
    pub deltas: Vec<DeltaId>,
    pub parents: Vec<SnapshotId>,
    pub partition_type: String,
    pub created_at: i64,
    pub has_conflicts: bool,
}

impl Snapshot {
    pub fn new_initial(file: FileNode, delta_id: DeltaId) -> Self {
        let snapshot = Snapshot {
            id: ContentId([0u8; 32]),
            file,
            deltas: vec![delta_id],
            parents: vec![],
            partition_type: String::new(),
            created_at: chrono::Utc::now().timestamp_millis(),
            has_conflicts: false,
        };
        let mut s = snapshot;
        s.id = s.compute_id();
        s
    }

    pub fn from_parent(parent: &Snapshot, delta_id: DeltaId, partition_type: String) -> Self {
        let mut deltas = parent.deltas.clone();
        deltas.push(delta_id);

        let snapshot = Snapshot {
            id: ContentId([0u8; 32]),
            file: parent.file.clone(),
            deltas,
            parents: vec![parent.id],
            partition_type,
            created_at: chrono::Utc::now().timestamp_millis(),
            has_conflicts: false,
        };
        let mut s = snapshot;
        s.id = s.compute_id();
        s
    }

    pub fn apply_delta(&self, delta_id: DeltaId) -> Self {
        Snapshot::from_parent(self, delta_id, self.partition_type.clone())
    }

    pub fn merge(
        parents: Vec<&Snapshot>,
        delta_id: DeltaId,
        partition_type: String,
        has_conflicts: bool,
    ) -> Self {
        let file = parents[0].file.clone();
        let mut deltas = parents[0].deltas.clone();
        deltas.push(delta_id);

        let snapshot = Snapshot {
            id: ContentId([0u8; 32]),
            file,
            deltas,
            parents: parents.iter().map(|p| p.id).collect(),
            partition_type,
            created_at: chrono::Utc::now().timestamp_millis(),
            has_conflicts,
        };
        let mut s = snapshot;
        s.id = s.compute_id();
        s
    }

    pub fn compute_id(&self) -> SnapshotId {
        let snapshot_for_id = SnapshotForId {
            file: &self.file,
            deltas: &self.deltas,
            parents: &self.parents,
            partition_type: &self.partition_type,
            has_conflicts: self.has_conflicts,
        };
        let json = serde_json::to_vec(&snapshot_for_id).unwrap_or_default();
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
    has_conflicts: bool,
}

impl SnapshotBuilder {
    pub fn new() -> Self {
        SnapshotBuilder {
            file: None,
            deltas: vec![],
            parents: vec![],
            partition_type: String::new(),
            has_conflicts: false,
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

    pub fn with_conflicts(mut self, has_conflicts: bool) -> Self {
        self.has_conflicts = has_conflicts;
        self
    }

    pub fn build(self) -> Result<Snapshot, &'static str> {
        let file = self.file.ok_or("file is required")?;
        let snapshot = Snapshot {
            id: ContentId([0u8; 32]),
            file,
            deltas: self.deltas,
            parents: self.parents,
            partition_type: self.partition_type,
            created_at: chrono::Utc::now().timestamp_millis(),
            has_conflicts: self.has_conflicts,
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
