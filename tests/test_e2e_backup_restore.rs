mod common;

use common::e2e;
use stratum::storage::repository::{PartitionStore, SnapshotStore};

/// Helper: get the backup db path (same directory as the main db)
fn backup_db_path(fx: &e2e::E2eFixture) -> std::path::PathBuf {
    // db_path is dir/.stratum/stratum.db, so backup is dir/.stratum/stratum-backup.db
    let db_dir = fx.db_path().parent().expect("db_path should have a parent");
    db_dir.join("stratum-backup.db")
}

/// E2E-BK-01: Create a backup and verify it exists
///
/// Steps:
///   1. init -> edit f.txt
///   2. Get staged snapshot ID from storage
///   3. backup <snapshot_id> --label "test-label"
///   4. Verify backup file was created
#[test]
fn test_create_backup() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "backup content\n"));
    assert_eq!(code, 0);

    // Get staged snapshot hex for backup
    let storage = fx.open_storage();
    let snapshot_hex = e2e::staged_snapshot_hex(&storage)
        .expect("staged snapshot should exist");
    drop(storage);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_backup(&snapshot_hex, Some("test-label")));
    assert_eq!(code, 0, "backup should succeed");

    // Verify backup.db file exists in the same directory as the main db
    let backup_db_path = backup_db_path(&fx);
    assert!(backup_db_path.exists(), "backup database file should exist at {:?}", backup_db_path);
}

/// E2E-BK-02: Create backup without label
///
/// Steps:
///   1. init -> edit f.txt
///   2. backup <snapshot_id> (no label)
///   3. Should succeed
#[test]
fn test_create_backup_no_label() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "no label\n"));
    assert_eq!(code, 0);

    let storage = fx.open_storage();
    let snapshot_hex = e2e::staged_snapshot_hex(&storage)
        .expect("staged snapshot should exist");
    drop(storage);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_backup(&snapshot_hex, None));
    assert_eq!(code, 0, "backup without label should succeed");
}

/// E2E-BK-03: Invalid snapshot ID backup should fail
///
/// Steps:
///   1. init
///   2. backup deadbeef (invalid snapshot ID)
///   3. Should return USAGE_ERROR
#[test]
fn test_backup_invalid_snapshot_id() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_backup("deadbeef", None));
    assert_eq!(code, 2, "invalid snapshot ID should return USAGE_ERROR");
}

/// E2E-BK-04: Restore from backup
///
/// Steps:
///   1. init -> edit "original content" -> backup
///   2. edit "modified content" (overwrite)
///   3. restore backup
///   4. Verify restored deltas exist in storage
#[test]
fn test_restore_from_backup() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "original content\n"));
    assert_eq!(code, 0);

    let storage = fx.open_storage();
    let snapshot_hex = e2e::staged_snapshot_hex(&storage)
        .expect("staged snapshot should exist");
    let snapshot_id = e2e::staged_snapshot_id(&storage).unwrap();
    drop(storage);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_backup(&snapshot_hex, Some("pre-modify")));
    assert_eq!(code, 0);

    // Modify content
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "modified content\n"));
    assert_eq!(code, 0);

    // Get backup ID using query_backups API from the correct path
    let backup_db_path = backup_db_path(&fx);
    assert!(backup_db_path.exists(), "backup db should exist at {:?}", backup_db_path);
    let backup_repo = stratum::backup::backup_repo::BackupRepo::new(&backup_db_path)
        .expect("backup repo should open");
    let filter = stratum::backup::backup_snapshot::BackupFilter::new();
    let backups = backup_repo.query_backups(&filter).expect("should query backups");
    assert!(!backups.is_empty(), "should have at least one backup");
    let backup_id = backups[0].id;
    drop(backup_repo);

    // Restore from backup
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_restore(&backup_id.to_hex()));
    assert_eq!(code, 0, "restore should succeed");

    // Verify the backup's snapshot delta was restored into storage
    let storage = fx.open_storage();
    let restored_snapshot = storage.get_snapshot(&snapshot_id)
        .expect("original snapshot should still exist");
    assert!(!restored_snapshot.deltas.is_empty(), "snapshot should have deltas");
}

/// E2E-BK-05: Restore with invalid backup ID should fail
///
/// Steps:
///   1. init
///   2. restore deadbeef... (invalid/unknown backup ID)
///   3. Should return error
#[test]
fn test_restore_invalid_backup_id() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_restore("0000000000000000000000000000000000000000000000000000000000000000"));
    assert_eq!(code, 1, "restore with invalid backup ID should return error");
}