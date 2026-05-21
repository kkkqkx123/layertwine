//! CheckpointRepo — 检查点仓库总控
//!
//! 管理检查点提交、分支创建/切换/合并、DAG 历史追踪。
//! 参考 architecture/05-检查点仓库与分支管理.md §5.5

use crate::checkpoint::branch::Branch;
use crate::checkpoint::checkpoint::{Checkpoint, CheckpointMetadata};
use crate::checkpoint::dag::CheckpointDag;
use crate::core::types::{CheckpointId, SnapshotId};
use crate::error::{Result, StratumError};
use std::collections::HashMap;

/// 检查点仓库 — 版本管理核心
///
/// 独立于 Git 的版本管理，管理检查点提交、分支、DAG 历史。
pub struct CheckpointRepo {
    /// 所有分支
    pub branches: Vec<Branch>,
    /// 当前分支索引
    pub current_branch: usize,
    /// 检查点 DAG
    pub checkpoint_dag: CheckpointDag,
    /// 所有检查点（ID → Checkpoint）
    checkpoints: HashMap<CheckpointId, Checkpoint>,
}

impl CheckpointRepo {
    /// 创建新的检查点仓库
    pub fn new(initial_snapshot: SnapshotId) -> Self {
        // 创建初始 checkpoint
        let metadata = CheckpointMetadata::new("system", "root checkpoint");
        let root = Checkpoint::new(initial_snapshot, vec![], metadata);
        let root_id = root.id;

        let mut dag = CheckpointDag::new();
        dag.add_node(root_id);

        let mut checkpoints = HashMap::new();
        checkpoints.insert(root_id, root);

        // 创建 main 分支
        let main_branch = Branch::new("main", root_id);

        CheckpointRepo {
            branches: vec![main_branch],
            current_branch: 0,
            checkpoint_dag: dag,
            checkpoints,
        }
    }

    // ── 检查点操作 ──

    /// 获取指定检查点
    pub fn get_checkpoint(&self, id: &CheckpointId) -> Result<&Checkpoint> {
        self.checkpoints
            .get(id)
            .ok_or_else(|| StratumError::NotFound(format!("checkpoint {} not found", id)))
    }

    /// 获取可变引用
    pub fn get_checkpoint_mut(&mut self, id: &CheckpointId) -> Result<&mut Checkpoint> {
        self.checkpoints
            .get_mut(id)
            .ok_or_else(|| StratumError::NotFound(format!("checkpoint {} not found", id)))
    }

    /// 提交：将 staged 的当前状态打包为 Checkpoint
    ///
    /// 1. 创建新 Checkpoint
    /// 2. 添加到 DAG
    /// 3. 更新分支 head
    pub fn commit(
        &mut self,
        snapshot_id: SnapshotId,
        message: &str,
        author: &str,
    ) -> Result<CheckpointId> {
        let current_head = self.current_branch_head();
        let metadata = CheckpointMetadata::new(author, message);
        let cp = Checkpoint::new(snapshot_id, vec![current_head], metadata);
        let cp_id = cp.id;

        // 存储
        self.checkpoints.insert(cp_id, cp);

        // 更新 DAG
        self.checkpoint_dag.add_node(cp_id);
        self.checkpoint_dag.add_edge(current_head, cp_id);

        // 更新分支 head
        self.current_branch_mut().set_head(cp_id);

        Ok(cp_id)
    }

    // ── 分支操作 ──

    /// 获取当前分支的 head Checkpoint ID
    pub fn current_branch_head(&self) -> CheckpointId {
        self.branches[self.current_branch].head
    }

    /// 获取当前分支的可变引用
    pub fn current_branch_mut(&mut self) -> &mut Branch {
        &mut self.branches[self.current_branch]
    }

    /// 获取当前分支名称
    pub fn current_branch_name(&self) -> &str {
        &self.branches[self.current_branch].name
    }

    /// 基于当前 head 创建新分支
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

    /// 在指定 checkpoint 上创建新分支
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

    /// 切换分支
    pub fn switch_branch(&mut self, name: &str) -> Result<usize> {
        let idx = self
            .branches
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| StratumError::NotFound(format!("branch '{}' not found", name)))?;

