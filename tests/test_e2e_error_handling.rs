mod common;

use common::e2e;

/// E2E-ER-01: Run commands without initializing
///
/// Steps:
///   1. edit on uninitialized db -> should fail
///   2. commit on uninitialized db -> should fail
///   3. status on uninitialized db -> should succeed (opens new empty db)
///   4. log on empty db -> should succeed (returns empty list)
#[test]
fn test_commands_without_init() {
    let fx = e2e::E2eFixture::new();

    // Edit on uninitialized db should fail
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_edit("f.txt", "content\n"));
    assert_ne!(code, 0, "edit without init should fail");

    // Commit on uninitialized db should fail
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("test", "user"));
    assert_ne!(code, 0, "commit without init should fail");

    // Log on empty db should succeed (no checkpoints)
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_log());
    assert_eq!(code, 0, "log on empty db should succeed");

    // Status on empty db should succeed (no partitions)
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_status());
    assert_eq!(code, 0, "status on empty db should succeed");
}

/// E2E-ER-02: Invalid agent operations
///
/// Steps:
///   1. init
///   2. approve non-existent agent -> should fail
///   3. submit for non-existent agent -> should fail
#[test]
fn test_invalid_agent_operations() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    // Approve non-existent agent
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_approve("ghost"));
    assert_ne!(code, 0, "approve non-existent agent should fail");

    // Submit for non-existent agent (no edits made)
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_submit("ghost2"));
    assert_ne!(code, 0, "submit non-existent agent should fail");
}

/// E2E-ER-03: Merge without branches
///
/// Steps:
///   1. init
///   2. merge non-existent branch -> should fail
#[test]
fn test_merge_nonexistent_branch() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_merge("nonexistent", "test"));
    assert_ne!(code, 0, "merge non-existent branch should fail");
}

/// E2E-ER-04: Garbage collection on empty repo
///
/// Steps:
///   1. init
///   2. gc -> should succeed (no-op)
#[test]
fn test_gc_on_empty_repo() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), cmd_gc());
    assert_eq!(code, 0, "gc should succeed on empty repo");
}

/// E2E-ER-05: Push without git repo path
///
/// Steps:
///   1. init
///   2. push -> should fail because --git-repo is required
#[test]
fn test_push_without_git_repo() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    // Push requires --git-repo, which is None by default
    let code = e2e::run_cmd(fx.db_path_str(), cmd_push());
    assert_ne!(code, 0, "push without --git-repo should fail");
}

/// E2E-ER-06: Pull without git repo path
///
/// Steps:
///   1. init
///   2. pull -> should fail because --git-repo is required
#[test]
fn test_pull_without_git_repo() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), cmd_pull());
    assert_ne!(code, 0, "pull without --git-repo should fail");
}

/// E2E-ER-07: Agent edit on non-existent file path
///
/// Steps:
///   1. init
///   2. agent-x edit a file with very long name
///   3. Should succeed (non-existent files are created)
#[test]
fn test_agent_edit_unknown_file() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_edit("agent-z", "/unknown/path/file.txt", "content\n"));
    assert_eq!(code, 0, "agent edit on new path should succeed");
}

// ===== Helper command builders for push/pull/gc =====

fn cmd_push() -> stratum::cli::commands::Commands {
    stratum::cli::commands::Commands::Push {
        remote: "origin".to_string(),
        message: "test".to_string(),
    }
}

fn cmd_pull() -> stratum::cli::commands::Commands {
    stratum::cli::commands::Commands::Pull {
        remote: "origin".to_string(),
        git_ref: "HEAD".to_string(),
    }
}

fn cmd_gc() -> stratum::cli::commands::Commands {
    stratum::cli::commands::Commands::Gc
}