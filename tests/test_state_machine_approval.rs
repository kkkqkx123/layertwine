mod common;

use stratum::core::partition::Partition;
use stratum::core::snapshot::Snapshot;
use stratum::core::types::{AgentInstanceId, PartitionType, SnapshotId};
use stratum::state_machine::approval;
use stratum::state_machine::transition::reconstruct_text;
use stratum::storage::repository::{DeltaStore, PartitionStore, SnapshotStore};
use stratum::storage::sqlite_storage::SqliteStorage;

fn setup_approval_snapshot(
    storage: &SqliteStorage,
    content: &str,
) -> SnapshotId {
    common::create_initial_snapshot(storage, "test.txt", content)
}

fn create_approval_partition(
    storage: &SqliteStorage,
    agent_id: &AgentInstanceId,
    init_id: SnapshotId,
) {
    let pid = approval::approval_agent_partition_id(agent_id);
    let part = Partition {
        id: pid,
        name: format!("approval/{}", agent_id),
        current_snapshot: init_id,
        history: vec![init_id],
        partition_type: PartitionType::Approval(agent_id.clone()),
    };
    storage.create_partition(&part).unwrap();
}

fn create_integrated_partition(
    storage: &SqliteStorage,
    name: &str,
    init_id: SnapshotId,
) {
    let pid = approval::integrated_partition_id(name);
    let part = Partition {
        id: pid,
        name: format!("integrated/{}", name),
        current_snapshot: init_id,
        history: vec![init_id],
        partition_type: PartitionType::Integrated(name.to_string()),
    };
    storage.create_partition(&part).unwrap();
}

// AP-01: Ensure approval agent partition idempotent
#[test]
fn test_ensure_approval_agent_partition() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("ap-test".into());
    let init_id = setup_approval_snapshot(&storage, "base\n");
    let p1 = approval::ensure_approval_agent_partition(&storage, &agent_id, init_id).unwrap();
    let p2 = approval::ensure_approval_agent_partition(&storage, &agent_id, init_id).unwrap();
    assert_eq!(p1.id, p2.id);
}

// AP-02: Ensure integrated partition idempotent
#[test]
fn test_ensure_integrated_partition() {
    let storage = common::create_storage();
    let init_id = setup_approval_snapshot(&storage, "base\n");
    let p1 = approval::ensure_integrated_partition(&storage, "feat-1", init_id).unwrap();
    let p2 = approval::ensure_integrated_partition(&storage, "feat-1", init_id).unwrap();
    assert_eq!(p1.id, p2.id);
    let p3 = approval::ensure_integrated_partition(&storage, "feat-2", init_id).unwrap();
    assert_ne!(p1.id, p3.id);
}

// AP-03: Ensure unified partition singleton
#[test]
fn test_ensure_unified_partition() {
    let storage = common::create_storage();
    let init_id = setup_approval_snapshot(&storage, "base\n");
    let p1 = approval::ensure_unified_partition(&storage, init_id).unwrap();
    let p2 = approval::ensure_unified_partition(&storage, init_id).unwrap();
    assert_eq!(p1.id, p2.id);
}

// AP-04: Move approval to integrated advances integrated pointer
#[test]
fn test_move_approval_to_integrated() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("ap-agent".into());
    let init_id = setup_approval_snapshot(&storage, "base\n");
    create_approval_partition(&storage, &agent_id, init_id);
    create_integrated_partition(&storage, "feat-int", init_id);

    // Advance approval with new content
    let snap = storage.get_snapshot(&init_id).unwrap();
    let delta = common::make_insert_delta(&snap.file, "modified");
    storage.store_delta(&delta).unwrap();
    let new_snap = Snapshot::apply_delta(&snap, delta.id);
    storage.store_snapshot(&new_snap, b"").unwrap();
    let approval_pid = approval::approval_agent_partition_id(&agent_id);
    storage.update_pointer(&approval_pid, &new_snap.id).unwrap();

    let result = approval::move_approval_to_integrated(&storage, &agent_id, "feat-int").unwrap();
    let integrated = storage.get_partition(&approval::integrated_partition_id("feat-int")).unwrap();
    assert_eq!(integrated.current_snapshot, result);

    let integrated_snap = storage.get_snapshot(&result).unwrap();
    let text = reconstruct_text(&storage, &integrated_snap).unwrap();
    assert_eq!(text, "modified\nbase");
}

// AP-05: Move approval with no changes returns current
#[test]
fn test_move_approval_to_integrated_no_change() {
    let storage = common::create_storage();
    let agent_id = AgentInstanceId("ap-nochange".into());
    let init_id = setup_approval_snapshot(&storage, "same\n");
    create_approval_partition(&storage, &agent_id, init_id);
    create_integrated_partition(&storage, "feat-nc", init_id);
    let result = approval::move_approval_to_integrated(&storage, &agent_id, "feat-nc").unwrap();
    assert_eq!(result, init_id);
}