        self.current_branch = idx;
        Ok(idx)
    }

    /// 列出所有分支
    pub fn list_branches(&self) -> &[Branch] {
        &self.branches
    }

    /// 查找分支索引
    pub fn find_branch(&self, name: &str) -> Result<usize> {
        self.branches
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| StratumError::NotFound(format!("branch '{}' not found", name)))
    }

    /// 获取指定分支的 head
    pub fn get_branch_head(&self, name: &str) -> Result<CheckpointId> {
        let idx = self.find_branch(name)?;
        Ok(self.branches[idx].head)
    }

    /// 合并分支：将 source_branch 合并到当前分支
    ///
    /// 生成多父 Checkpoint，添加到 DAG。
    pub fn merge_branches(
        &mut self,
        source_branch: &str,
        snapshot_id: SnapshotId,
        message: &str,
        author: &str,
    ) -> Result<CheckpointId> {
        let source_head = self.get_branch_head(source_branch)?;
        let current_head = self.current_branch_head();

        let metadata = CheckpointMetadata::new(author, message);
        let cp = Checkpoint::new(snapshot_id, vec![current_head, source_head], metadata);
        let cp_id = cp.id;

        self.checkpoints.insert(cp_id, cp);

        // 更新 DAG
        self.checkpoint_dag.add_node(cp_id);
        self.checkpoint_dag.add_edge(current_head, cp_id);
        self.checkpoint_dag.add_edge(source_head, cp_id);

        // 更新当前分支 head
        self.current_branch_mut().set_head(cp_id);

        Ok(cp_id)
    }

    // ── 日志 ──

    /// 从当前 head 回溯祖先链
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

    /// 从指定 checkpoint 回溯
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

    // ── 查询 ──

    /// 获取所有检查点数量
    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }

    /// DAG 引用
    pub fn dag(&self) -> &CheckpointDag {
        &self.checkpoint_dag
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
        let repo = CheckpointRepo::new(snap);
        assert_eq!(repo.branches.len(), 1);
        assert_eq!(repo.branches[0].name, "main");
        assert_eq!(repo.checkpoint_count(), 1);
    }

    #[test]
    fn test_linear_commit() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new(snap1);

        let snap2 = dummy_snapshot_id(2);
        let cp1 = repo.commit(snap2, "second commit", "user").unwrap();

        let snap3 = dummy_snapshot_id(3);
        let cp2 = repo.commit(snap3, "third commit", "user").unwrap();

        // log 应该返回 3 条（含 root）
        let log = repo.log(10);
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].id, cp2);
        assert_eq!(log[1].id, cp1);
    }

    #[test]
    fn test_create_and_switch_branch() {
        let snap = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new(snap);

        repo.create_branch("feature").unwrap();
        assert_eq!(repo.branches.len(), 2);

        repo.switch_branch("feature").unwrap();
        assert_eq!(repo.current_branch_name(), "feature");
    }

    #[test]
    fn test_merge_branches() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new(snap1);

        // main 分支上提交一次
        let snap2 = dummy_snapshot_id(2);
        repo.commit(snap2, "main v2", "user").unwrap();

        // 创建 feature 分支并切换
        repo.create_branch("feature").unwrap();

        // feature 上提交
        let snap3 = dummy_snapshot_id(3);
        repo.commit(snap3, "feature v1", "user").unwrap();

        // 切回 main
        repo.switch_branch("main").unwrap();

        // main 上合并 feature
        let snap4 = dummy_snapshot_id(4);
        let merge_cp = repo
            .merge_branches("feature", snap4, "merge feature", "user")
            .unwrap();

        let cp = repo.get_checkpoint(&merge_cp).unwrap();
        assert_eq!(cp.parents.len(), 2, "merge checkpoint should have 2 parents");
    }

    #[test]
    fn test_log_count() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new(snap1);

        for i in 2..=10 {
            repo.commit(dummy_snapshot_id(i), &format!("commit {}", i), "user")
                .unwrap();
        }

        let log = repo.log(5);
        assert_eq!(log.len(), 5);
        assert_eq!(log[0].metadata.message, "commit 10");
    }

    #[test]
    fn test_list_branches() {
        let snap = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new(snap);

        repo.create_branch("feature-a").unwrap();
        repo.create_branch("feature-b").unwrap();

        let branches = repo.list_branches();
        assert_eq!(branches.len(), 3);
    }
}
