mod common;

use stratum::checkpoint::branch::Branch;
use stratum::checkpoint::checkpoint::{Checkpoint, CheckpointMetadata};
use stratum::checkpoint::dag::CheckpointDag;
use stratum::checkpoint::repo::CheckpointRepo;
use stratum::core::types::{ContentId, SnapshotId};
use stratum::storage::repository::{BranchStore, CheckpointStore, DagStore};

// CK-01: Create checkpoint repo and verify root checkpoint exists
#[test]
fn test_checkpoint_repo_creation() {
    let sid = SnapshotId::from_content(b"root-snapshot");
    let repo = CheckpointRepo::new_single(sid);
    assert_eq!(repo.branches.len(), 1);
    assert_eq!(repo.branches[0].name, "main");
    let root_id = repo.current_branch_head();
    assert!(repo.checkpoint_count() >= 1);
    let root = repo.get_checkpoint(&root_id).unwrap();
    assert!(root.parents.is_empty());
    assert!(root.baseline_snapshots.contains(&sid));
}

// CK-02: Commit snapshot creates new checkpoint with correct chain
#[test]
fn test_checkpoint_commit() {
    let mut repo = CheckpointRepo::new_single(ContentId::from_content(b"v0"));
    let v1 = ContentId::from_content(b"v1");
    let cp_id = repo.commit_single(v1, "first commit", "tester").unwrap();
    assert!(repo.checkpoint_count() >= 2);
    let cp = repo.get_checkpoint(&cp_id).unwrap();
    assert_eq!(cp.metadata.message, "first commit");
    assert_eq!(cp.metadata.author, "tester");
    assert_eq!(cp.parents.len(), 1);
    assert_eq!(cp.baseline_snapshots, vec![v1]);
    assert_eq!(repo.current_branch_head(), cp_id);
}

// CK-03: Multiple commits form linear DAG
#[test]
fn test_checkpoint_linear_chain() {
    let mut repo = CheckpointRepo::new_single(ContentId::from_content(b"root"));
    let mut prev = repo.current_branch_head();
    for i in 0..5 {
        let snap = ContentId::from_content(format!("v{}", i).as_bytes());
        let cp_id = repo.commit_single(snap, &format!("commit {}", i), "dev").unwrap();
        assert_ne!(cp_id, prev);
        let cp = repo.get_checkpoint(&cp_id).unwrap();
        assert_eq!(cp.parents, vec![prev]);
        prev = cp_id;
    }
    // 6 nodes: root + 5 commits
    assert_eq!(repo.dag().len(), 6);
}

// CK-04: Branch creation and switching
#[test]
fn test_checkpoint_branch_create_and_switch() {
    let mut repo = CheckpointRepo::new_single(ContentId::from_content(b"root"));
    let main_head = repo.current_branch_head();

    repo.create_branch("feature").unwrap();
    assert_eq!(repo.branches.len(), 2);
    assert_eq!(repo.branches[1].name, "feature");
    assert_eq!(repo.branches[1].head, main_head);

    repo.current_branch = 1;
    let cp_id = repo
        .commit_single(ContentId::from_content(b"feature-snap"), "feat", "dev")
        .unwrap();
    assert_eq!(repo.current_branch_head(), cp_id);
    assert_eq!(repo.branches[0].head, main_head);
}

// CK-05: Duplicate branch name returns error
#[test]
fn test_checkpoint_duplicate_branch() {
    let mut repo = CheckpointRepo::new_single(ContentId::from_content(b"root"));
    let result = repo.create_branch("main");
    assert!(result.is_err());
}

// CK-06: Checkpoint DAG edge tracking
#[test]
fn test_checkpoint_dag_edges() {
    let mut repo = CheckpointRepo::new_single(ContentId::from_content(b"root"));
    let root_id = repo.current_branch_head();
    let c1 = repo.commit_single(ContentId::from_content(b"s1"), "c1", "u").unwrap();
    let c2 = repo.commit_single(ContentId::from_content(b"s2"), "c2", "u").unwrap();

    assert!(repo.dag().get_children(&root_id).contains(&c1));
    assert!(repo.dag().get_children(&c1).contains(&c2));
    assert!(!repo.dag().get_children(&root_id).contains(&c2));
}

// CK-07: Checkpoint storage persistence (full storage)
#[test]
fn test_checkpoint_storage_persistence() {
    let storage = common::create_full_storage();
    let sid = common::create_initial_snapshot(&storage, "f.txt", "content");

    let cp = Checkpoint::new(
        vec![sid],
        vec![],
        CheckpointMetadata::new("author", "msg"),
    );
    storage.store_checkpoint(&cp).unwrap();
    assert!(storage.checkpoint_exists(&cp.id).unwrap());

    let retrieved = storage.get_checkpoint(&cp.id).unwrap();
    assert_eq!(retrieved.id, cp.id);
    assert_eq!(retrieved.metadata.message, "msg");
}

// CK-08: Branch storage roundtrip
#[test]
fn test_branch_storage_roundtrip() {
    let storage = common::create_full_storage();
    let cp_id = ContentId::from_content(b"branch-cp");
    let branch = Branch::new("develop", cp_id);
    storage.store_branch(&branch).unwrap();
    let retrieved = storage.get_branch("develop").unwrap();
    assert_eq!(retrieved.name, "develop");
    assert_eq!(retrieved.head, cp_id);
}

// CK-09: Branch head update
#[test]
fn test_branch_head_update() {
    let storage = common::create_full_storage();
    let cp1 = ContentId::from_content(b"head1");
    let branch = Branch::new("feature", cp1);
    storage.store_branch(&branch).unwrap();
    let cp2 = ContentId::from_content(b"head2");
    storage.update_branch_head("feature", &cp2).unwrap();
    let updated = storage.get_branch("feature").unwrap();
    assert_eq!(updated.head, cp2);
}

// CK-10: DAG storage roundtrip
#[test]
fn test_dag_storage_roundtrip() {
    let storage = common::create_full_storage();
    let mut dag = CheckpointDag::new();
    let n1 = ContentId::from_content(b"node1");
    let n2 = ContentId::from_content(b"node2");
    dag.add_node(n1);
    dag.add_node(n2);
    dag.add_edge(n1, n2);
    storage.store_dag(&dag).unwrap();
    let loaded = storage.load_dag().unwrap();
    assert!(!loaded.is_empty());
    assert!(loaded.is_ancestor(&n1, &n2));
}