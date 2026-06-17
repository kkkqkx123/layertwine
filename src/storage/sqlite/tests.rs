use crate::checkpoint::checkpoint::Checkpoint;
use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{
    CheckpointId, ContentId, DiffOp, Hunk, PartitionType, SnapshotId, SourceType,
};
use crate::storage::repository::{
    BranchStore, CheckpointStore, DeltaStore, FileNodeStore, PartitionStore,
    SnapshotStore,
};
use crate::storage::sqlite::connection::SqliteStorage;
use crate::StorageResult;
use rusqlite::Connection;
use std::path::PathBuf;

fn create_test_storage() -> SqliteStorage {
    SqliteStorage::new_in_memory().unwrap()
}

fn create_test_file_node(path: &str, content: &[u8]) -> FileNode {
    FileNode::new(PathBuf::from(path), content)
}

fn create_test_delta(file: &FileNode) -> Delta {
    let hunk = Hunk {
        old_start: 1,
        old_len: 0,
        new_start: 1,
        new_len: 1,
        ops: vec![DiffOp::Insert {
            new_start: 1,
            lines: vec!["new line".to_string()],
        }],
    };
    let diff = crate::core::types::LineDiff::new(vec![hunk]);
    Delta::new(file.clone(), diff, SourceType::Manual)
}

#[test]
fn test_store_and_get_snapshot() {
    let storage = create_test_storage();
    let file = create_test_file_node("test.txt", b"original content");
    storage.store_file_node(&file, b"original content").unwrap();

    let delta = create_test_delta(&file);
    storage.store_delta(&delta).unwrap();

    let snapshot = Snapshot::new_initial(file, delta.id);
    storage.store_snapshot(&snapshot, b"").unwrap();

    let retrieved = storage.get_snapshot(&snapshot.id).unwrap();
    assert_eq!(retrieved.id, snapshot.id);
    assert_eq!(retrieved.deltas.len(), 1);
    assert_eq!(retrieved.deltas[0], delta.id);
}

#[test]
fn test_store_and_get_delta() {
    let storage = create_test_storage();
    let file = create_test_file_node("test.txt", b"content");
    let delta = create_test_delta(&file);

    storage.store_delta(&delta).unwrap();
    let retrieved = storage.get_delta(&delta.id).unwrap();

    assert_eq!(retrieved.id, delta.id);
    assert_eq!(retrieved.file.path_str(), "test.txt");
}

#[test]
fn test_file_node_roundtrip() {
    let storage = create_test_storage();
    let file = create_test_file_node("hello.txt", b"hello world");

    storage.store_file_node(&file, b"hello world").unwrap();
    assert!(storage
        .file_node_exists(file.path_str(), &file.base_hash)
        .unwrap());

    let content = storage
        .get_file_content(file.path_str(), &file.base_hash)
        .unwrap();
    assert_eq!(content, b"hello world");
}

#[test]
fn test_partition_crud() {
    let storage = create_test_storage();
    let file = create_test_file_node("test.txt", b"initial");
    storage.store_file_node(&file, b"initial").unwrap();

    let delta = create_test_delta(&file);
    storage.store_delta(&delta).unwrap();

    let snapshot = Snapshot::new_initial(file, delta.id);
    storage.store_snapshot(&snapshot, b"").unwrap();

    let partition = Partition::new(
        "test_partition".to_string(),
        PartitionType::Manual,
        snapshot.id,
    );
    storage.create_partition(&partition).unwrap();

    let retrieved = storage.get_partition(&partition.id).unwrap();
    assert_eq!(retrieved.name, "test_partition");
    assert_eq!(retrieved.current_snapshot, snapshot.id);
    assert_eq!(retrieved.history.len(), 1);

    let by_name = storage.get_partition_by_name("test_partition").unwrap();
    assert_eq!(by_name.id, partition.id);

    let snapshot2 = Snapshot::from_parent(&snapshot, delta.id, "manual".to_string());
    storage.store_snapshot(&snapshot2, b"").unwrap();
    storage
        .update_pointer(&partition.id, &snapshot2.id)
        .unwrap();

    let updated = storage.get_partition(&partition.id).unwrap();
    assert_eq!(updated.current_snapshot, snapshot2.id);
    assert_eq!(updated.history.len(), 2);
}

