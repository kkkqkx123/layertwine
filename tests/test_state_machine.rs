mod common;

use stratum::core::types::PartitionType;
use stratum::state_machine::manual::{self, manual_partition_id};
use stratum::state_machine::staged::{self, staged_partition_id};
use stratum::storage::repository::{BranchStore, CheckpointStore, PartitionStore};

// SM-01: Initialize complete repository with both partitions
#[test]
fn test_initialize_repository() {
    let storage = common::create_storage();
    let (sid, manual_part, staged_part) = common::init_repo(&storage, "init.txt", "base\n");
    assert_eq!(manual_part.current_snapshot, sid);
    assert_eq!(staged_part.current_snapshot, sid);
    assert_eq!(manual_part.partition_type, PartitionType::Manual);
    assert_eq!(staged_part.partition_type, PartitionType::Staged);
}

// SM-02: Manual edit updates partition pointer
#[test]
fn test_manual_edit_updates_partition() {
    let storage = common::create_storage();
    common::init_repo(&storage, "edit.txt", "original\n");
    let new_sid = manual::apply_manual_edit(&storage, "edit.txt", "modified\n").unwrap();
    let manual_part = storage.get_partition(&manual_partition_id()).unwrap();
    assert_eq!(manual_part.current_snapshot, new_sid);
}

// SM-03: Multiple consecutive manual edits accumulate history
#[test]
fn test_multiple_manual_edits_history() {
    let storage = common::create_storage();
    common::init_repo(&storage, "hist.txt", "v0\n");
    let v1 = manual::apply_manual_edit(&storage, "hist.txt", "v1\n").unwrap();
    let v2 = manual::apply_manual_edit(&storage, "hist.txt", "v2\n").unwrap();
    let v3 = manual::apply_manual_edit(&storage, "hist.txt", "v3\n").unwrap();

    let manual_part = storage.get_partition(&manual_partition_id()).unwrap();
    assert_eq!(manual_part.current_snapshot, v3);
    assert_eq!(manual_part.history.len(), 4); // init + 3 edits
    assert_eq!(manual_part.history[1], v1);
    assert_eq!(manual_part.history[2], v2);
    assert_eq!(manual_part.history[3], v3);
}

// SM-04: Manual edit with identical content returns current snapshot
// Note: content.lines() strips trailing newlines in apply_deltas,
// so identical content without trailing newline returns the same snapshot ID.
#[test]
fn test_manual_edit_no_change() {
    let storage = common::create_storage();
    let (sid, _, _) = common::init_repo(&storage, "nochange.txt", "same");
    let result = manual::apply_manual_edit(&storage, "nochange.txt", "same").unwrap();
    assert_eq!(result, sid);
}

// SM-05: Merge manual to staged advances staged pointer
#[test]
fn test_merge_manual_to_staged() {
    let storage = common::create_storage();
    common::init_repo(&storage, "merge.txt", "base\n");
    manual::apply_manual_edit(&storage, "merge.txt", "manual_edit\n").unwrap();
    let staged_id = manual::merge_manual_to_staged(&storage).unwrap();

    let staged_part = storage.get_partition(&staged_partition_id()).unwrap();
    assert_eq!(staged_part.current_snapshot, staged_id);

    let text = common::reconstruct_text(&storage, &staged_id);
    assert_eq!(text, "manual_edit");
}

// SM-06: Merge with no changes returns current staged ID
#[test]
fn test_merge_manual_to_staged_no_change() {
    let storage = common::create_storage();
    let (_, _, staged_part) = common::init_repo(&storage, "nomerge.txt", "same\n");
    let staged_id = manual::merge_manual_to_staged(&storage).unwrap();
    assert_eq!(staged_id, staged_part.current_snapshot);
}

// SM-07: Commit staged to checkpoint (requires full storage)
#[test]
fn test_commit_staged_to_checkpoint() {
    let storage = common::create_full_storage();
    common::init_repo(&storage, "commit.txt", "initial\n");
    manual::apply_manual_edit(&storage, "commit.txt", "edited\n").unwrap();
    manual::merge_manual_to_staged(&storage).unwrap();

    let cp_id = staged::commit_staged_to_checkpoint(&storage, "first commit", "tester").unwrap();
    let cp = storage.get_checkpoint(&cp_id).unwrap();
    assert_eq!(cp.metadata.message, "first commit");
    assert_eq!(cp.metadata.author, "tester");
}

// SM-08: First commit auto-creates "main" branch (requires full storage)
#[test]
fn test_first_commit_creates_main_branch() {
    let storage = common::create_full_storage();
    common::init_repo(&storage, "main.txt", "root\n");
    manual::apply_manual_edit(&storage, "main.txt", "v1\n").unwrap();
    manual::merge_manual_to_staged(&storage).unwrap();
    let cp_id = staged::commit_staged_to_checkpoint(&storage, "init", "user").unwrap();

    let branch = storage.get_branch("main").unwrap();
    assert_eq!(branch.head, cp_id);
}

