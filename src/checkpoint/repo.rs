use crate::checkpoint::branch::Branch;
use crate::checkpoint::checkpoint::{Checkpoint, CheckpointMetadata};
use crate::checkpoint::dag::CheckpointDag;
use crate::core::types::{CheckpointId, SnapshotId};
use crate::error::{Result, StratumError};
use crate::storage::repository::CheckpointPersist;
use std::collections::{HashMap, HashSet, VecDeque};

/// Checkpoint Repository - Versioning Core
///
/// Git-independent versioning, managing checkpoint commits, branches, and DAG history.
/// When `storage` is set, all mutations auto-persist to the backend.
pub struct CheckpointRepo {
    /// All branches
    pub branches: Vec<Branch>,
    /// Current branch index
    pub current_branch: usize,
    /// Checkpoint DAG
    pub checkpoint_dag: CheckpointDag,
    /// All checkpoints (ID → Checkpoint)
    pub(crate) checkpoints: HashMap<CheckpointId, Checkpoint>,
    /// Optional persistence backend — when set, mutations auto-persist
    pub(crate) storage: Option<Box<dyn CheckpointPersist>>,
}

impl CheckpointRepo {
    /// Create a new checkpoint repository with multi-file initialization support
    pub fn new(initial_snapshots: Vec<SnapshotId>) -> Self {
        let metadata = CheckpointMetadata::new("system", "root checkpoint");
        let root = Checkpoint::new(initial_snapshots, vec![], metadata);
        let root_id = root.id;

        let mut dag = CheckpointDag::new();
        dag.add_node(root_id);

        let mut checkpoints = HashMap::new();
        checkpoints.insert(root_id, root);

        let main_branch = Branch::new("main", root_id);

        CheckpointRepo {
            branches: vec![main_branch],
            current_branch: 0,
            checkpoint_dag: dag,
            checkpoints,
            storage: None,
        }
    }

    /// Convenient single-snapshot compatible construction
    pub fn new_single(initial_snapshot: SnapshotId) -> Self {
        CheckpointRepo::new(vec![initial_snapshot])
    }

    /// Load checkpoint repository from persistent storage.
    ///
    /// If the database is empty, a root checkpoint + "main" branch are created automatically.
    /// All subsequent mutations will auto-persist through the provided storage backend.
    pub fn load(storage: Box<dyn CheckpointPersist>) -> Result<Self> {
        let mut checkpoint_dag = storage.load_dag()?;
        let mut checkpoints: HashMap<CheckpointId, Checkpoint> = storage
            .list_checkpoints()?
            .into_iter()
            .map(|cp| (cp.id, cp))
            .collect();
        let mut branches = storage.list_branches()?;

        // Initialize with root when storage is empty
        if checkpoints.is_empty() {
            let metadata = CheckpointMetadata::new("system", "root checkpoint");
            let root = Checkpoint::new(vec![CheckpointId::from_content(&[])], vec![], metadata);
            let root_id = root.id;

            storage.store_checkpoint(&root)?;
            checkpoints.insert(root_id, root);

            checkpoint_dag = CheckpointDag::new();
            checkpoint_dag.add_node(root_id);
            storage.store_dag(&checkpoint_dag)?;

            let main_branch = Branch::new("main", root_id);
            storage.store_branch(&main_branch)?;
            branches.push(main_branch);

            storage.store_metadata("current_branch", "main")?;
        }

        let current_branch_name = storage
            .load_metadata("current_branch")?
            .unwrap_or_else(|| "main".to_string());
        let current_branch = branches
            .iter()
            .position(|b| b.name == current_branch_name)
            .unwrap_or(0);

        Ok(CheckpointRepo {
            branches,
            current_branch,
            checkpoint_dag,
            checkpoints,
            storage: Some(storage),
        })
    }

    /// Attach a persistence backend to an existing in-memory repo.
    /// Subsequent mutations will auto-persist.
    pub fn attach_storage(&mut self, storage: Box<dyn CheckpointPersist>) {
        self.storage = Some(storage);
    }

