pub mod e2e;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use stratum::core::delta::{Delta, LineDiff};
use stratum::core::file_node::FileNode;
use stratum::core::partition::Partition;
use stratum::core::snapshot::Snapshot;
use stratum::core::types::{ContentId, SnapshotId, SourceType};
use stratum::engine::merge::apply_deltas;
use stratum::storage::migrations;
use stratum::storage::repository::{DeltaStore, FileNodeStore, SnapshotStore};
use stratum::storage::sqlite_storage::SqliteStorage;

/// Create an in-memory SqliteStorage with P1 tables only.
pub fn create_storage() -> SqliteStorage {
    SqliteStorage::new_in_memory().unwrap()
}

/// Create an in-memory SqliteStorage with full tables (P1 + checkpoint).
pub fn create_full_storage() -> SqliteStorage {
    let conn = Connection::open_in_memory().unwrap();
    migrations::initialize_full(&conn).unwrap();
    let conn = Arc::new(Mutex::new(conn));
    SqliteStorage::new_with_connection_arc(&conn)
}

/// Create an initial Snapshot for a file with the given content.
/// Returns the SnapshotId.
pub fn create_initial_snapshot(
    storage: &SqliteStorage,
    path: &str,
    content: &str,
) -> SnapshotId {
    let file_node = FileNode::new(PathBuf::from(path), content.as_bytes());
    storage
        .store_file_node(&file_node, content.as_bytes())
        .unwrap();
    let empty_diff = LineDiff::new(vec![]);
    let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Manual);
    storage.store_delta(&delta).unwrap();
    let snapshot = Snapshot::new_initial(file_node, delta.id);
    storage.store_snapshot(&snapshot, b"").unwrap();
    snapshot.id
}

/// Initialize a minimal repository with manual and staged partitions.
/// Returns (initial_snapshot_id, manual_partition, staged_partition).
pub fn init_repo(
    storage: &SqliteStorage,
    path: &str,
    content: &str,
) -> (SnapshotId, Partition, Partition) {
    let snapshot_id = create_initial_snapshot(storage, path, content);
    let manual =
        stratum::state_machine::manual::ensure_manual_partition(storage, snapshot_id).unwrap();
    let staged =
        stratum::state_machine::staged::ensure_staged_partition(storage, snapshot_id).unwrap();
    (snapshot_id, manual, staged)
}

/// Reconstruct the full text content of a Snapshot by resolving its Delta chain.
pub fn reconstruct_text(storage: &SqliteStorage, snapshot_id: &SnapshotId) -> String {
    let snapshot = storage.get_snapshot(snapshot_id).unwrap();
    let file_content = storage.get_file_content(&snapshot.file).unwrap();
    let content_str = String::from_utf8_lossy(&file_content).to_string();
    let deltas = storage.get_deltas(&snapshot.deltas).unwrap();
    apply_deltas(&content_str, &deltas).unwrap()
}

/// Create a simple non-empty Delta (single insert op).
pub fn make_insert_delta(file: &FileNode, line: &str) -> Delta {
    let hunk = stratum::core::types::Hunk {
        old_start: 1,
        old_len: 0,
        new_start: 1,
        new_len: 1,
        ops: vec![stratum::core::types::DiffOp::Insert {
            new_start: 1,
            lines: vec![line.to_string()],
        }],
    };
    let diff = LineDiff::new(vec![hunk]);
    Delta::new(file.clone(), diff, SourceType::Manual)
}

/// Create a checkpoint for use in tests that need checkpoint tables.
pub fn make_checkpoint_id(data: &[u8]) -> stratum::core::types::CheckpointId {
    ContentId::from_content(data)
}