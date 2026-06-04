//! Agent operations integration tests
//!
//! Tests individual agent management operations

use crate::common;

/// INT-AG-01: Single agent edit, submit, and commit (skipping approve)
///
/// Tests basic agent edit and submit workflow
#[test]
fn test_single_agent_edit_and_submit() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_agent_edit("agent-a", "f.txt", "agent content\n"),
    );
    assert_eq!(code, 0, "agent edit should succeed");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_agent_submit("agent-a"));
    assert_eq!(code, 0, "agent submit should succeed");
}

/// INT-AG-04: Agent edit with empty content
///
/// Tests that agent can edit files with empty content
#[test]
fn test_agent_edit_empty_content() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_agent_edit("agent-c", "f.txt", ""),
    );
    assert_eq!(code, 0, "agent edit with empty content should succeed");

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_agent_submit("agent-c"));
    assert_eq!(code, 0, "submit after empty edit should succeed");
}

/// INT-AG-05: Agent submit without prior edit
///
/// Tests that submit without edit handles gracefully
#[test]
fn test_agent_submit_without_edit() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_agent_submit("ghost"));
    assert!(
        code == 0 || code == 1,
        "submit without edit should handle gracefully"
    );
}

/// INT-AG-06: Approve auto-creates integrated/unified partitions
///
/// Tests that approve command auto-creates required partitions
#[test]
fn test_approve_auto_creates_partitions() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_agent_edit("agent-x", "f.txt", "content\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_agent_submit("agent-x"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_approve("agent-x"));
    assert_eq!(code, 0, "approve should auto-create partitions and succeed");

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_commit("approved changes", "user"),
    );
    assert_eq!(code, 0, "commit after approve should succeed");
}