    /// Persist a specific checkpoint to storage (used after in-memory metadata changes).
    pub fn sync_checkpoint(&self, cp_id: &CheckpointId) -> Result<()> {
        if let Some(storage) = &self.storage {
            if let Some(cp) = self.checkpoints.get(cp_id) {
                storage.store_checkpoint(cp)?;
            }
        }
        Ok(())
    }

    /// Persist the full current state (all checkpoints, DAG, branches, current_branch metadata).
    pub fn sync_all(&self) -> Result<()> {
        if let Some(storage) = &self.storage {
            for cp in self.checkpoints.values() {
                storage.store_checkpoint(cp)?;
            }
            storage.store_dag(&self.checkpoint_dag)?;
            for branch in &self.branches {
                storage.store_branch(branch)?;
            }
            storage.store_metadata("current_branch", &self.branches[self.current_branch].name)?;
        }
        Ok(())
    }

    // Checkpoint operations -

    /// Getting the specified checkpoints
    pub fn get_checkpoint(&self, id: &CheckpointId) -> Result<&Checkpoint> {
        self.checkpoints
            .get(id)
            .ok_or_else(|| StratumError::NotFound(format!("checkpoint {} not found", id)))
    }

    /// Getting variable references
    pub fn get_checkpoint_mut(&mut self, id: &CheckpointId) -> Result<&mut Checkpoint> {
        self.checkpoints
            .get_mut(id)
            .ok_or_else(|| StratumError::NotFound(format!("checkpoint {} not found", id)))
    }

    /// Commit: Packages the current state of staged as a Checkpoint.
    ///
    /// 1. Create a new Checkpoint (supports multi-file snapshots)
    /// 2. Add to DAG
    /// 3. Update branch head
    /// 4. Auto-persist to storage if backend is attached
    pub fn commit(
        &mut self,
        snapshot_ids: Vec<SnapshotId>,
        message: &str,
        author: &str,
    ) -> Result<CheckpointId> {
        if snapshot_ids.is_empty() {
            return Err(StratumError::Checkpoint(
                "cannot commit with empty snapshot list".to_string(),
            ));
        }
        let current_head = self.current_branch_head();
        let metadata = CheckpointMetadata::new(author, message);
        let cp = Checkpoint::new(snapshot_ids, vec![current_head], metadata);
        let cp_id = cp.id;

        self.checkpoints.insert(cp_id, cp);
        self.checkpoint_dag.add_node(cp_id);
        self.checkpoint_dag.add_edge(current_head, cp_id);
        self.current_branch_mut().set_head(cp_id);

        // Auto-persist
        if let Some(storage) = &self.storage {
            if let Some(cp) = self.checkpoints.get(&cp_id) {
                storage.store_checkpoint(cp)?;
            }
            storage.store_dag(&self.checkpoint_dag)?;
            let branch = &self.branches[self.current_branch];
            storage.update_branch_head(&branch.name, &cp_id)?;
        }

        Ok(cp_id)
    }

    /// Compatible with single snapshot submission
    pub fn commit_single(
        &mut self,
        snapshot_id: SnapshotId,
        message: &str,
        author: &str,
    ) -> Result<CheckpointId> {
        self.commit(vec![snapshot_id], message, author)
    }

    // Branching out.

    /// Get the head Checkpoint ID of the current branch
    pub fn current_branch_head(&self) -> CheckpointId {
        self.branches[self.current_branch].head
    }

    /// Get a mutable reference to the current branch
    pub fn current_branch_mut(&mut self) -> &mut Branch {
        &mut self.branches[self.current_branch]
    }

    /// Get current branch name
    pub fn current_branch_name(&self) -> &str {
        &self.branches[self.current_branch].name
    }

    /// Create a new branch based on the current head.
    /// Auto-persists the new branch to storage.
    pub fn create_branch(&mut self, name: &str) -> Result<()> {
        if self.branches.iter().any(|b| b.name == name) {
            return Err(StratumError::Checkpoint(format!(
                "branch '{}' already exists",
                name
            )));
        }
        let head = self.current_branch_head();
        let branch = Branch::new(name, head);
        if let Some(storage) = &self.storage {
            storage.store_branch(&branch)?;
        }
        self.branches.push(branch);
        Ok(())
    }