#[test]
fn test_list_partitions() {
    let storage = create_test_storage();
    let file = create_test_file_node("test.txt", b"content");
    storage.store_file_node(&file, b"content").unwrap();
    let delta = create_test_delta(&file);
    let snapshot = Snapshot::new_initial(file, delta.id);
    storage.store_delta(&delta).unwrap();
    storage.store_snapshot(&snapshot, b"").unwrap();

    let p1 = Partition::new("p1".to_string(), PartitionType::Manual, snapshot.id);
    let p2 = Partition::new(
        "p2".to_string(),
        PartitionType::Agent("agent1".into()),
        snapshot.id,
    );

    storage.create_partition(&p1).unwrap();
    storage.create_partition(&p2).unwrap();

    let list = storage.list_partitions().unwrap();
    assert_eq!(list.len(), 2);
}

#[test]
fn test_delta_exists() {
    let storage = create_test_storage();
    let file = create_test_file_node("test.txt", b"content");
    let delta = create_test_delta(&file);

    assert!(!storage.delta_exists(&delta.id).unwrap());
    storage.store_delta(&delta).unwrap();
    assert!(storage.delta_exists(&delta.id).unwrap());
}

#[test]
fn test_snapshot_exists() {
    let storage = create_test_storage();
    let file = create_test_file_node("test.txt", b"content");
    storage.store_file_node(&file, b"content").unwrap();

    let delta = create_test_delta(&file);
    storage.store_delta(&delta).unwrap();

    let snapshot = Snapshot::new_initial(file, delta.id);
    assert!(!storage.snapshot_exists(&snapshot.id).unwrap());
    storage.store_snapshot(&snapshot, b"").unwrap();
    assert!(storage.snapshot_exists(&snapshot.id).unwrap());
}

#[test]
fn test_find_snapshots_by_file() {
    let storage = create_test_storage();
    let file = create_test_file_node("test.txt", b"content");
    storage.store_file_node(&file, b"content").unwrap();

    let delta = create_test_delta(&file);
    storage.store_delta(&delta).unwrap();

    let s1 = Snapshot::new_initial(file.clone(), delta.id);
    storage.store_snapshot(&s1, b"").unwrap();

    let s2 = Snapshot::from_parent(&s1, delta.id, "manual".to_string());
    storage.store_snapshot(&s2, b"").unwrap();

    let found = storage.find_snapshots_by_file("test.txt").unwrap();
    assert_eq!(found.len(), 2);
}

