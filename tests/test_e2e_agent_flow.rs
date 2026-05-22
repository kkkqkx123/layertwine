mod common;

use common::e2e;

/// E2E-AG-01: Single agent edit, submit, and commit (skipping approve)
///
/// Note: The CLI's `approve` step requires integrated/unified partitions
/// which are not auto-created in the current implementation.
/// This test covers the working edit -> submit path.
///
/// Steps:
///   1. init
///   2. agent agent-a edit f.txt -c "agent content\n"
///   3. agent agent-a submit
///   4. commit staged changes
#[test]
fn test_single_agent_edit_and_submit() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_edit("agent-a", "f.txt", "agent content\n"));
    assert_eq!(code, 0, "agent edit should succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_submit("agent-a"));
    assert_eq!(code, 0, "agent submit should succeed");
}

/// E2E-AG-02: Full agent flow with approve
///
/// Steps:
///   1. init
///   2. agent edit -> submit (via CLI)
///   3. approve (via CLI) — auto-creates integrated/unified partitions
///   4. commit (via CLI)
#[test]
fn test_full_agent_flow_with_partitions() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_edit("agent-b", "f.txt", "agent edit\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_submit("agent-b"));
    assert_eq!(code, 0);

    // Approve now auto-creates integrated/unified partitions
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_approve("agent-b"));
    assert_eq!(code, 0, "approve should succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("agent-b changes", "user"));
    assert_eq!(code, 0, "commit should succeed");
}

/// E2E-AG-03: Dual agent parallel editing of the same file
///
/// Steps:
///   1. init
///   2. agent alpha edit f.txt
///   3. agent beta edit f.txt
///   4. Both edits should succeed
///   5. Approve both (auto-creates integrated/unified partitions)
#[test]
fn test_dual_agent_parallel_edit() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_edit("alpha", "f.txt", "alpha-line\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_edit("beta", "f.txt", "beta-line\n"));
    assert_eq!(code, 0);

    // Submit both
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_submit("alpha"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_submit("beta"));
    assert_eq!(code, 0);

    // Approve both (integrated/unified partitions auto-created)
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_approve("alpha"));
    assert_eq!(code, 0, "approve alpha should succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_approve("beta"));
    assert_eq!(code, 0, "approve beta should succeed");
}

/// E2E-AG-04: Agent edit with empty content
///
/// Steps:
///   1. init
///   2. agent agent-c edit f.txt (empty content)
///   3. submit should succeed
#[test]
fn test_agent_edit_empty_content() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_edit("agent-c", "f.txt", ""));
    assert_eq!(code, 0, "agent edit with empty content should succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_submit("agent-c"));
    assert_eq!(code, 0, "submit after empty edit should succeed");
}

/// E2E-AG-05: Agent submit without prior edit
///
/// Steps:
///   1. init
///   2. agent ghost submit (no edits made)
///   3. Should handle gracefully
#[test]
fn test_agent_submit_without_edit() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_submit("ghost"));
    assert!(code == 0 || code == 1, "submit without edit should handle gracefully");
}

/// E2E-AG-06: Approve now auto-creates integrated/unified partitions
///
/// This test verifies that the approve command no longer requires
/// manually ensuring partitions — they are auto-created.
#[test]
fn test_approve_auto_creates_partitions() {
    let fx = e2e::E2eFixture::new();

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_init());
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_edit("agent-x", "f.txt", "content\n"));
    assert_eq!(code, 0);

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_agent_submit("agent-x"));
    assert_eq!(code, 0);

    // Approve should now succeed because integrated/unified are auto-created
    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_approve("agent-x"));
    assert_eq!(code, 0, "approve should auto-create partitions and succeed");

    let code = e2e::run_cmd(fx.db_path_str(), e2e::cmd_commit("approved changes", "user"));
    assert_eq!(code, 0, "commit after approve should succeed");
}