    /// Create a new branch on the specified checkpoint.
    /// Auto-persists the new branch to storage.
    pub fn create_branch_from(&mut self, name: &str, from_checkpoint: CheckpointId) -> Result<()> {
        if !self.checkpoints.contains_key(&from_checkpoint) {
            return Err(StratumError::NotFound(format!(
                "checkpoint {} not found",
                from_checkpoint
            )));
        }
        if self.branches.iter().any(|b| b.name == name) {
            return Err(StratumError::Checkpoint(format!(
                "branch '{}' already exists",
                name
            )));
        }
        let branch = Branch::new(name, from_checkpoint);
        if let Some(storage) = &self.storage {
            storage.store_branch(&branch)?;
        }
        self.branches.push(branch);
        Ok(())
    }

    /// Switching Branches
    /// Auto-persists the current branch name to storage.
    pub fn switch_branch(&mut self, name: &str) -> Result<usize> {
        let idx = self
            .branches
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| StratumError::NotFound(format!("branch '{}' not found", name)))?;

        self.current_branch = idx;
        if let Some(storage) = &self.storage {
            storage.store_metadata("current_branch", name)?;
        }
        Ok(idx)
    }

    /// List all branches
    pub fn list_branches(&self) -> &[Branch] {
        &self.branches
    }

    /// Find Branch Index
    pub fn find_branch(&self, name: &str) -> Result<usize> {
        self.branches
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| StratumError::NotFound(format!("branch '{}' not found", name)))
    }

    /// Get the head of the specified branch
    pub fn get_branch_head(&self, name: &str) -> Result<CheckpointId> {
        let idx = self.find_branch(name)?;
        Ok(self.branches[idx].head)
    }

    /// Merge branch: Merge source_branch into the current branch.
    ///
    /// Generate multiple parent Checkpoints to add to the DAG.
    /// Auto-persists the new checkpoint, DAG, and updated branch head.
    pub fn merge_branches(
        &mut self,
        source_branch: &str,
        snapshot_ids: Vec<SnapshotId>,
        message: &str,
        author: &str,
    ) -> Result<CheckpointId> {
        if snapshot_ids.is_empty() {
            return Err(StratumError::Checkpoint(
                "cannot merge with empty snapshot list".to_string(),
            ));
        }
        let source_head = self.get_branch_head(source_branch)?;
        let current_head = self.current_branch_head();

        let metadata = CheckpointMetadata::new(author, message);
        let cp = Checkpoint::new(snapshot_ids, vec![current_head, source_head], metadata);
        let cp_id = cp.id;

        self.checkpoints.insert(cp_id, cp);
        self.checkpoint_dag.add_node(cp_id);
        self.checkpoint_dag.add_edge(current_head, cp_id);
        self.checkpoint_dag.add_edge(source_head, cp_id);
        self.current_branch_mut().set_head(cp_id);

        // Auto-persist
        if let Some(storage) = &self.storage {
            if let Some(cp) = self.checkpoints.get(&cp_id) {
                storage.store_checkpoint(cp)?;
            }
            storage.store_dag(&self.checkpoint_dag)?;
            let branch = &self.branches[self.current_branch];
            storage.update_branch_head(&branch.name, &cp_id)?;
        }

        Ok(cp_id)
    }

    // - -Logs - -

    /// Trace back the ancestor chain from the current head
    ///
    /// Returns checkpoints in BFS order from head to root.
    /// For merge commits, traverses all parents to ensure no history is lost.
    pub fn log(&self, count: usize) -> Vec<&Checkpoint> {
        self.log_from(&self.current_branch_head(), count)
    }

    /// Backtracking from a given checkpoint
    ///
    /// Returns checkpoints in BFS order from start checkpoint.
    /// For merge commits, traverses all parents to ensure no history is lost.
    pub fn log_from(&self, start: &CheckpointId, count: usize) -> Vec<&Checkpoint> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(*start);

        while let Some(cp_id) = queue.pop_front() {
            if result.len() >= count {
                break;
            }
            if !visited.insert(cp_id) {
                continue;
            }
            if let Some(cp) = self.checkpoints.get(&cp_id) {
                result.push(cp);
                for parent in &cp.parents {
                    if !visited.contains(parent) {
                        queue.push_back(*parent);
                    }
                }
            }
        }

        result
    }

    // - - Enquiry - -

    /// Get the number of all checkpoints
    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }

    /// DAG references
    pub fn dag(&self) -> &CheckpointDag {
        &self.checkpoint_dag
    }

    /// Delete checkpoints
    /// Auto-persists the deletion and updated DAG to storage.
    pub fn remove_checkpoint(&mut self, id: &CheckpointId) -> Result<()> {
        if self.checkpoints.remove(id).is_none() {
            return Err(StratumError::NotFound(format!(
                "checkpoint {} not found",
                id
            )));
        }
        self.checkpoint_dag.remove_node(id);
        if let Some(storage) = &self.storage {
            storage.delete_checkpoint(id)?;
            storage.store_dag(&self.checkpoint_dag)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::ContentId;

    fn dummy_snapshot_id(n: u8) -> SnapshotId {
        ContentId::from_content(&[n; 8])
    }

    #[test]
    fn test_init_repo() {
        let snap = dummy_snapshot_id(1);
        let repo = CheckpointRepo::new_single(snap);
        assert_eq!(repo.branches.len(), 1);
        assert_eq!(repo.branches[0].name, "main");
        assert_eq!(repo.checkpoint_count(), 1);
    }

    #[test]
    fn test_init_repo_multi_snapshot() {
        let snap1 = dummy_snapshot_id(1);
        let snap2 = dummy_snapshot_id(2);
        let snapshots = vec![snap1, snap2];
        let repo = CheckpointRepo::new(snapshots);
        assert_eq!(repo.checkpoint_count(), 1);
        let root = repo.get_checkpoint(&repo.current_branch_head()).unwrap();
        assert_eq!(root.baseline_snapshots.len(), 2);
    }

    #[test]
    fn test_linear_commit() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);

        let snap2 = dummy_snapshot_id(2);
        let cp1 = repo.commit_single(snap2, "second commit", "user").unwrap();

        let snap3 = dummy_snapshot_id(3);
        let cp2 = repo.commit_single(snap3, "third commit", "user").unwrap();

        let log = repo.log(10);
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].id, cp2);
        assert_eq!(log[1].id, cp1);
    }

    #[test]
    fn test_multi_snapshot_commit() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);

        let snap2 = dummy_snapshot_id(2);
        let snap3 = dummy_snapshot_id(3);
        let cp1 = repo
            .commit(vec![snap2, snap3], "multi-file commit", "user")
            .unwrap();

        let cp = repo.get_checkpoint(&cp1).unwrap();
        assert_eq!(cp.baseline_snapshots.len(), 2);
    }

    #[test]
    fn test_create_and_switch_branch() {
        let snap = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap);

        repo.create_branch("feature").unwrap();
        assert_eq!(repo.branches.len(), 2);

        repo.switch_branch("feature").unwrap();
        assert_eq!(repo.current_branch_name(), "feature");
    }

    #[test]
    fn test_merge_branches() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);

        let snap2 = dummy_snapshot_id(2);
        repo.commit_single(snap2, "main v2", "user").unwrap();

        repo.create_branch("feature").unwrap();

        let snap3 = dummy_snapshot_id(3);
        repo.commit_single(snap3, "feature v1", "user").unwrap();

        repo.switch_branch("main").unwrap();

        let snap4 = dummy_snapshot_id(4);
        let merge_cp = repo
            .merge_branches("feature", vec![snap4], "merge feature", "user")
            .unwrap();

        let cp = repo.get_checkpoint(&merge_cp).unwrap();
        assert_eq!(
            cp.parents.len(),
            2,
            "merge checkpoint should have 2 parents"
        );
    }

    #[test]
    fn test_log_count() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);

        for i in 2..=10 {
            repo.commit_single(dummy_snapshot_id(i), &format!("commit {}", i), "user")
                .unwrap();
        }

        let log = repo.log(5);
        assert_eq!(log.len(), 5);
        assert_eq!(log[0].metadata.message, "commit 10");
    }

    #[test]
    fn test_list_branches() {
        let snap = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap);

        repo.create_branch("feature-a").unwrap();
        repo.create_branch("feature-b").unwrap();

        let branches = repo.list_branches();
        assert_eq!(branches.len(), 3);
    }

    #[test]
    fn test_empty_commit_fails() {
        let snap = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap);
        let result = repo.commit(vec![], "empty", "user");
        assert!(result.is_err());
    }

    #[test]
    fn test_switch_nonexistent_branch_fails() {
        let snap = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap);
        let result = repo.switch_branch("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_nonexistent_checkpoint_fails() {
        let snap = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap);
        let fake_id = CheckpointId::from_content(b"fake");
        let result = repo.remove_checkpoint(&fake_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_duplicate_branch_fails() {
        let snap = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap);
        repo.create_branch("feature").unwrap();
        let result = repo.create_branch("feature");
        assert!(result.is_err());
    }

    #[test]
    fn test_create_branch_from_nonexistent_checkpoint_fails() {
        let snap = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap);
        let fake_id = CheckpointId::from_content(b"fake");
        let result = repo.create_branch_from("feature", fake_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_merge_with_empty_snapshots_fails() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);
        repo.create_branch("feature").unwrap();
        let result = repo.merge_branches("feature", vec![], "merge", "user");
        assert!(result.is_err());
    }

    #[test]
    fn test_merge_nonexistent_branch_fails() {
        let snap1 = dummy_snapshot_id(1);
        let snap2 = dummy_snapshot_id(2);
        let mut repo = CheckpointRepo::new_single(snap1);
        let result = repo.merge_branches("nonexistent", vec![snap2], "merge", "user");
        assert!(result.is_err());
    }

    #[test]
    fn test_log_from_merge_commit() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);

        let snap2 = dummy_snapshot_id(2);
        repo.commit_single(snap2, "main v2", "user").unwrap();

        repo.create_branch("feature").unwrap();

        let snap3 = dummy_snapshot_id(3);
        repo.commit_single(snap3, "feature v1", "user").unwrap();

        repo.switch_branch("main").unwrap();

        let snap4 = dummy_snapshot_id(4);
        repo.merge_branches("feature", vec![snap4], "merge feature", "user")
            .unwrap();

        let log = repo.log(10);
        assert!(log.len() >= 4);

        let merge_cp = &log[0];
        assert_eq!(merge_cp.parents.len(), 2, "merge should have 2 parents");
    }

    #[test]
    fn test_get_branch_head_nonexistent_fails() {
        let snap = dummy_snapshot_id(1);
        let repo = CheckpointRepo::new_single(snap);
        let result = repo.get_branch_head("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_branch_switching() {
        let snap = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap);

        repo.create_branch("feature-a").unwrap();
        repo.create_branch("feature-b").unwrap();

        repo.switch_branch("feature-a").unwrap();
        assert_eq!(repo.current_branch_name(), "feature-a");

        repo.switch_branch("feature-b").unwrap();
        assert_eq!(repo.current_branch_name(), "feature-b");

        repo.switch_branch("main").unwrap();
        assert_eq!(repo.current_branch_name(), "main");
    }

    #[test]
    fn test_remove_checkpoint_updates_dag() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);

        let snap2 = dummy_snapshot_id(2);
        let _cp1 = repo.commit_single(snap2, "second", "user").unwrap();

        let snap3 = dummy_snapshot_id(3);
        let cp2 = repo.commit_single(snap3, "third", "user").unwrap();

        repo.remove_checkpoint(&cp2).unwrap();

        assert_eq!(repo.checkpoint_count(), 2);
        assert!(repo.get_checkpoint(&cp2).is_err());
        assert!(!repo.checkpoint_dag.has_node(&cp2));
    }

    #[test]
    fn test_find_branch() {
        let snap = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap);

        repo.create_branch("feature").unwrap();
        let idx = repo.find_branch("feature").unwrap();
        assert_eq!(repo.branches[idx].name, "feature");

        let result = repo.find_branch("nonexistent");
        assert!(result.is_err());
    }
}