#[test]
fn test_transaction_rollback() {
    let storage = create_test_storage();

    let result: StorageResult<()> = storage.with_transaction(|conn| {
        conn.execute("INSERT INTO layers (layer_type, partition_ids, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["test_layer", b"[]", 1000, 1000])?;

        Err(crate::StorageError::Database(
            rusqlite::Error::InvalidParameterName("rollback test".to_string()),
        ))
    });

    assert!(result.is_err());

    let conn = storage.conn.lock();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM layers WHERE layer_type = ?1",
            rusqlite::params!["test_layer"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_partition_advance_and_rollback() {
    let storage = create_test_storage();
    let file = create_test_file_node("test.txt", b"content");
    let delta = create_test_delta(&file);

    storage.store_file_node(&file, b"content").unwrap();
    storage.store_delta(&delta).unwrap();

    let s1 = Snapshot::new_initial(file.clone(), delta.id);
    let s2 = Snapshot::from_parent(&s1, delta.id, "manual".to_string());
    let s3 = Snapshot::from_parent(&s2, delta.id, "manual".to_string());

    storage.store_snapshot(&s1, b"").unwrap();
    storage.store_snapshot(&s2, b"").unwrap();
    storage.store_snapshot(&s3, b"").unwrap();

    let mut partition = Partition::new("rollback_test".to_string(), PartitionType::Manual, s1.id);
    assert_eq!(partition.history.len(), 1);

    partition.advance(s2.id);
    assert_eq!(partition.history.len(), 2);
    assert_eq!(partition.current_snapshot, s2.id);

    partition.advance(s3.id);
    assert_eq!(partition.history.len(), 3);

    let prev = partition.rollback_one();
    assert_eq!(prev, Some(s2.id));
    assert_eq!(partition.current_snapshot, s2.id);
    assert_eq!(partition.history.len(), 2);

    assert!(partition.rollback_to(&s1.id));
    assert_eq!(partition.current_snapshot, s1.id);
    assert_eq!(partition.history.len(), 1);
}

#[test]
fn test_find_snapshots_by_partition() {
    let storage = create_test_storage();
    let file = create_test_file_node("multi.txt", b"multi");
    storage.store_file_node(&file, b"multi").unwrap();
    let delta = create_test_delta(&file);
    storage.store_delta(&delta).unwrap();

    let base = Snapshot::new_initial(file.clone(), delta.id);
    storage.store_snapshot(&base, b"").unwrap();

    let s1 = Snapshot::from_parent(&base, delta.id, PartitionType::Manual.name());
    storage.store_snapshot(&s1, b"").unwrap();

    let s2 = Snapshot::from_parent(
        &base,
        delta.id,
        PartitionType::Agent("agent_test".into()).name(),
    );
    storage.store_snapshot(&s2, b"").unwrap();

    let manual_snapshots = storage
        .find_snapshots_by_partition(&PartitionType::Manual)
        .unwrap();
    assert_eq!(manual_snapshots.len(), 1, "should find 1 manual snapshot");

    let agent_snapshots = storage
        .find_snapshots_by_partition(&PartitionType::Agent("agent_test".into()))
        .unwrap();
    assert_eq!(
        agent_snapshots.len(),
        1,
        "should find 1 agent_test snapshot"
    );
}

#[test]
fn test_store_and_get_deltas_batch() {
    let storage = create_test_storage();

    let file1 = create_test_file_node("f1.txt", b"content1");
    let delta1 = create_test_delta(&file1);
    storage.store_delta(&delta1).unwrap();

    let file2 = create_test_file_node("f2.txt", b"content2");
    let delta2 = create_test_delta(&file2);
    storage.store_delta(&delta2).unwrap();

    let deltas = storage.get_deltas(&[delta1.id, delta2.id]).unwrap();
    assert_eq!(deltas.len(), 2, "should retrieve both deltas");
    let ids: Vec<_> = deltas.iter().map(|d| d.id).collect();
    assert!(ids.contains(&delta1.id));
    assert!(ids.contains(&delta2.id));
}

fn create_full_storage() -> SqliteStorage {
    let conn = Connection::open_in_memory().unwrap();
    crate::storage::migrations::initialize_full(&conn).unwrap();
    let conn = std::sync::Arc::new(parking_lot::ReentrantMutex::new(conn));
    SqliteStorage::new_with_connection_arc(&conn)
}

fn make_checkpoint_id(data: &[u8]) -> CheckpointId {
    ContentId::from_content(data)
}

fn make_snapshot_id(data: &[u8]) -> SnapshotId {
    ContentId::from_content(data)
}

#[test]
fn test_checkpoint_store_roundtrip() {
    let storage = create_full_storage();
    let snap_id = make_snapshot_id(b"snap1");
    let cp = Checkpoint::new(
        vec![snap_id],
        vec![],
        crate::checkpoint::checkpoint::CheckpointMetadata::new("author1", "msg1"),
    );

    storage.store_checkpoint(&cp).unwrap();
    assert!(storage.checkpoint_exists(&cp.id).unwrap());

    let retrieved = storage.get_checkpoint(&cp.id).unwrap();
    assert_eq!(retrieved.metadata.author, "author1");
    assert_eq!(retrieved.metadata.message, "msg1");
    assert_eq!(retrieved.baseline_snapshots, vec![snap_id]);
}

#[test]
fn test_checkpoint_list_and_delete() {
    let storage = create_full_storage();

    let cp1 = Checkpoint::new(
        vec![make_snapshot_id(b"a")],
        vec![],
        crate::checkpoint::checkpoint::CheckpointMetadata::new("author1", "first"),
    );
    let cp2 = Checkpoint::new(
        vec![make_snapshot_id(b"b")],
        vec![cp1.id],
        crate::checkpoint::checkpoint::CheckpointMetadata::new("author2", "second"),
    );

    storage.store_checkpoint(&cp1).unwrap();
    storage.store_checkpoint(&cp2).unwrap();

    let list = storage.list_checkpoints().unwrap();
    assert_eq!(list.len(), 2, "should list 2 checkpoints");

    storage.delete_checkpoint(&cp1.id).unwrap();
    assert!(!storage.checkpoint_exists(&cp1.id).unwrap());
    assert!(storage.checkpoint_exists(&cp2.id).unwrap());
}

#[test]
fn test_delete_checkpoint_not_found() {
    let storage = create_full_storage();
    let fake_id = make_checkpoint_id(b"nonexistent");
    let result = storage.delete_checkpoint(&fake_id);
    assert!(
        result.is_err(),
        "deleting non-existent checkpoint should fail"
    );
}

#[test]
fn test_checkpoint_exists_false() {
    let storage = create_full_storage();
    let fake_id = make_checkpoint_id(b"nope");
    assert!(!storage.checkpoint_exists(&fake_id).unwrap());
}

#[test]
fn test_branch_store_roundtrip() {
    let storage = create_full_storage();
    let cp_id = make_checkpoint_id(b"branch-root");
    let branch = crate::checkpoint::branch::Branch::new("main", cp_id);

    storage.store_branch(&branch).unwrap();

    let retrieved = storage.get_branch("main").unwrap();
    assert_eq!(retrieved.name, "main");
    assert_eq!(retrieved.head, cp_id);
}

#[test]
fn test_branch_update_head() {
    let storage = create_full_storage();
    let cp1 = make_checkpoint_id(b"head1");
    let branch = crate::checkpoint::branch::Branch::new("feature", cp1);
    storage.store_branch(&branch).unwrap();

    let cp2 = make_checkpoint_id(b"head2");
    storage.update_branch_head("feature", &cp2).unwrap();

    let updated = storage.get_branch("feature").unwrap();
    assert_eq!(updated.head, cp2);
}

#[test]
fn test_branch_list_and_delete() {
    let storage = create_full_storage();
    let cp_id = make_checkpoint_id(b"root");

    let b1 = crate::checkpoint::branch::Branch::new("main", cp_id);
    let b2 = crate::checkpoint::branch::Branch::new("develop", cp_id);
    storage.store_branch(&b1).unwrap();
    storage.store_branch(&b2).unwrap();

    let list = storage.list_branches().unwrap();
    assert_eq!(list.len(), 2);

    storage.delete_branch("develop").unwrap();
    let list = storage.list_branches().unwrap();
    assert_eq!(list.len(), 1);
}

#[test]
fn test_sqlite_storage_repeated_snapshot_ops() {
    let storage = create_test_storage();
    let file = create_test_file_node("stratum.txt", b"stratum content");
    storage.store_file_node(&file, b"stratum content").unwrap();

    assert!(storage
        .file_node_exists(file.path_str(), &file.base_hash)
        .unwrap());

    let delta = create_test_delta(&file);
    storage.store_delta(&delta).unwrap();

    let snapshot = Snapshot::new_initial(file, delta.id);
    storage.store_snapshot(&snapshot, b"").unwrap();

    let retrieved = storage.get_snapshot(&snapshot.id).unwrap();
    assert_eq!(retrieved.id, snapshot.id);
}

#[test]
fn test_sqlite_storage_repeated_partition_ops() {
    let storage = create_test_storage();
    let file = create_test_file_node("sp.txt", b"sp");
    storage.store_file_node(&file, b"sp").unwrap();
    let delta = create_test_delta(&file);
    storage.store_delta(&delta).unwrap();
    let snapshot = Snapshot::new_initial(file, delta.id);
    storage.store_snapshot(&snapshot, b"").unwrap();

    let partition = Partition::new(
        "stratum-partition".to_string(),
        PartitionType::Manual,
        snapshot.id,
    );
    storage.create_partition(&partition).unwrap();

    let retrieved = storage.get_partition(&partition.id).unwrap();
    assert_eq!(retrieved.name, "stratum-partition");
}