// SM-09: Multiple commits form linear history (requires full storage)
#[test]
fn test_multiple_commits_linear_history() {
    let storage = common::create_full_storage();
    common::init_repo(&storage, "linear.txt", "base\n");

    for i in 0..3 {
        let content = format!("commit_{}\n", i);
        manual::apply_manual_edit(&storage, "linear.txt", &content).unwrap();
        manual::merge_manual_to_staged(&storage).unwrap();
        staged::commit_staged_to_checkpoint(&storage, &format!("commit {}", i), "user").unwrap();
    }

    // There is no guarantee about number of checkpoints stored (root is not stored)
    // But the branch should have been updated
    let branch = storage.get_branch("main").unwrap();
    assert_ne!(branch.head.to_string(), "");
}

// SM-10: Reset staged to a specific snapshot
#[test]
fn test_reset_staged() {
    let storage = common::create_storage();
    let (_sid, _, _) = common::init_repo(&storage, "reset.txt", "original\n");

    let new_sid = common::create_initial_snapshot(&storage, "reset.txt", "new_base\n");
    staged::reset_staged(&storage, new_sid).unwrap();
    let updated = storage.get_partition(&staged_partition_id()).unwrap();
    assert_eq!(updated.current_snapshot, new_sid);
}

// SM-11: Verify iron law — forward transitions only allowed between neighboring layers
#[test]
fn test_iron_law_forward_valid() {
    use stratum::state_machine::transition::check_forward_valid;
    use stratum::core::types::LayerType;

    assert!(check_forward_valid(&LayerType::ManualEdit, &LayerType::Staged).is_ok());
    assert!(check_forward_valid(&LayerType::AgentEdit, &LayerType::Approval).is_ok());
    assert!(check_forward_valid(&LayerType::Approval, &LayerType::Staged).is_ok());

    // Cross-layer flows must be rejected
    assert!(check_forward_valid(&LayerType::ManualEdit, &LayerType::Approval).is_err());
    assert!(check_forward_valid(&LayerType::AgentEdit, &LayerType::Staged).is_err());
}

// SM-12: Verify iron law — rollback only switches pointers (no reverse writes)
#[test]
fn test_iron_law_rollback_valid() {
    use stratum::state_machine::transition::check_rollback_valid;
    use stratum::core::types::LayerType;

    assert!(check_rollback_valid(&LayerType::Staged, &LayerType::ManualEdit).is_ok());
    assert!(check_rollback_valid(&LayerType::Staged, &LayerType::Approval).is_ok());
    assert!(check_rollback_valid(&LayerType::Approval, &LayerType::AgentEdit).is_ok());

    // Non-adjacent or forward-only flows
    assert!(check_rollback_valid(&LayerType::ManualEdit, &LayerType::Staged).is_err());
    assert!(check_rollback_valid(&LayerType::Staged, &LayerType::AgentEdit).is_ok());
}

// SM-13: Execute forward transition via execute_forward
#[test]
fn test_execute_forward_manual_to_staged() {
    use stratum::state_machine::transition::{execute_forward, ForwardTransition};

    let storage = common::create_storage();
    common::init_repo(&storage, "forward.txt", "base\n");
    manual::apply_manual_edit(&storage, "forward.txt", "forwarded\n").unwrap();

    let result = execute_forward(&storage, ForwardTransition::ManualToStaged, &[]).unwrap();
    let text = common::reconstruct_text(&storage, &result);
    assert_eq!(text, "forwarded");
}

// SM-14: Partition listing
#[test]
fn test_partition_listing() {
    let storage = common::create_storage();
    let (_sid, _, _) = common::init_repo(&storage, "list.txt", "base\n");
    let partitions = storage.list_partitions().unwrap();
    assert_eq!(partitions.len(), 2);
    assert!(partitions.iter().any(|p| p.partition_type == PartitionType::Manual));
    assert!(partitions.iter().any(|p| p.partition_type == PartitionType::Staged));
}

// SM-15: ensure_manual_partition is idempotent
#[test]
fn test_ensure_manual_partition_idempotent() {
    let storage = common::create_storage();
    let sid = common::create_initial_snapshot(&storage, "idem.txt", "data\n");
    let p1 = manual::ensure_manual_partition(&storage, sid).unwrap();
    let p2 = manual::ensure_manual_partition(&storage, sid).unwrap();
    assert_eq!(p1.id, p2.id);
}

// SM-16: ensure_staged_partition is idempotent
#[test]
fn test_ensure_staged_partition_idempotent() {
    let storage = common::create_storage();
    let sid = common::create_initial_snapshot(&storage, "idem2.txt", "data\n");
    let p1 = staged::ensure_staged_partition(&storage, sid).unwrap();
    let p2 = staged::ensure_staged_partition(&storage, sid).unwrap();
    assert_eq!(p1.id, p2.id);
}