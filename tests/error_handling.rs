//! Error handling integration tests
//!
//! Tests error scenarios and edge cases

use crate::common;

use stratum::api::cli::commands::Commands;

/// INT-ER-01: Run commands without initializing
///
/// Tests that commands fail appropriately when database is not initialized
#[test]
fn test_commands_without_init() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "content\n"),
    );
    assert_ne!(code, 0, "edit without init should fail");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_commit("test", "user"));
    assert_ne!(code, 0, "commit without init should fail");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_log());
    assert_eq!(code, 0, "log on empty db should succeed");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_status());
    assert_eq!(code, 0, "status on empty db should succeed");
}

/// INT-ER-02: Invalid agent operations
///
/// Tests that invalid agent operations are rejected
#[test]
fn test_invalid_agent_operations() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_approve("ghost"));
    assert_ne!(code, 0, "approve non-existent agent should fail");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_agent_submit("ghost2"));
    assert_ne!(code, 0, "submit non-existent agent should fail");
}

/// INT-ER-03: Merge without branches
///
/// Tests that merging non-existent branches fails
#[test]
fn test_merge_nonexistent_branch() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_merge("nonexistent", "test"),
    );
    assert_ne!(code, 0, "merge non-existent branch should fail");
}

/// INT-ER-04: Garbage collection on empty repo
///
/// Tests that GC works on empty repository
#[test]
fn test_gc_on_empty_repo() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), cmd_gc());
    assert_eq!(code, 0, "gc should succeed on empty repo");
}

/// INT-ER-05: Push without git repo path
///
/// Tests that push fails without git repo configuration
#[test]
fn test_push_without_git_repo() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), cmd_push());
    assert_ne!(code, 0, "push without --git-repo should fail");
}

/// INT-ER-06: Pull without git repo path
///
/// Tests that pull fails without git repo configuration
#[test]
fn test_pull_without_git_repo() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), cmd_pull());
    assert_ne!(code, 0, "pull without --git-repo should fail");
}

/// INT-ER-07: Agent edit on non-existent file path
///
/// Tests that agent can edit new files
#[test]
fn test_agent_edit_unknown_file() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_agent_edit("agent-z", "/unknown/path/file.txt", "content\n"),
    );
    assert_eq!(code, 0, "agent edit on new path should succeed");
}

fn cmd_push() -> Commands {
    Commands::Push {
        remote: "origin".to_string(),
        message: "test".to_string(),
    }
}

fn cmd_pull() -> Commands {
    Commands::Pull {
        remote: "origin".to_string(),
        git_ref: "HEAD".to_string(),
    }
}

fn cmd_gc() -> Commands {
    Commands::Gc
}
