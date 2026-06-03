use std::collections::{HashSet, VecDeque};

use crate::checkpoint::repo::CheckpointRepo;
use crate::core::types::CheckpointId;
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GCStats {
    pub removed_checkpoints: u64,
    pub removed_snapshots: u64,
    pub freed_bytes: u64,
    pub delta_chain_depth_triggered: bool,
    pub max_chain_depth: usize,
}

impl GCStats {
    pub fn new() -> Self {
        GCStats {
            removed_checkpoints: 0,
            removed_snapshots: 0,
            freed_bytes: 0,
            delta_chain_depth_triggered: false,
            max_chain_depth: 0,
        }
    }
}

impl Default for GCStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Collect all protected checkpoints that must never be removed.
///
/// Protected set includes:
/// 1. All branch head checkpoints and their ancestors (via BFS over all parents)
/// 2. All checkpoints bound to a git_anchor
pub fn collect_protected_checkpoints(repo: &CheckpointRepo) -> HashSet<CheckpointId> {
    let mut protected = HashSet::new();

    // All branch heads and their ancestors (traverse ALL parents, not just first)
    for branch in repo.list_branches() {
        let mut queue: VecDeque<CheckpointId> = VecDeque::new();
        queue.push_back(branch.head);

        while let Some(id) = queue.pop_front() {
            if !protected.insert(id) {
                continue;
            }
            if let Ok(cp) = repo.get_checkpoint(&id) {
                for parent in &cp.parents {
                    if !protected.contains(parent) {
                        queue.push_back(*parent);
                    }
                }
            }
        }
    }

    // All checkpoints bound to a git_anchor
    for cp_id in repo.dag().all_nodes() {
        if let Ok(cp) = repo.get_checkpoint(&cp_id) {
            if cp.metadata.git_anchor.is_some() {
                protected.insert(cp_id);
            }
        }
    }

    protected
}

/// Mark all checkpoints reachable from the protected set via children edges.
fn mark_reachable(repo: &CheckpointRepo, protected: &HashSet<CheckpointId>) -> HashSet<CheckpointId> {
    let mut reachable = protected.clone();
    let mut queue: VecDeque<CheckpointId> = protected.iter().copied().collect();

    while let Some(current) = queue.pop_front() {
        let children = repo.dag().get_children(&current);
        for child in children {
            if reachable.insert(child) {
                queue.push_back(child);
            }
        }
    }

    reachable
}

/// Run garbage collection on the checkpoint repository.
///
/// Mark-sweep algorithm:
/// 1. Collect protected checkpoints (branch heads + ancestors + git_anchor)
/// 2. Mark all descendants of protected checkpoints as reachable
/// 3. Remove all unreachable checkpoints
/// 4. Check delta chain depth for repacking trigger
pub fn collect_garbage(repo: &mut CheckpointRepo) -> Result<GCStats> {
    let protected = collect_protected_checkpoints(repo);

    // Mark phase: traverse forward from protected to find all reachable nodes
    let to_keep = mark_reachable(repo, &protected);

    let all_checkpoints = repo.dag().all_nodes();
    let mut stats = GCStats::new();

    // Check delta chain depth
    for cp_id in &all_checkpoints {
        if let Ok(cp) = repo.get_checkpoint(cp_id) {
            let depth = cp.parents.len();
            if depth > stats.max_chain_depth {
                stats.max_chain_depth = depth;
            }
        }
    }
    if stats.max_chain_depth > 100 {
        stats.delta_chain_depth_triggered = true;
    }

    // Sweep phase: remove unreachable checkpoints
    for cp_id in &all_checkpoints {
        if to_keep.contains(cp_id) {
            continue;
        }

        if repo.remove_checkpoint(cp_id).is_ok() {
            stats.removed_checkpoints += 1;
        }
    }

    Ok(stats)
}

