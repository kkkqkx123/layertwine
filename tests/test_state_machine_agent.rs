mod common;

use stratum::core::delta::Delta;
use stratum::core::file_node::FileNode;
use stratum::core::partition::Partition;
use stratum::core::snapshot::Snapshot;
use stratum::core::types::{AgentInstanceId, PartitionType, SnapshotId, SourceType};
use stratum::state_machine::agent;
use stratum::state_machine::approval;
use stratum::state_machine::transition::reconstruct_text;
use stratum::storage::repository::{DeltaStore, FileNodeStore, PartitionStore, SnapshotStore};
use stratum::storage::sqlite_storage::SqliteStorage;

fn setup_snapshot(storage: &SqliteStorage, content: &str) -> SnapshotId {
    let file_node = FileNode::new(std::path::PathBuf::from("test.txt"), content.as_bytes());
    storage.store_file_node(&file_node, content.as_bytes()).unwrap();
    let empty_diff = stratum::core::delta::LineDiff::new(vec![]);
    let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Agent("setup".into()));
    storage.store_delta(&delta).unwrap();
    let snap = Snapshot::new_initial(file_node, delta.id);
    storage.store_snapshot(&snap, b"").unwrap();
    snap.id
}

// AG-01: Single Agent Edit — partition advances
#[test]
fn test_single_agent_edit_advances_partition() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("agent-1".into());
    let init_id = setup_snapshot(&storage, "base\n");
    agent::ensure_agent_partition(&storage, &agent_id, init_id).unwrap();

    let new_id = agent::apply_agent_edit(&storage, &agent_id, "test.txt", "base\nedited\n").unwrap();
    let part = storage.get_partition(&agent::agent_partition_id(&agent_id)).unwrap();
    assert_eq!(part.current_snapshot, new_id);
    assert_ne!(new_id, init_id);
}

// AG-02: Agent isolation — two agents edit same file independently
#[test]
fn test_agent_isolation_two_agents() {
    let storage = common::create_storage();
    let init_id = setup_snapshot(&storage, "shared\n");
    let agent_a = AgentInstanceId("agent-a".into());
    let agent_b = AgentInstanceId("agent-b".into());
    agent::ensure_agent_partition(&storage, &agent_a, init_id).unwrap();
    agent::ensure_agent_partition(&storage, &agent_b, init_id).unwrap();

    let a_id = agent::apply_agent_edit(&storage, &agent_a, "test.txt", "shared\na\n").unwrap();
    let b_id = agent::apply_agent_edit(&storage, &agent_b, "test.txt", "shared\nb\n").unwrap();
    assert_ne!(a_id, b_id);

    let pa = storage.get_partition(&agent::agent_partition_id(&agent_a)).unwrap();
    let pb = storage.get_partition(&agent::agent_partition_id(&agent_b)).unwrap();
    assert_eq!(pa.current_snapshot, a_id);
    assert_eq!(pb.current_snapshot, b_id);
}

// AG-05: Agent edit with no changes returns current snapshot
#[test]
fn test_agent_edit_no_change() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("agent-5".into());
    let init_id = setup_snapshot(&storage, "same");
    agent::ensure_agent_partition(&storage, &agent_id, init_id).unwrap();

    let result = agent::apply_agent_edit(&storage, &agent_id, "test.txt", "same").unwrap();
    assert_eq!(result, init_id);
}

// AG-04: Discard agent edit rolls back to parent
#[test]
fn test_discard_agent_edit() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("agent-4".into());
    let init_id = setup_snapshot(&storage, "original\n");
    agent::ensure_agent_partition(&storage, &agent_id, init_id).unwrap();
    let edited_id = agent::apply_agent_edit(&storage, &agent_id, "test.txt", "original\nchanged\n").unwrap();
    assert_ne!(edited_id, init_id);

    agent::discard_agent_edit(&storage, &agent_id).unwrap();
    let part = storage.get_partition(&agent::agent_partition_id(&agent_id)).unwrap();
    assert_eq!(part.current_snapshot, init_id);
}

// AG-05: Discard fails when no parent exists
#[test]
fn test_discard_agent_edit_no_parent() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("agent-5".into());
    let init_id = setup_snapshot(&storage, "only\n");
    agent::ensure_agent_partition(&storage, &agent_id, init_id).unwrap();
    let result = agent::discard_agent_edit(&storage, &agent_id);
    assert!(result.is_err());
}

