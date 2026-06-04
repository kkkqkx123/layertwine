//! Test helpers for layered module tests

use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{AgentInstanceId, PartitionId, PartitionType, SourceType};
use crate::engine::diff::diff_to_line_diff;
use crate::layered::transition::reconstruct_text;
use crate::storage::repository::{DeltaStore, FileNodeStore, PartitionStore, SnapshotStore};
use crate::storage::SqliteStorage;
use std::path::PathBuf;

pub fn setup_storage() -> SqliteStorage {
    let storage = SqliteStorage::new_in_memory().unwrap();
    storage
        .with_conn(|conn| crate::storage::migrations::initialize_full(conn))
        .unwrap();
    storage
}

pub fn create_initial_snapshot(
    storage: &SqliteStorage,
    content: &str,
) -> crate::core::types::SnapshotId {
    let file_node = FileNode::new(PathBuf::from("test.txt"), content.as_bytes());
    storage
        .store_file_node(&file_node, content.as_bytes())
        .unwrap();
    let empty_diff = crate::core::types::LineDiff::new(vec![]);
    let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Manual);
    storage.store_delta(&delta).unwrap();
    let snapshot = Snapshot::new_initial(file_node, delta.id);
    storage.store_snapshot(&snapshot, b"").unwrap();
    snapshot.id
}

pub fn create_snapshot_with_content(
    storage: &SqliteStorage,
    parent_id: &crate::core::types::SnapshotId,
    content: &str,
    partition_type: &str,
) -> crate::core::types::SnapshotId {
    let parent = storage.get_snapshot(parent_id).unwrap();
    let file_node = FileNode::new(PathBuf::from("test.txt"), content.as_bytes());
    storage
        .store_file_node(&file_node, content.as_bytes())
        .unwrap();

    let parent_text = reconstruct_text(storage, &parent).unwrap();
    let diff = diff_to_line_diff(&parent_text, content);
    let delta = Delta::new(file_node, diff, SourceType::Manual);
    storage.store_delta(&delta).unwrap();

    let snap = Snapshot::from_parent(&parent, delta.id, partition_type.to_string());
    storage.store_snapshot(&snap, b"").unwrap();
    snap.id
}

pub fn create_approval_partition(
    storage: &SqliteStorage,
    agent_id: &AgentInstanceId,
    initial_snapshot_id: crate::core::types::SnapshotId,
) -> PartitionId {
    let pid = crate::layered::approval::approval_agent_partition_id(agent_id);
    let partition = Partition {
        id: pid,
        name: format!("approval/{}", agent_id),
        current_snapshot: initial_snapshot_id,
        history: vec![initial_snapshot_id],
        partition_type: PartitionType::Approval(agent_id.clone()),
    };
    storage.create_partition(&partition).unwrap();
    pid
}

pub fn create_staged_partition(
    storage: &SqliteStorage,
    initial_snapshot_id: crate::core::types::SnapshotId,
) -> PartitionId {
    let pid = crate::layered::staged::staged_partition_id();
    let partition = Partition {
        id: pid,
        name: "staged".to_string(),
        current_snapshot: initial_snapshot_id,
        history: vec![initial_snapshot_id],
        partition_type: PartitionType::Staged,
    };
    storage.create_partition(&partition).unwrap();
    pid
}

pub fn create_manual_partition(
    storage: &SqliteStorage,
    initial_snapshot_id: crate::core::types::SnapshotId,
) -> PartitionId {
    let pid = crate::layered::manual::manual_partition_id();
    let partition = Partition {
        id: pid,
        name: "manual_edit".to_string(),
        current_snapshot: initial_snapshot_id,
        history: vec![initial_snapshot_id],
        partition_type: PartitionType::Manual,
    };
    storage.create_partition(&partition).unwrap();
    pid
}
