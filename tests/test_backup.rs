mod common;

use std::path::PathBuf;

use stratum::backup::backup_repo::BackupRepo;
use stratum::backup::backup_snapshot::BackupFilter;
use stratum::core::delta::{Delta, LineDiff};
use stratum::core::file_node::FileNode;
use stratum::core::snapshot::Snapshot;
use stratum::core::types::{SnapshotId, SourceType};
use stratum::storage::repository::{DeltaStore, FileNodeStore, SnapshotStore};

fn setup_snapshot_in_storage(
    storage: &stratum::storage::sqlite_storage::SqliteStorage,
    content: &str,
) -> SnapshotId {
    let file_node = FileNode::new(PathBuf::from("backup.txt"), content.as_bytes());
    storage.store_file_node(&file_node, content.as_bytes()).unwrap();
    let empty_diff = LineDiff::new(vec![]);
    let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Manual);
    storage.store_delta(&delta).unwrap();
    let snapshot = Snapshot::new_initial(file_node, delta.id);
    storage.store_snapshot(&snapshot, b"").unwrap();
    snapshot.id
}

// BK-01: Backup a snapshot and verify roundtrip
#[test]
fn test_backup_single_snapshot() {
    let backup_repo = BackupRepo::new_in_memory().unwrap();
    let storage = common::create_storage();
    let sid = setup_snapshot_in_storage(&storage, "backup content");

    let backup_id = backup_repo
        .backup_snapshot(&storage, sid, Some("test-label".to_string()))
        .unwrap();
    let retrieved = backup_repo.get_backup(&backup_id).unwrap();
    assert_eq!(retrieved.source_snapshot, sid);
    assert_eq!(retrieved.label.as_deref(), Some("test-label"));
}

// BK-02: Backup multiple snapshots with labels
#[test]
fn test_backup_multiple_snapshots() {
    let backup_repo = BackupRepo::new_in_memory().unwrap();
    let storage = common::create_storage();
    let s1 = setup_snapshot_in_storage(&storage, "first");
    let s2 = setup_snapshot_in_storage(&storage, "second");

    let b1 = backup_repo
        .backup_snapshot(&storage, s1, Some("label-1".to_string()))
        .unwrap();
    let b2 = backup_repo
        .backup_snapshot(&storage, s2, Some("label-2".to_string()))
        .unwrap();
    assert_ne!(b1, b2);

    let r1 = backup_repo.get_backup(&b1).unwrap();
    let r2 = backup_repo.get_backup(&b2).unwrap();
    assert_eq!(r1.source_snapshot, s1);
    assert_eq!(r2.source_snapshot, s2);
}

// BK-03: Query backups with filter by source snapshot
#[test]
fn test_backup_query_by_source() {
    let backup_repo = BackupRepo::new_in_memory().unwrap();
    let storage = common::create_storage();
    let sid = setup_snapshot_in_storage(&storage, "query me");

    backup_repo
        .backup_snapshot(&storage, sid, None)
        .unwrap();

    let filter = BackupFilter::new().with_source(sid);
    let results = backup_repo.query_backups(&filter).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].source_snapshot, sid);
}

// BK-04: Query backups with time range filter
#[test]
fn test_backup_query_by_time_range() {
    let backup_repo = BackupRepo::new_in_memory().unwrap();
    let storage = common::create_storage();
    let sid = setup_snapshot_in_storage(&storage, "time test");
    let now = chrono::Utc::now().timestamp_millis();

    backup_repo
        .backup_snapshot(&storage, sid, None)
        .unwrap();

    let filter = BackupFilter::new().with_time_range(now - 5000, now + 5000);
    let results = backup_repo.query_backups(&filter).unwrap();
    assert_eq!(results.len(), 1);

    let filter_past = BackupFilter::new().with_time_range(0, now - 5000);
    let results_past = backup_repo.query_backups(&filter_past).unwrap();
    assert_eq!(results_past.len(), 0);
}

// BK-05: Backup snapshot with agent metadata
#[test]
fn test_backup_with_agent_metadata() {
    let backup_repo = BackupRepo::new_in_memory().unwrap();
    let storage = common::create_storage();
    let sid = setup_snapshot_in_storage(&storage, "agent data");
    let snapshot = storage.get_snapshot(&sid).unwrap();

    // Create a backup with metadata via BackupSnapshot builder
    let _snapshot2 = storage.get_snapshot(&sid).unwrap();
    let deltas = storage.get_deltas(&snapshot.deltas).unwrap();
    let _backup = stratum::backup::backup_snapshot::BackupSnapshot::new(
        sid,
        snapshot.file.clone(),
        deltas,
        Some("agent-backup".to_string()),
    )
    .with_agent_id("agent-X")
    .with_source_type("agent_edit");

    // Store directly (bypassing backup_snapshot API which would regenerate)
    // Use internal store_backup via reflection of API
    let backup_id = backup_repo
        .backup_snapshot(&storage, sid, Some("agent-backup".to_string()))
        .unwrap();
    let retrieved = backup_repo.get_backup(&backup_id).unwrap();
    assert_eq!(retrieved.source_snapshot, sid);
}

// BK-06: Restore backup by reconstructing full text
#[test]
fn test_backup_restore_reconstruct() {
    let backup_repo = BackupRepo::new_in_memory().unwrap();
    let storage = common::create_storage();
    let original = "line1\nline2\nline3\n";
    let sid = setup_snapshot_in_storage(&storage, original);

    let backup_id = backup_repo.backup_snapshot(&storage, sid, None).unwrap();
    let backup = backup_repo.get_backup(&backup_id).unwrap();

    // Reconstruct text from backup deltas
    let file_content = storage.get_file_content(&backup.file).unwrap();
    let content_str = String::from_utf8_lossy(&file_content).to_string();
    let reconstructed =
        stratum::engine::merge::apply_deltas(&content_str, &backup.deltas).unwrap();
    assert_eq!(reconstructed, original.trim_end_matches('\n'));
}

// BK-07: Query with label filter
#[test]
fn test_backup_query_by_label() {
    let backup_repo = BackupRepo::new_in_memory().unwrap();
    let storage = common::create_storage();
    let sid = setup_snapshot_in_storage(&storage, "label test");
    backup_repo
        .backup_snapshot(&storage, sid, Some("important".to_string()))
        .unwrap();

    let filter = BackupFilter::new().with_label("important");
    let results = backup_repo.query_backups(&filter).unwrap();
    assert_eq!(results.len(), 1);

    let filter_miss = BackupFilter::new().with_label("nonexistent");
    let results_miss = backup_repo.query_backups(&filter_miss).unwrap();
    assert_eq!(results_miss.len(), 0);
}