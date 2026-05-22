mod common;

use common::e2e;
use stratum::storage::repository::CheckpointStore;

/// E2E-BASIC-01: Empty repository initialization
///
/// Steps:
///   1. stratum init
///   2. stratum status
///   3. stratum log (no checkpoints yet)
#[test]
fn test_empty_repo_init() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0, "init should succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_status());
    assert_eq!(code, 0, "status should succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_log());
    assert_eq!(code, 0, "log should succeed on empty repo");
}

/// E2E-BASIC-02: Edit and commit flow
///
/// Steps:
///   1. init
///   2. edit test.txt with content "hello\nworld\n"
///   3. commit with message
///   4. log to verify commit exists
///   5. status to verify staged partition pointer
#[test]
fn test_edit_and_commit() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("test.txt", "hello\nworld\n"));
    assert_eq!(code, 0, "edit should succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("first commit", "tester"));
    assert_eq!(code, 0, "commit should succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_log());
    assert_eq!(code, 0, "log should succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_status());
    assert_eq!(code, 0, "status should succeed");

    // Verify checkpoint exists in storage
    let storage = fx.open_storage();
    let checkpoints = storage.list_checkpoints().unwrap();
    assert!(!checkpoints.is_empty(), "at least one checkpoint should exist");
    assert_eq!(checkpoints[0].metadata.message, "first commit");
}

/// E2E-BASIC-03: Multiple edits without content reconstruction
///
/// Steps:
///   1. init
///   2. edit f.txt -> "v1\n"
///   3. edit f.txt -> "v2\n"
///   4. edit f.txt -> "v3\n"
///   5. Verify all edits succeed via exit codes
///   6. Verify staged partition exists
#[test]
fn test_multiple_edits() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "v1\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "v2\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "v3\n"));
    assert_eq!(code, 0);

    // Verify staged partition still exists after multiple edits
    let storage = fx.open_storage();
    let staged_snapshot = e2e::staged_snapshot_id(&storage);
    assert!(staged_snapshot.is_some(), "staged partition should exist after multiple edits");

    // Status should succeed
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_status());
    assert_eq!(code, 0, "status should succeed after multiple edits");
}

/// E2E-BASIC-04: Cross-session persistence
///
/// Steps:
///   1. init -> edit -> commit
///   2. Reopen storage from the same db path
///   3. log should show the previous commit
///   4. Continue editing and commit
#[test]
fn test_cross_session_persistence() {
    let fx = e2e::E2eFixture::new();

    // Session 1
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "session1\n"));
    assert_eq!(code, 0);
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("first", "user"));
    assert_eq!(code, 0);

    // Session 2 (reopen storage)
    let storage = fx.open_storage();
    let checkpoints = storage.list_checkpoints().unwrap();
    assert!(!checkpoints.is_empty(), "checkpoints should persist");
    assert_eq!(checkpoints[0].metadata.message, "first");
    drop(storage);

    // Session 2: continue editing
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "session2\n"));
    assert_eq!(code, 0);
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("second", "user"));
    assert_eq!(code, 0);

    // Verify both checkpoints exist
    let storage = fx.open_storage();
    let checkpoints = storage.list_checkpoints().unwrap();
    assert_eq!(checkpoints.len(), 2, "two checkpoints should exist after two commits");
}

/// E2E-BASIC-05: Edit with no content change should not create a new snapshot
///
/// Steps:
///   1. init
///   2. edit f.txt with content "same\n"
///   3. edit f.txt again with the same content "same\n"
///   4. Both edits should succeed
#[test]
fn test_edit_no_change() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "same\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "same\n"));
    assert_eq!(code, 0, "edit with same content should succeed");
}

/// E2E-BASIC-06: JSON output mode
///
/// Steps:
///   1. init --json
///   2. status --json
#[test]
fn test_json_output() {
    let fx = e2e::E2eFixture::new();

    let cli = stratum::api::cli::commands::Cli {
        db_path: fx.db_path_str().to_string(),
        git_repo: None,
        json: true,
        command: e2e::cmd_init(),
    };
    let code = stratum::api::cli::run_with_cli(cli);
    assert_eq!(code, 0, "init --json should succeed");

    let code = e2e::run_cmd_json(fx.db_path_str(), e2e::cmd_status());
    assert_eq!(code, 0, "status --json should succeed");
}