// AG-06: Move agent edit to approval advances approval partition
#[test]
fn test_move_agent_to_approval() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("agent-6".into());
    let init_id = setup_snapshot(&storage, "base\n");
    agent::ensure_agent_partition(&storage, &agent_id, init_id).unwrap();

    let approval_pid = approval::approval_agent_partition_id(&agent_id);
    let approval_part = Partition {
        id: approval_pid,
        name: format!("approval/{}", agent_id),
        current_snapshot: init_id,
        history: vec![init_id],
        partition_type: PartitionType::Approval(agent_id.clone()),
    };
    storage.create_partition(&approval_part).unwrap();

    agent::apply_agent_edit(&storage, &agent_id, "test.txt", "base\nagent\n").unwrap();
    let merged_id = agent::move_agent_to_approval(&storage, &agent_id).unwrap();
    let approval_part = storage.get_partition(&approval_pid).unwrap();
    assert_eq!(approval_part.current_snapshot, merged_id);
    assert_ne!(merged_id, init_id);
}

// AG-07: Verify content after move_agent_to_approval
#[test]
fn test_move_agent_to_approval_content() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("agent-7".into());
    let init_id = setup_snapshot(&storage, "line1\nline2\n");
    agent::ensure_agent_partition(&storage, &agent_id, init_id).unwrap();

    let approval_pid = approval::approval_agent_partition_id(&agent_id);
    let approval_part = Partition {
        id: approval_pid,
        name: format!("approval/{}", agent_id),
        current_snapshot: init_id,
        history: vec![init_id],
        partition_type: PartitionType::Approval(agent_id.clone()),
    };
    storage.create_partition(&approval_part).unwrap();

    agent::apply_agent_edit(&storage, &agent_id, "test.txt", "line1\nline2\nagent_line\n").unwrap();
    let merged_id = agent::move_agent_to_approval(&storage, &agent_id).unwrap();
    let merged_snap = storage.get_snapshot(&merged_id).unwrap();
    let text = reconstruct_text(&storage, &merged_snap).unwrap();
    assert_eq!(text, "line1\nline2\nagent_line");
}

// AG-08: Move with no changes returns current approval snapshot
#[test]
fn test_move_agent_to_approval_no_change() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("agent-8".into());
    let init_id = setup_snapshot(&storage, "same\n");
    agent::ensure_agent_partition(&storage, &agent_id, init_id).unwrap();

    let approval_pid = approval::approval_agent_partition_id(&agent_id);
    let approval_part = Partition {
        id: approval_pid,
        name: format!("approval/{}", agent_id),
        current_snapshot: init_id,
        history: vec![init_id],
        partition_type: PartitionType::Approval(agent_id.clone()),
    };
    storage.create_partition(&approval_part).unwrap();

    let result = agent::move_agent_to_approval(&storage, &agent_id).unwrap();
    assert_eq!(result, init_id);
}

// AG-09: Sequential edits accumulate correctly
#[test]
fn test_agent_sequential_edits() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("agent-9".into());
    let init_id = setup_snapshot(&storage, "a\nb\n");
    agent::ensure_agent_partition(&storage, &agent_id, init_id).unwrap();

    let v1 = agent::apply_agent_edit(&storage, &agent_id, "test.txt", "a\nmodified\n").unwrap();
    let v2 = agent::apply_agent_edit(&storage, &agent_id, "test.txt", "a\nmodified\nc\n").unwrap();
    assert_ne!(v1, v2);

    let part = storage.get_partition(&agent::agent_partition_id(&agent_id)).unwrap();
    assert_eq!(part.current_snapshot, v2);
    assert_eq!(part.history.len(), 3); // init + v1 + v2
}

// AG-10: Agent partition is created once and reused
#[test]
fn test_agent_partition_idempotent() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("agent-10".into());
    let init_id = setup_snapshot(&storage, "data\n");
    let p1 = agent::ensure_agent_partition(&storage, &agent_id, init_id).unwrap();
    let p2 = agent::ensure_agent_partition(&storage, &agent_id, init_id).unwrap();
    assert_eq!(p1.id, p2.id);
}