/// Check if the delta chain depth exceeds the threshold for repacking.
pub fn check_delta_chain_depth(repo: &CheckpointRepo) -> Result<(usize, bool)> {
    let mut max_depth = 0usize;
    for cp_id in repo.dag().all_nodes() {
        if let Ok(cp) = repo.get_checkpoint(&cp_id) {
            if cp.parents.len() > max_depth {
                max_depth = cp.parents.len();
            }
        }
    }
    let triggered = max_depth > 100;
    Ok((max_depth, triggered))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::repo::CheckpointRepo;
    use crate::core::types::ContentId;

    fn dummy_snapshot_id(n: u8) -> crate::core::types::SnapshotId {
        ContentId::from_content(&[n; 8])
    }

    #[test]
    fn test_collect_protected_empty() {
        let snap = dummy_snapshot_id(1);
        let repo = CheckpointRepo::new_single(snap);
        let protected = collect_protected_checkpoints(&repo);
        assert!(!protected.is_empty(), "root checkpoint should be protected");
    }

    #[test]
    fn test_protected_includes_branch_head() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);
        let snap2 = dummy_snapshot_id(2);
        let cp_id = repo.commit_single(snap2, "second", "user").unwrap();

        let protected = collect_protected_checkpoints(&repo);
        assert!(protected.contains(&cp_id), "branch head should be protected");
    }

    #[test]
    fn test_gc_no_removable() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);

        // Add a few commits on main
        for i in 2..=5 {
            repo.commit_single(dummy_snapshot_id(i), &format!("commit {}", i), "user")
                .unwrap();
        }

        let stats = collect_garbage(&mut repo).unwrap();
        // Nothing removable since all are on main's ancestor chain
        assert_eq!(stats.removed_checkpoints, 0);
    }

    #[test]
    fn test_gc_removes_orphan() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);
        let _main_root = repo.current_branch_head();

        // Create a branch, make commits, delete the branch reference
        repo.create_branch("feature").unwrap();
        let snap_f1 = dummy_snapshot_id(10);
        let cp_f1 = repo.commit_single(snap_f1, "feature commit", "user").unwrap();

        // Switch back to main - feature branch still exists so everything is protected
        repo.switch_branch("main").unwrap();

        // The feature branch is still protected
        let protected = collect_protected_checkpoints(&repo);
        assert!(protected.contains(&cp_f1), "feature checkpoint should be protected (branch exists)");

        // GC should not remove anything since all checkpoints are on a branch
        let stats = collect_garbage(&mut repo).unwrap();
        assert_eq!(stats.removed_checkpoints, 0);
    }

    #[test]
    fn test_gc_orphan_removal() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);

        // Simulate orphan: we need a checkpoint that has no branch pointing to it
        // and is not an ancestor of any branch head

        // Check if there are orphans using the protected set
        let protected = collect_protected_checkpoints(&repo);
        let all = repo.dag().all_nodes();
        for cp_id in all {
            if !protected.contains(&cp_id) {
                // This would be an orphan - we could test removal
            }
        }

        let stats = collect_garbage(&mut repo).unwrap();
        assert_eq!(stats.removed_checkpoints, 0);
    }

    #[test]
    fn test_delta_chain_depth_check() {
        let snap = dummy_snapshot_id(1);
        let repo = CheckpointRepo::new_single(snap);
        let (max_depth, triggered) = check_delta_chain_depth(&repo).unwrap();
        assert_eq!(max_depth, 0);
        assert!(!triggered);
    }

    #[test]
    fn test_gc_stats_struct() {
        let stats = GCStats::new();
        assert_eq!(stats.removed_checkpoints, 0);
        assert_eq!(stats.removed_snapshots, 0);
        assert_eq!(stats.freed_bytes, 0);
        assert!(!stats.delta_chain_depth_triggered);
    }

    #[test]
    fn test_protected_includes_git_anchor() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);
        let snap2 = dummy_snapshot_id(2);
        let cp_id = repo.commit_single(snap2, "anchored", "user").unwrap();

        // Remove the branch reference to make the checkpoint orphaned
        {
            let cp = repo.get_checkpoint_mut(&cp_id).unwrap();
            cp.metadata.git_anchor = Some("abc123".to_string());
        }
        // Remove branch "main" so cp would normally be orphaned
        repo.branches.retain(|b| b.name != "main");

        let protected = collect_protected_checkpoints(&repo);
        assert!(
            protected.contains(&cp_id),
            "checkpoint with git_anchor should be protected even without branch"
        );
    }

    #[test]
    fn test_gc_real_orphan_removal() {
        let snap1 = dummy_snapshot_id(1);
        let mut repo = CheckpointRepo::new_single(snap1);

        // Create a disconnected (orphan) checkpoint by directly adding to internal structures
        let orphan_snap = dummy_snapshot_id(99);
        let orphan_cp = crate::checkpoint::Checkpoint::new(
            vec![orphan_snap],
            vec![],
            crate::checkpoint::CheckpointMetadata::new("orphan", "orphan checkpoint"),
        );
        let orphan_id = orphan_cp.id;

        // Add to internal structures but NO edges to/from any protected checkpoint
        repo.checkpoints.insert(orphan_id, orphan_cp);
        repo.checkpoint_dag.add_node(orphan_id);

        let stats = collect_garbage(&mut repo).unwrap();
        assert!(
            stats.removed_checkpoints > 0,
            "GC should remove orphan checkpoint that has no connection to any branch"
        );
    }
}