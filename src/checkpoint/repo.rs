use crate::checkpoint::branch::Branch;
use crate::checkpoint::checkpoint::{Checkpoint, CheckpointMetadata};
use crate::checkpoint::dag::CheckpointDag;
use crate::core::types::{CheckpointId, SnapshotId};
use crate::error::{Result, StratumError};
use std::collections::HashMap;

/// Checkpoint Repository - Versioning Core
///
/// Git-independent versioning, managing checkpoint commits, branches, and DAG history.
pub struct CheckpointRepo {
    /// All branches
    pub branches: Vec<Branch>,
    /// Current branch index
    pub current_branch: usize,
    /// Checkpoint DAG
    pub checkpoint_dag: CheckpointDag,
    /// All checkpoints (ID → Checkpoint)
    checkpoints: HashMap<CheckpointId, Checkpoint>,
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
        }
    }

    /// Convenient single-snapshot compatible construction
    pub fn new_single(initial_snapshot: SnapshotId) -> Self {
        CheckpointRepo::new(vec![initial_snapshot])
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

    /// Create a new branch based on the current head
    pub fn create_branch(&mut self, name: &str) -> Result<()> {
        if self.branches.iter().any(|b| b.name == name) {
            return Err(StratumError::Checkpoint(format!(
                "branch '{}' already exists",
                name
            )));
        }
        let head = self.current_branch_head();
        let branch = Branch::new(name, head);
        self.branches.push(branch);
        Ok(())
    }

    /// Create a new branch on the specified checkpoint
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
        self.branches.push(branch);
        Ok(())
    }

    /// Switching Branches
    pub fn switch_branch(&mut self, name: &str) -> Result<usize> {
        let idx = self
            .branches
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| StratumError::NotFound(format!("branch '{}' not found", name)))?;

        self.current_branch = idx;
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

        Ok(cp_id)
    }

    // - -Logs - -

    /// Trace back the ancestor chain from the current head
    pub fn log(&self, count: usize) -> Vec<&Checkpoint> {
        let mut result = Vec::new();
        let mut current = Some(self.current_branch_head());

        while let Some(cp_id) = current {
            if result.len() >= count {
                break;
            }
            if let Some(cp) = self.checkpoints.get(&cp_id) {
                result.push(cp);
                current = cp.parents.first().copied();
            } else {
                break;
            }
        }

        result
    }

    /// Backtracking from a given checkpoint
    pub fn log_from(&self, start: &CheckpointId, count: usize) -> Vec<&Checkpoint> {
        let mut result = Vec::new();
        let mut current = Some(*start);

        while let Some(cp_id) = current {
            if result.len() >= count {
                break;
            }
            if let Some(cp) = self.checkpoints.get(&cp_id) {
                result.push(cp);
                current = cp.parents.first().copied();
            } else {
                break;
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
    pub fn remove_checkpoint(&mut self, id: &CheckpointId) -> Result<()> {
        if self.checkpoints.remove(id).is_none() {
            return Err(crate::error::StratumError::NotFound(
                format!("checkpoint {} not found", id),
            ));
        }
        self.checkpoint_dag.remove_node(id);
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
        let cp1 = repo.commit(vec![snap2, snap3], "multi-file commit", "user").unwrap();

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
        assert_eq!(cp.parents.len(), 2, "merge checkpoint should have 2 parents");
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
}