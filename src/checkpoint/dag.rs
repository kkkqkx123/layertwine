//! DAG — 有向无环图
//!
//! 管理检查点的父子关系，提供祖先查询、可达性判断、共同祖先查找等功能。
//! 参考 architecture/05-检查点仓库与分支管理.md §5.4

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use crate::core::types::CheckpointId;

/// 有向无环图
///
/// `nodes`: node → children (反向索引，用于向前遍历)
/// parents 关系在 Checkpoint 实体中维护（向后遍历）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointDag {
    /// node → children 映射（反向索引）
    nodes: HashMap<CheckpointId, HashSet<CheckpointId>>,
    /// Generation number: node → 从根开始的最大距离
    generation: HashMap<CheckpointId, u64>,
}

impl CheckpointDag {
    /// 创建空的 DAG
    pub fn new() -> Self {
        CheckpointDag {
            nodes: HashMap::new(),
            generation: HashMap::new(),
        }
    }

    /// 添加节点
    pub fn add_node(&mut self, id: CheckpointId) {
        self.nodes.entry(id).or_default();
        self.generation.entry(id).or_insert(0);
    }

    /// 添加父子关系（parent → child）
    pub fn add_edge(&mut self, parent: CheckpointId, child: CheckpointId) {
        // 确保两个节点都存在
        self.nodes.entry(parent).or_default();
        self.nodes.entry(child).or_default();

        // 添加子关系
        self.nodes.entry(parent).or_default().insert(child);

        // 更新 generation number
        let parent_gen = *self.generation.get(&parent).unwrap_or(&0);
        let child_gen = self.generation.entry(child).or_insert(0);
        *child_gen = (*child_gen).max(parent_gen + 1);
    }

    /// 检查节点是否存在
    pub fn has_node(&self, id: &CheckpointId) -> bool {
        self.nodes.contains_key(id)
    }

    /// 获取节点的所有祖先（从近到远，线性遍历）
    ///
    /// 通过 Checkpoint 实体中的 parents 列表进行遍历。
    /// 注意：此方法需要传入 parent 查询函数。
    pub fn ancestors<F>(&self, id: &CheckpointId, get_parents: F) -> Vec<CheckpointId>
    where
        F: Fn(&CheckpointId) -> Vec<CheckpointId>,
    {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(*id);

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            if current != *id {
                result.push(current);
            }
            for parent in get_parents(&current) {
                if !visited.contains(&parent) {
                    queue.push_back(parent);
                }
            }
        }

        result
    }

    /// 判断 ancestor 是否是 descendant 的祖先（可达性判断）
    ///
    /// 从 ancestor 的子节点向前遍历 BFS。
    pub fn is_ancestor(&self, ancestor: &CheckpointId, descendant: &CheckpointId) -> bool {
        if ancestor == descendant {
            return true;
        }

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(*ancestor);

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(children) = self.nodes.get(&current) {
                for child in children {
                    if *child == *descendant {
                        return true;
                    }
                    if !visited.contains(child) {
                        queue.push_back(*child);
                    }
                }
            }
        }

        false
    }

    /// 寻找两个节点的共同祖先
    ///
    /// 获取 id1 的所有祖先，再从 id2 反向遍历找第一个匹配的。
    pub fn merge_base<F>(&self, id1: &CheckpointId, id2: &CheckpointId, get_parents: F) -> Option<CheckpointId>
    where
        F: Fn(&CheckpointId) -> Vec<CheckpointId>,
    {
        let ancestors1: HashSet<CheckpointId> = self
            .ancestors(id1, &get_parents)
            .into_iter()
            .chain(std::iter::once(*id1))
            .collect();

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(*id2);

        while let Some(current) = queue.pop_front() {
            if ancestors1.contains(&current) {
                return Some(current);
            }
            if !visited.insert(current) {
                continue;
            }
            for parent in get_parents(&current) {
                if !visited.contains(&parent) {
                    queue.push_back(parent);
                }
            }
        }

        None
    }

    /// 获取节点的子节点列表
    pub fn get_children(&self, id: &CheckpointId) -> Vec<CheckpointId> {
        self.nodes
            .get(id)
            .map(|children| children.iter().copied().collect())
            .unwrap_or_default()
    }

    /// 获取节点的 generation number
    pub fn generation(&self, id: &CheckpointId) -> Option<u64> {
        self.generation.get(id).copied()
    }

    /// 获取所有节点
    pub fn all_nodes(&self) -> Vec<CheckpointId> {
        self.nodes.keys().copied().collect()
    }

    /// 节点数量
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl Default for CheckpointDag {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::ContentId;

    fn cid(data: &[u8]) -> CheckpointId {
        ContentId::from_content(data)
    }

    /// 辅助函数：模拟从 Checkpoint 中获取 parents
    fn make_get_parents(checkpoints: &HashMap<CheckpointId, Vec<CheckpointId>>) -> impl Fn(&CheckpointId) -> Vec<CheckpointId> + '_ {
        move |id| checkpoints.get(id).cloned().unwrap_or_default()
    }

    #[test]
    fn test_dag_add_node_and_edge() {
        let mut dag = CheckpointDag::new();
        let a = cid(b"a");
        let b = cid(b"b");

        dag.add_node(a);
        dag.add_node(b);
        assert_eq!(dag.len(), 2);

        dag.add_edge(a, b);
        assert_eq!(dag.get_children(&a), vec![b]);
    }

    #[test]
    fn test_dag_is_ancestor() {
        let mut dag = CheckpointDag::new();
        let a = cid(b"a");
        let b = cid(b"b");
        let c = cid(b"c");

        dag.add_edge(a, b);
        dag.add_edge(b, c);

        assert!(dag.is_ancestor(&a, &c));
        assert!(dag.is_ancestor(&a, &b));
        assert!(!dag.is_ancestor(&c, &a));
    }

    #[test]
    fn test_dag_ancestors() {
        let mut dag = CheckpointDag::new();
        let a = cid(b"a");
        let b = cid(b"b");
        let c = cid(b"c");

        dag.add_edge(a, b);
        dag.add_edge(b, c);

        let mut parents = HashMap::new();
        parents.insert(b, vec![a]);
        parents.insert(c, vec![b]);
        let get_p = make_get_parents(&parents);

        // c 的祖先：a, b
        let ancestors = dag.ancestors(&c, &get_p);
        assert!(ancestors.contains(&a));
        assert!(ancestors.contains(&b));
    }

    #[test]
    fn test_dag_merge_base() {
        let mut dag = CheckpointDag::new();
        let a = cid(b"root");
        let b = cid(b"branch1");
        let c = cid(b"branch2");

        dag.add_edge(a, b);
        dag.add_edge(a, c);

        let mut parents: HashMap<CheckpointId, Vec<CheckpointId>> = HashMap::new();
        parents.insert(b, vec![a]);
        parents.insert(c, vec![a]);
        let get_p = make_get_parents(&parents);

        let base = dag.merge_base(&b, &c, &get_p);
        assert_eq!(base, Some(a));
    }

    #[test]
    fn test_generation_number() {
        let mut dag = CheckpointDag::new();
        let a = cid(b"root");
        let b = cid(b"child");
        let c = cid(b"grandchild");

        dag.add_edge(a, b);
        dag.add_edge(b, c);

        assert_eq!(dag.generation(&a), Some(0));
        assert_eq!(dag.generation(&b), Some(1));
        assert_eq!(dag.generation(&c), Some(2));
    }
}
