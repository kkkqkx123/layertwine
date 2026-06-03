use serde::{Deserialize, Serialize};
use crate::core::types::{CheckpointId, ContentId, SnapshotId};

/// Checkpoint Metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointMetadata {
    /// Author / Agent ID
    pub author: String,
    /// Submit information
    pub message: String,
    /// Git synchronization anchor Git commit hash (optional)
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

/// Checkpoint - Self-Research Submission Unit
///
/// Immutable and lightweight (only Delta references are stored, not full files).
/// Content addressing ID (determined by content hash).
/// Single parent = linear commit, multiple parents = branch merge.
/// `baseline_snapshots` stores the snapshot IDs of all files involved in the commit.
/// This allows a Checkpoint to correspond to a single multi-file commit in Git.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Unique hash ID (content addressing)
    pub id: CheckpointId,
    /// Parent submission (single parent = normal, multiple parents = merged)
    pub parents: Vec<CheckpointId>,
    /// Baseline snapshot of all files at commit time (supports multi-file commits)
    pub baseline_snapshots: Vec<SnapshotId>,
    /// metadata
    pub metadata: CheckpointMetadata,
    /// Creation time (Unix milliseconds)
    pub created_at: i64,
}

impl Checkpoint {
    /// Create new Checkpoints (IDs are automatically calculated), support for multi-file snapshots
    pub fn new(
        baseline_snapshots: Vec<SnapshotId>,
        parents: Vec<CheckpointId>,
        metadata: CheckpointMetadata,
    ) -> Self {
        let mut cp = Checkpoint {
            id: ContentId([0u8; 32]),
            parents,
            baseline_snapshots,
            metadata,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        cp.id = cp.compute_id();
        cp
    }

    /// Convenient construction for single-file snapshot compatibility
    pub fn new_single(
        baseline_snapshot: SnapshotId,
        parents: Vec<CheckpointId>,
        metadata: CheckpointMetadata,
    ) -> Self {
        Checkpoint::new(vec![baseline_snapshot], parents, metadata)
    }

    /// Content-based ID calculation (content addressing)
    ///
    /// Excludes `created_at` and `git_anchor` from the hash:
    /// - `created_at` is a runtime timestamp, not content
    /// - `git_anchor` is post-hoc external metadata set after push,
    ///   not part of the checkpoint's intrinsic content identity
    pub fn compute_id(&self) -> CheckpointId {
        let mut clone = self.clone();
        clone.created_at = 0;
        clone.metadata.git_anchor = None;
        let json = serde_json::to_vec(&clone).unwrap_or_default();
        CheckpointId::from_content(&json)
    }
}

/// Checkpoint Chained Constructor (refer to jj CommitBuilder)
#[derive(Debug, Clone)]
pub struct CheckpointBuilder {
    parents: Vec<CheckpointId>,
    baseline_snapshots: Vec<SnapshotId>,
    author: String,
    message: String,
    git_anchor: Option<String>,
}

impl CheckpointBuilder {
    pub fn new() -> Self {
        CheckpointBuilder {
            parents: vec![],
            baseline_snapshots: vec![],
            author: "unknown".to_string(),
            message: String::new(),
            git_anchor: None,
        }
    }

    /// Add Parent Submission
    pub fn parent(mut self, parent_id: CheckpointId) -> Self {
        self.parents.push(parent_id);
        self
    }

    /// Setting up multiple parent commits
    pub fn parents(mut self, parents: Vec<CheckpointId>) -> Self {
        self.parents = parents;
        self
    }

    /// Add a baseline snapshot
    pub fn baseline_snapshot(mut self, snapshot_id: SnapshotId) -> Self {
        self.baseline_snapshots.push(snapshot_id);
        self
    }

    /// Set all baseline snapshots
    pub fn baseline_snapshots(mut self, snapshots: Vec<SnapshotId>) -> Self {
        self.baseline_snapshots = snapshots;
        self
    }

    /// Setting the Author
    pub fn author(mut self, author: &str) -> Self {
        self.author = author.to_string();
        self
    }

    /// Setting Up Submission Information
    pub fn message(mut self, message: &str) -> Self {
        self.message = message.to_string();
        self
    }

    /// Setting Git Anchor Points
    pub fn git_anchor(mut self, anchor: &str) -> Self {
        self.git_anchor = Some(anchor.to_string());
        self
    }

    /// Building Checkpoint
    pub fn build(self) -> Result<Checkpoint, &'static str> {
        if self.baseline_snapshots.is_empty() {
            return Err("at least one baseline_snapshot is required");
        }
        let metadata = CheckpointMetadata {
            author: self.author,
            message: self.message,
            git_anchor: self.git_anchor,
        };
        Ok(Checkpoint::new(self.baseline_snapshots, self.parents, metadata))
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
        let cp = Checkpoint::new_single(snap_id, vec![], metadata);
        assert_eq!(cp.parents.len(), 0);
        assert_eq!(cp.baseline_snapshots, vec![snap_id]);
        assert_eq!(cp.metadata.author, "test-user");
        assert_eq!(cp.metadata.message, "initial commit");
    }

    #[test]
    fn test_checkpoint_multi_snapshot() {
        let snap1 = dummy_snapshot_id();
        let snap2 = ContentId::from_content(b"second-file");
        let snapshots = vec![snap1, snap2];
        let metadata = CheckpointMetadata::new("user", "multi-file commit");
        let cp = Checkpoint::new(snapshots.clone(), vec![], metadata);
        assert_eq!(cp.baseline_snapshots.len(), 2);
        assert_eq!(cp.baseline_snapshots, snapshots);
    }

    #[test]
    fn test_checkpoint_content_addressing() {
        let snap_id = dummy_snapshot_id();
        let cp1 = Checkpoint::new_single(
            snap_id,
            vec![],
            CheckpointMetadata::new("user", "message"),
        );
        let cp2 = Checkpoint::new_single(
            snap_id,
            vec![],
            CheckpointMetadata::new("user", "message"),
        );
        assert_eq!(cp1.id, cp2.id, "same content = same id");

        let cp3 = Checkpoint::new_single(
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
        assert_eq!(cp.baseline_snapshots, vec![snap_id]);
        assert_eq!(cp.parents, vec![parent_id]);
    }

    #[test]
    fn test_checkpoint_builder_empty_snapshots_fails() {
        let result = CheckpointBuilder::new()
            .author("user")
            .message("no snapshots")
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_checkpoint_builder_multi_snapshots() {
        let snap1 = dummy_snapshot_id();
        let snap2 = ContentId::from_content(b"another-file");
        let parent_id = CheckpointId::from_content(b"parent");

        let cp = CheckpointBuilder::new()
            .baseline_snapshot(snap1)
            .baseline_snapshot(snap2)
            .author("multi")
            .message("multi snapshot commit")
            .parent(parent_id)
            .build()
            .unwrap();

        assert_eq!(cp.baseline_snapshots.len(), 2);
        assert_eq!(cp.baseline_snapshots[0], snap1);
        assert_eq!(cp.baseline_snapshots[1], snap2);
    }
}