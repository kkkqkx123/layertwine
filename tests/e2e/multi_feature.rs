//! Multi-feature E2E tests
//!
//! Tests workflows that combine multiple features

use crate::common;

/// E2E-MULTI-01: Branch merge workflow
///
/// Tests complete branch and merge workflow:
///   1. Initialize and make initial commit on main
///   2. Create feature branch
///   3. Switch to feature and make edits
///   4. Commit feature changes
///   5. Switch back to main
///   6. Merge feature into main
#[test]
fn test_branch_merge_workflow() {
    let fx = common::E2eFixture::new();

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_init());
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "main base\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_commit("base", "user"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_create("feature"));
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_switch("feature"));
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_edit("f.txt", "feature edit\n"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_commit("feature work", "user"),
    );
    assert_eq!(code, 0);

    let code = common::run_cmd(fx.db_path_str(), common::cli::cmd_branch_switch("main"));
    assert_eq!(code, 0);

    let code = common::run_cmd(
        fx.db_path_str(),
        common::cli::cmd_merge("feature", "merge feature"),
    );
    assert_eq!(code, 0, "merge should succeed");
}
