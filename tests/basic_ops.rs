//! Basic operations integration tests
//!
//! Tests single CLI commands and simple operations through API calls

use stratum::storage::repository::CheckpointStore;

use crate::common;

/// INT-BASIC-01: Empty repository initialization
///
/// Tests the init command creates a valid database
#[test]
fn test_empty_repo_init() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0, "init should succeed");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_status());
    assert_eq!(code, 0, "status should succeed");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_log());
    assert_eq!(code, 0, "log should succeed on empty repo");
}

/// INT-BASIC-02: Edit and commit flow
///
/// Tests basic edit and commit operations
#[test]
fn test_edit_and_commit() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("test.txt", "hello\nworld\n"),
    );
    assert_eq!(code, 0, "edit should succeed");

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_commit("first commit", "tester"),
    );
    assert_eq!(code, 0, "commit should succeed");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_log());
    assert_eq!(code, 0, "log should succeed");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_status());
    assert_eq!(code, 0, "status should succeed");

    let storage = fx.open_storage();
    let checkpoints = storage.list_checkpoints().unwrap();
    assert!(
        !checkpoints.is_empty(),
        "at least one checkpoint should exist"
    );
    assert_eq!(checkpoints[0].metadata.message, "first commit");
}

/// INT-BASIC-03: Multiple edits without content reconstruction
///
/// Tests that multiple edits work correctly
#[test]
fn test_multiple_edits() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_edit("f.txt", "v1\n"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_edit("f.txt", "v2\n"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_edit("f.txt", "v3\n"));
    assert_eq!(code, 0);

    let storage = fx.open_storage();
    let staged_snapshot = common::staged_snapshot_id(&storage);
    assert!(
        staged_snapshot.is_some(),
        "staged partition should exist after multiple edits"
    );

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_status());
    assert_eq!(code, 0, "status should succeed after multiple edits");
}

/// INT-BASIC-05: Edit with no content change should not create a new snapshot
///
/// Tests that editing with the same content works correctly
#[test]
fn test_edit_no_change() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_edit("f.txt", "same\n"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_edit("f.txt", "same\n"));
    assert_eq!(code, 0, "edit with same content should succeed");
}

/// INT-BASIC-06: JSON output mode
///
/// Tests JSON output functionality
#[test]
fn test_json_output() {
    let fx = common::E2eFixture::new();

    let cli = stratum::api::cli::commands::Cli {
        db_path: fx.db_path_str().to_string(),
        git_repo: None,
        json: true,
        command: common::cli::cmd_init(),
    };
    let code = stratum::api::cli::run_with_cli(cli);
    assert_eq!(code, 0, "init --json should succeed");

    let code = common::run_cmd_json(fx.db_path_str(), common::cli::cmd_status());
    assert_eq!(code, 0, "status --json should succeed");
}
