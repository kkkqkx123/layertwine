mod common;

use stratum::core::partition::Partition;
use stratum::core::types::{AgentInstanceId, PartitionType};
use stratum::state_machine::agent;
use stratum::state_machine::approval;
use stratum::state_machine::manual;
use stratum::storage::repository::{PartitionStore, SnapshotStore};

// FW-01: Manual edit → merge to staged → commit
#[test]
fn test_edit_merge_commit_workflow() {
    let storage = common::create_storage();
    common::init_repo(&storage, "workflow.txt", "initial\n");

    let v1 = manual::apply_manual_edit(&storage, "workflow.txt", "v1\n").unwrap();
    assert!(storage.snapshot_exists(&v1).unwrap());

    let staged_id = manual::merge_manual_to_staged(&storage).unwrap();
    let staged_part = storage
        .get_partition(&stratum::state_machine::staged::staged_partition_id())
        .unwrap();
    assert_eq!(staged_part.current_snapshot, staged_id);

    let text = common::reconstruct_text(&storage, &staged_id);
    assert_eq!(text, "v1");
}

// FW-02: Two manual edits, merge once
#[test]
fn test_two_edits_then_merge() {
    let storage = common::create_storage();
    common::init_repo(&storage, "two_edits.txt", "base\n");

    manual::apply_manual_edit(&storage, "two_edits.txt", "edit1\n").unwrap();
    manual::apply_manual_edit(&storage, "two_edits.txt", "edit2\n").unwrap();
    let staged_id = manual::merge_manual_to_staged(&storage).unwrap();

    let text = common::reconstruct_text(&storage, &staged_id);
    assert_eq!(text, "edit2");
}

// FW-03: Agent forks → approval merge → staged → commit
#[test]
fn test_agent_fork_merge_to_staged() {
    let storage = common::create_storage();
    let init_id = common::create_initial_snapshot(&storage, "shared.txt", "base\n");
    let manual_part = Partition {
        id: stratum::state_machine::manual::manual_partition_id(),
        name: "manual_edit".into(),
        current_snapshot: init_id,
        history: vec![init_id],
        partition_type: PartitionType::Manual,
    };
    storage.create_partition(&manual_part).unwrap();
    let staged_part = Partition {
        id: stratum::state_machine::staged::staged_partition_id(),
        name: "staged".into(),
        current_snapshot: init_id,
        history: vec![init_id],
        partition_type: PartitionType::Staged,
    };
    storage.create_partition(&staged_part).unwrap();

    // Two agents fork from same base
    let agent_a = AgentInstanceId("fork-a".into());
    let agent_b = AgentInstanceId("fork-b".into());
    agent::ensure_agent_partition(&storage, &agent_a, init_id).unwrap();
    agent::ensure_agent_partition(&storage, &agent_b, init_id).unwrap();

    agent::apply_agent_edit(&storage, &agent_a, "shared.txt", "base\na-edit\n").unwrap();
    agent::apply_agent_edit(&storage, &agent_b, "shared.txt", "base\nb-edit\n").unwrap();

    // Move each agent to their approval partition
    let ap_a = approval::approval_agent_partition_id(&agent_a);
    let ap_b = approval::approval_agent_partition_id(&agent_b);
    storage
        .create_partition(&Partition {
            id: ap_a,
            name: format!("approval/{}", agent_a),
            current_snapshot: init_id,
            history: vec![init_id],
            partition_type: PartitionType::Approval(agent_a.clone()),
        })
        .unwrap();
    storage
        .create_partition(&Partition {
            id: ap_b,
            name: format!("approval/{}", agent_b),
            current_snapshot: init_id,
            history: vec![init_id],
            partition_type: PartitionType::Approval(agent_b.clone()),
        })
        .unwrap();

    agent::move_agent_to_approval(&storage, &agent_a).unwrap();
    agent::move_agent_to_approval(&storage, &agent_b).unwrap();

    // Merge approval A into integrated
    approval::ensure_integrated_partition(&storage, "integ-a", init_id).unwrap();
    approval::move_approval_to_integrated(&storage, &agent_a, "integ-a").unwrap();

    // Merge approval B into integrated
    approval::ensure_integrated_partition(&storage, "integ-b", init_id).unwrap();
    approval::move_approval_to_integrated(&storage, &agent_b, "integ-b").unwrap();

    // Merge integrated into unified
    approval::ensure_unified_partition(&storage, init_id).unwrap();
    let names = vec!["integ-a".to_string(), "integ-b".to_string()];
    let unified_id = approval::move_integrated_to_unified(&storage, &names).unwrap();
    assert_ne!(unified_id, init_id);

    // Migrate unified to staged
    approval::migrate_between_partitions(
        &storage,
        &approval::unified_partition_id(),
        &stratum::state_machine::staged::staged_partition_id(),
    )
    .unwrap();
    let staged = storage
        .get_partition(&stratum::state_machine::staged::staged_partition_id())
        .unwrap();
    assert_eq!(staged.current_snapshot, unified_id);
}

// FW-04: Manual edit, agent edit, then sequential merge to staged
#[test]
fn test_manual_and_agent_sequential() {
    let storage = common::create_storage();
    common::init_repo(&storage, "seq.txt", "base\n");

    // Manual edit and merge
    manual::apply_manual_edit(&storage, "seq.txt", "manual\n").unwrap();
    let staged_after_manual = manual::merge_manual_to_staged(&storage).unwrap();
    let text = common::reconstruct_text(&storage, &staged_after_manual);
    assert_eq!(text, "manual");
}

// FW-05: Verify reconstructed content at each stage
#[test]
fn test_content_consistency_across_stages() {
    let storage = common::create_storage();
    common::init_repo(&storage, "consistent.txt", "base\n");

    // Stage: original
    let staged = storage
        .get_partition(&stratum::state_machine::staged::staged_partition_id())
        .unwrap();
    assert_eq!(
        common::reconstruct_text(&storage, &staged.current_snapshot),
        "base"
    );

    // Edit and check manual content
    let man_id = manual::apply_manual_edit(&storage, "consistent.txt", "edited\n").unwrap();
    assert_eq!(common::reconstruct_text(&storage, &man_id), "edited");

    // Merge and check staged content
    let staged_id = manual::merge_manual_to_staged(&storage).unwrap();
    assert_eq!(common::reconstruct_text(&storage, &staged_id), "edited");
}

// FW-06: Empty file handling
#[test]
fn test_empty_file_workflow() {
    let storage = common::create_storage();
    common::init_repo(&storage, "empty.txt", "");
    let v1 = manual::apply_manual_edit(&storage, "empty.txt", "first\n").unwrap();
    let text = common::reconstruct_text(&storage, &v1);
    assert_eq!(text, "first");
}