// AP-06: Move integrated to unifies with two inputs
#[test]
fn test_move_integrated_to_unified_two_sources() {
    let storage = common::create_storage();
    let init_id = setup_approval_snapshot(&storage, "base\n");
    approval::ensure_unified_partition(&storage, init_id).unwrap();

    let names = vec!["int-1", "int-2"];
    for name in &names {
        create_integrated_partition(&storage, name, init_id);
        let snap = storage.get_snapshot(&init_id).unwrap();
        let delta = common::make_insert_delta(&snap.file, &format!("from-{}\n", name));
        storage.store_delta(&delta).unwrap();
        let new_snap = Snapshot::apply_delta(&snap, delta.id);
        storage.store_snapshot(&new_snap, b"").unwrap();
        let pid = approval::integrated_partition_id(name);
        storage.update_pointer(&pid, &new_snap.id).unwrap();
    }

    let name_refs: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    let result = approval::move_integrated_to_unified(&storage, &name_refs).unwrap();
    let unified = storage.get_partition(&approval::unified_partition_id()).unwrap();
    assert_eq!(unified.current_snapshot, result);
    assert_ne!(result, init_id);
}

// AP-07: Migrate between partitions switches pointers
#[test]
fn test_migrate_between_partitions() {
    let storage = common::create_storage();
    let init_id = setup_approval_snapshot(&storage, "base\n");
    let agent_id = AgentInstanceId("migrate-agent".into());

    create_approval_partition(&storage, &agent_id, init_id);
    create_integrated_partition(&storage, "migrate-target", init_id);

    // Advance approval
    let snap = storage.get_snapshot(&init_id).unwrap();
    let new_snap = Snapshot::apply_delta(
        &snap,
        common::make_insert_delta(&snap.file, "migrated").id,
    );
    storage.store_snapshot(&new_snap, b"").unwrap();
    let approval_pid = approval::approval_agent_partition_id(&agent_id);
    storage.update_pointer(&approval_pid, &new_snap.id).unwrap();

    let int_pid = approval::integrated_partition_id("migrate-target");
    approval::migrate_between_partitions(&storage, &approval_pid, &int_pid).unwrap();
    let integrated = storage.get_partition(&int_pid).unwrap();
    assert_eq!(integrated.current_snapshot, new_snap.id);
}

// AP-08: Multi-step approval → integrated → staged full pipeline
#[test]
fn test_approval_to_staged_pipeline() {
    let storage = common::create_storage();
    let init_id = setup_approval_snapshot(&storage, "base\n");
    let agent_id = AgentInstanceId("pipeline-agent".into());
    create_approval_partition(&storage, &agent_id, init_id);
    create_integrated_partition(&storage, "pipeline-int", init_id);

    // Advance approval with content
    let snap = storage.get_snapshot(&init_id).unwrap();
    let delta = common::make_insert_delta(&snap.file, "pipeline\n");
    storage.store_delta(&delta).unwrap();
    let new_snap = Snapshot::apply_delta(&snap, delta.id);
    storage.store_snapshot(&new_snap, b"").unwrap();
    let approval_pid = approval::approval_agent_partition_id(&agent_id);
    storage.update_pointer(&approval_pid, &new_snap.id).unwrap();

    // approval → integrated
    let _int_id = approval::move_approval_to_integrated(&storage, &agent_id, "pipeline-int").unwrap();

    // integrated → unified
    approval::ensure_unified_partition(&storage, init_id).unwrap();
    let names = vec!["pipeline-int".to_string()];
    let unified_id = approval::move_integrated_to_unified(&storage, &names).unwrap();
    assert_ne!(unified_id, init_id);

    // unified → staged
    let staged_pid = stratum::state_machine::staged::staged_partition_id();
    stratum::state_machine::staged::ensure_staged_partition(&storage, unified_id).unwrap();
    approval::migrate_between_partitions(&storage, &approval::unified_partition_id(), &staged_pid).unwrap();
    let staged = storage.get_partition(&staged_pid).unwrap();
    assert_eq!(staged.current_snapshot, unified_id);
}

// AP-09: Partition ID uniqueness across agent/integrated
#[test]
fn test_approval_partition_id_uniqueness() {
    let a1 = AgentInstanceId("agent-x".into());
    let a2 = AgentInstanceId("agent-y".into());
    assert_ne!(
        approval::approval_agent_partition_id(&a1),
        approval::approval_agent_partition_id(&a2)
    );
    assert_ne!(
        approval::integrated_partition_id("feat-a"),
        approval::integrated_partition_id("feat-b")
    );
}

// AP-10: Ensure unified partition is singleton
#[test]
fn test_unified_partition_singleton() {
    let storage = common::create_storage();
    let init_id = setup_approval_snapshot(&storage, "data\n");
    let p1 = approval::ensure_unified_partition(&storage, init_id).unwrap();
    let p2 = approval::ensure_unified_partition(&storage, init_id).unwrap();
    assert_eq!(p1.id, p2.id);
    assert_eq!(p1.name, "unified");
}