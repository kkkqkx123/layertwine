//! Backup operations integration tests
//!
//! Tests individual backup and restore operations

use crate::common;

fn backup_db_path(fx: &common::E2eFixture) -> std::path::PathBuf {
    let db_dir = fx.db_path().parent().expect("db_path should have a parent");
    db_dir.join("stratum-backup.db")
}

/// INT-BK-01: Create a backup and verify it exists
///
/// Tests basic backup creation
#[test]
fn test_create_backup() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "backup content\n"),
    );
    assert_eq!(code, 0);

    let storage = fx.open_storage();
    let snapshot_hex = common::staged_snapshot_hex(&storage).expect("staged snapshot should exist");
    drop(storage);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_backup(&snapshot_hex, Some("test-label")),
    );
    assert_eq!(code, 0, "backup should succeed");

    let backup_db_path = backup_db_path(&fx);
    assert!(
        backup_db_path.exists(),
        "backup database file should exist at {:?}",
        backup_db_path
    );
}

/// INT-BK-02: Create backup without label
///
/// Tests that backup without label works
#[test]
fn test_create_backup_no_label() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "no label\n"),
    );
    assert_eq!(code, 0);

    let storage = fx.open_storage();
    let snapshot_hex = common::staged_snapshot_hex(&storage).expect("staged snapshot should exist");
    drop(storage);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_backup(&snapshot_hex, None),
    );
    assert_eq!(code, 0, "backup without label should succeed");
}

/// INT-BK-03: Invalid snapshot ID backup should fail
///
/// Tests that invalid snapshot IDs are rejected
#[test]
fn test_backup_invalid_snapshot_id() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_backup("deadbeef", None));
    assert_ne!(code, 0, "invalid snapshot ID should return error");
}

/// INT-BK-05: Restore with invalid backup ID should fail
///
/// Tests that invalid backup IDs are rejected
#[test]
fn test_restore_invalid_backup_id() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_restore(
            "0000000000000000000000000000000000000000000000000000000000000000",
        ),
    );
    assert_eq!(
        code, 1,
        "restore with invalid backup ID should return error"
    );
}
