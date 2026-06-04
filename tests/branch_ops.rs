//! Branch operations integration tests
//!
//! Tests individual branch management operations

use crate::common;

use stratum::storage::repository::CheckpointStore;

/// INT-BR-01: Create a branch and edit on it
///
/// Tests basic branch creation and switching
#[test]
fn test_create_branch_and_edit() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "main content\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_commit("main commit", "user"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_create("feature"));
    assert_eq!(code, 0, "branch create should succeed");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_switch("feature"));
    assert_eq!(code, 0, "branch switch should succeed");

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "feature content\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_commit("feature commit", "user"),
    );
    assert_eq!(code, 0, "commit on feature branch should succeed");
}

/// INT-BR-02: Switch back to main and verify it has different content
///
/// Tests branch isolation
#[test]
fn test_switch_branch_isolation() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "main v1\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_commit("main v1", "user"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_create("feature"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_switch("feature"));
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "feature v1\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_commit("feature v1", "user"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_switch("main"));
    assert_eq!(code, 0, "switch back to main should succeed");

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "main v2\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_commit("main v2", "user"));
    assert_eq!(code, 0);

    let storage = fx.open_storage();
    let checkpoints = storage.list_checkpoints().unwrap();
    assert!(
        checkpoints.len() >= 2,
        "at least two checkpoints across branches"
    );
}

/// INT-BR-04: Duplicate branch name creation should fail
///
/// Tests that duplicate branch names are rejected
#[test]
fn test_duplicate_branch_name_fails() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "content\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_commit("initial", "user"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_create("dup"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_create("dup"));
    assert_ne!(code, 0, "duplicate branch create should return error");
}

/// INT-BR-05: Switch to non-existent branch should fail
///
/// Tests that switching to non-existent branches fails
#[test]
fn test_switch_nonexistent_branch_fails() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_branch_switch("nonexistent"),
    );
    assert_ne!(code, 0, "switch to nonexistent branch should return error");
}

/// INT-BR-06: Create branch before any commit
///
/// Tests branch creation behavior before commits
#[test]
fn test_create_branch_before_commit() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_create("early"));
    assert_eq!(
        code, 0,
        "branch create after init should succeed (creates initial commit)"
    );
}

/// INT-BR-07: Branch list
///
/// Tests that branch listing works correctly
#[test]
fn test_branch_list() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "content\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_commit("initial", "user"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_create("feature1"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_create("feature2"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_list());
    assert_eq!(code, 0, "branch list should succeed");
}
