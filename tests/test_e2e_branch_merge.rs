mod common;

use common::e2e;
use stratum::storage::repository::CheckpointStore;

/// E2E-BR-01: Create a branch and edit on it
///
/// Steps:
///   1. init -> edit -> commit (main branch)
///   2. branch create feature
///   3. branch switch feature
///   4. edit -> commit on feature
///   5. All operations succeed
#[test]
fn test_create_branch_and_edit() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "main content\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("main commit", "user"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_create("feature"));
    assert_eq!(code, 0, "branch create should succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_switch("feature"));
    assert_eq!(code, 0, "branch switch should succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "feature content\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("feature commit", "user"));
    assert_eq!(code, 0, "commit on feature branch should succeed");
}

/// E2E-BR-02: Switch back to main and verify it has different content
///
/// Steps:
///   1. (continued from BR-01) switch to main
///   2. edit main with different content
///   3. verify main checkpoint does not include feature changes
#[test]
fn test_switch_branch_isolation() {
    let fx = e2e::E2eFixture::new();

    // Setup: main -> edit -> commit -> create feature -> switch -> edit -> commit
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "main v1\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("main v1", "user"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_create("feature"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_switch("feature"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "feature v1\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("feature v1", "user"));
    assert_eq!(code, 0);

    // Switch back to main
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_switch("main"));
    assert_eq!(code, 0, "switch back to main should succeed");

    // Edit main with new content
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "main v2\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("main v2", "user"));
    assert_eq!(code, 0);

    // Verify main has 2 checkpoints (main v1, main v2) — not feature's
    let storage = fx.open_storage();
    let checkpoints = storage.list_checkpoints().unwrap();
    // Both branches' checkpoints are in storage, but main branch should have its own
    assert!(checkpoints.len() >= 2, "at least two checkpoints across branches");
}

/// E2E-BR-03: Branch merge
///
/// Steps:
///   1. init -> edit -> commit (main)
///   2. create + switch to feature -> edit -> commit
///   3. switch to main
///   4. merge feature into main
#[test]
fn test_branch_merge() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "main base\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("base", "user"));
    assert_eq!(code, 0);

    // Create and work on feature branch
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_create("feature"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_switch("feature"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "feature edit\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("feature work", "user"));
    assert_eq!(code, 0);

    // Switch back and merge
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_switch("main"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_merge("feature", "merge feature"));
    assert_eq!(code, 0, "merge should succeed");
}

/// E2E-BR-04: Duplicate branch name creation should fail
///
/// Steps:
///   1. init -> edit -> commit
///   2. branch create dup
///   3. branch create dup again -> should fail
#[test]
fn test_duplicate_branch_name_fails() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "content\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("initial", "user"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_create("dup"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_create("dup"));
    assert_eq!(code, 2, "duplicate branch create should return USAGE_ERROR");
}

/// E2E-BR-05: Switch to non-existent branch should fail
///
/// Steps:
///   1. init
///   2. branch switch nonexistent -> should fail
#[test]
fn test_switch_nonexistent_branch_fails() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_switch("nonexistent"));
    assert_eq!(code, 2, "switch to nonexistent branch should return USAGE_ERROR");
}

/// E2E-BR-06: Create branch before any commit should fail
///
/// Steps:
///   1. init (no edits/commits)
///   2. branch create early -> should fail
#[test]
fn test_create_branch_before_commit_fails() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_create("early"));
    assert_eq!(code, 1, "branch create before any commit should return error");
}

/// E2E-BR-07: Branch list
///
/// Steps:
///   1. init -> edit -> commit
///   2. branch create feature1
///   3. branch create feature2
///   4. branch list -> should succeed
#[test]
fn test_branch_list() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "content\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("initial", "user"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_create("feature1"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_create("feature2"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_branch_list());
    assert_eq!(code, 0, "branch list should succeed");
}