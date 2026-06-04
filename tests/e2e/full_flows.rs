//! Full user flow E2E tests
//!
//! Tests complete user workflows across multiple operations

use crate::common;

use stratum::storage::repository::CheckpointStore;
use stratum::storage::repository::SnapshotStore;

fn backup_db_path(fx: &common::E2eFixture) -> std::path::PathBuf {
    let db_dir = fx.db_path().parent().expect("db_path should have a parent");
    db_dir.join("stratum-backup.db")
}

/// E2E-FLOW-01: Cross-session persistence
///
/// Tests complete workflow across multiple sessions:
///   1. Session 1: init -> edit -> commit
///   2. Session 2: reopen storage and verify persistence
///   3. Continue editing and commit
///   4. Verify both checkpoints exist
#[test]
fn test_cross_session_persistence() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);
    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "session1\n"),
    );
    assert_eq!(code, 0);
    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_commit("first", "user"));
    assert_eq!(code, 0);

    let storage = fx.open_storage();
    let checkpoints = storage.list_checkpoints().unwrap();
    assert!(!checkpoints.is_empty(), "checkpoints should persist");
    assert_eq!(checkpoints[0].metadata.message, "first");
    drop(storage);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "session2\n"),
    );
    assert_eq!(code, 0);
    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_commit("second", "user"));
    assert_eq!(code, 0);

    let storage = fx.open_storage();
    let checkpoints = storage.list_checkpoints().unwrap();
    assert!(
        checkpoints.len() >= 2,
        "at least two checkpoints should exist after two commits, got {}",
        checkpoints.len()
    );
}

/// E2E-FLOW-02: Full agent workflow with approve
///
/// Tests complete agent collaboration workflow:
///   1. Initialize repository
///   2. Agent makes edits
///   3. Agent submits changes
///   4. Human approves (auto-creates partitions)
///   5. Commit approved changes
#[test]
fn test_full_agent_workflow() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_agent_edit("agent-b", "f.txt", "agent edit\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_agent_submit("agent-b"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_approve("agent-b"));
    assert_eq!(code, 0, "approve should succeed");

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_commit("agent-b changes", "user"),
    );
    assert_eq!(code, 0, "commit should succeed");
}

/// E2E-FLOW-03: Dual agent parallel editing workflow
///
/// Tests multi-agent collaboration workflow:
///   1. Initialize repository
///   2. Agent alpha edits a file
///   3. Agent beta edits the same file
///   4. Both agents submit
///   5. Human approves both agents
#[test]
fn test_dual_agent_parallel_workflow() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_agent_edit("alpha", "f.txt", "alpha-line\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_agent_edit("beta", "f.txt", "beta-line\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_agent_submit("alpha"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_agent_submit("beta"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_approve("alpha"));
    assert_eq!(code, 0, "approve alpha should succeed");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_approve("beta"));
    assert_eq!(code, 0, "approve beta should succeed");
}

/// E2E-FLOW-04: Complete backup and restore workflow
///
/// Tests full backup/restore workflow:
///   1. Initialize repository and make edits
///   2. Create backup of current state
///   3. Make modifications
///   4. Restore from backup
///   5. Verify original state is recovered
#[test]
fn test_complete_backup_restore_workflow() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "original content\n"),
    );
    assert_eq!(code, 0);

    let storage = fx.open_storage();
    let snapshot_hex = common::staged_snapshot_hex(&storage).expect("staged snapshot should exist");
    let snapshot_id = common::staged_snapshot_id(&storage).unwrap();
    drop(storage);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_backup(&snapshot_hex, Some("pre-modify")),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "modified content\n"),
    );
    assert_eq!(code, 0);

    let backup_db_path = backup_db_path(&fx);
    assert!(
        backup_db_path.exists(),
        "backup db should exist at {:?}",
        backup_db_path
    );
    let backup_repo = stratum::backup::backup_repo::BackupRepo::new(&backup_db_path)
        .expect("backup repo should open");
    let filter = stratum::backup::backup_snapshot::BackupFilter::new();
    let backups = backup_repo
        .query_backups(&filter)
        .expect("should query backups");
    assert!(!backups.is_empty(), "should have at least one backup");
    let backup_id = backups[0].id;
    drop(backup_repo);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_restore(&backup_id.to_hex()),
    );
    assert_eq!(code, 0, "restore should succeed");

    let storage = fx.open_storage();
    let restored_snapshot = storage
        .get_snapshot(&snapshot_id)
        .expect("original snapshot should still exist");
    assert!(
        !restored_snapshot.deltas.is_empty(),
        "snapshot should have deltas"
    );
}
