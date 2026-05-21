//! agent_edit 层操作
//!
//! 每个 Agent 实例隔离在独立 Partition 中。Agent 修改先进入自身分区，
//! 然后通过 move_agent_to_approval 移入 approval 层的对应分区。

use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{
    AgentInstanceId, PartitionId, PartitionType, SnapshotId, SourceType,
};
use crate::engine::diff::diff_to_line_diff;
use crate::engine::merge::apply_deltas;
use crate::error::{Result, StratumError};
use crate::storage::repository::{DeltaStore, FileNodeStore, PartitionStore, SnapshotStore};
use crate::storage::sqlite_storage::SqliteStorage;
use std::path::PathBuf;
/// 生成 agent 分区的稳定 ID
pub fn agent_partition_id(agent_id: &AgentInstanceId) -> PartitionId {
    let uuid = uuid::Uuid::from_u128(0x2000_0000_0000_0000_0000_0000_0000_0000);
    let bytes = uuid.as_bytes();
    let agent_bytes = agent_id.0.as_bytes();
    let mut new_bytes = *bytes;
    for (i, b) in agent_bytes.iter().enumerate().take(16) {
        new_bytes[i] = new_bytes[i].wrapping_add(*b);
    }
    uuid::Uuid::from_bytes(new_bytes)
}

/// 获取或创建 agent_edit 分区
pub fn ensure_agent_partition(
    storage: &SqliteStorage,
    agent_id: &AgentInstanceId,
    initial_snapshot_id: SnapshotId,
) -> Result<Partition> {
    let pid = agent_partition_id(agent_id);
    match storage.get_partition(&pid) {
        Ok(p) => Ok(p),
        Err(_) => {
            let partition = Partition::new(
                format!("agent_edit/{}", agent_id),
                PartitionType::Agent(agent_id.clone()),
                initial_snapshot_id,
            );
            storage
                .create_partition(&partition)
                .map_err(|e| StratumError::Storage(e.into()))?;
            Ok(partition)
        }
    }
}

/// Agent 编辑文件
///
/// 将 Agent 的修改作为 Delta 追加到 agent_edit 层的对应分区。
/// 每个 Agent 实例有独立分区，互不干扰。
pub fn apply_agent_edit(
    storage: &SqliteStorage,
    agent_id: &AgentInstanceId,
    file_path: &str,
    new_content: &str,
) -> Result<SnapshotId> {
    let pid = agent_partition_id(agent_id);
    let partition = storage
        .get_partition(&pid)
        .map_err(|_| StratumError::NotFound(format!(
            "agent partition for {} not found, call ensure_agent_partition first", agent_id
        )))?;

    let current_snapshot = storage
        .get_snapshot(&partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    // 读取旧内容
    let old_content = {
        let deltas = storage
            .get_deltas(&current_snapshot.deltas)
            .map_err(|e| StratumError::Storage(e.into()))?;
        let content_str = String::from_utf8_lossy(
            &storage
                .get_file_content(&current_snapshot.file)
                .map_err(|e| StratumError::Storage(e.into()))?,
        )
        .to_string();
        apply_deltas(&content_str, &deltas)
            .map_err(|e| StratumError::Engine(e.to_string()))?
    };

    // 计算 diff
    let line_diff = diff_to_line_diff(&old_content, new_content);
    if line_diff.is_empty() {
        return Ok(partition.current_snapshot);
    }

    // 创建 Delta
    let file_node = FileNode::new(PathBuf::from(file_path), old_content.as_bytes());
    let delta = Delta::new(
        file_node.clone(),
        line_diff,
        SourceType::Agent(agent_id.clone()),
    );
    storage
        .store_file_node(&file_node, old_content.as_bytes())
        .map_err(|e| StratumError::Storage(e.into()))?;
    storage
        .store_delta(&delta)
        .map_err(|e| StratumError::Storage(e.into()))?;
    storage
        .store_file_node(&file_node, new_content.as_bytes())
        .map_err(|e| StratumError::Storage(e.into()))?;

    // 创建新 Snapshot
    let new_snapshot = Snapshot::from_parent(
        &current_snapshot,
        delta.id,
        PartitionType::Agent(agent_id.clone()).name(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(|e| StratumError::Storage(e.into()))?;

    // 更新分区指针
    storage
        .update_pointer(&pid, &new_snapshot.id)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(new_snapshot.id)
}

/// 将 Agent 的修改移动到 approval 层的 Agent 分区
///
/// 对应架构文档的 `move_agent_to_approval`:
/// - 取 agent_raw 分区和 approval agent 分区的当前快照
/// - 合并生成新快照推入 approval agent 分区
pub fn move_agent_to_approval(
    storage: &SqliteStorage,
    agent_id: &AgentInstanceId,
) -> Result<SnapshotId> {
    let agent_pid = agent_partition_id(agent_id);
    let approval_pid = crate::state_machine::approval::approval_agent_partition_id(agent_id);

    let agent_partition = storage
        .get_partition(&agent_pid)
        .map_err(|_| StratumError::NotFound(format!("agent partition {} not found", agent_id)))?;
    let approval_partition = storage
        .get_partition(&approval_pid)
        .map_err(|_| StratumError::NotFound(format!(
            "approval partition for agent {} not found, call ensure_approval_agent_partition first",
            agent_id
        )))?;

    let agent_snapshot = storage
        .get_snapshot(&agent_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;
    let approval_snapshot = storage
        .get_snapshot(&approval_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    // 重建文本
    let agent_text =
        crate::state_machine::transition::reconstruct_text(storage, &agent_snapshot)?;
    let approval_text =
        crate::state_machine::transition::reconstruct_text(storage, &approval_snapshot)?;

    // 计算合并 diff
    let merge_diff = diff_to_line_diff(&approval_text, &agent_text);
    if merge_diff.is_empty() {
        return Ok(approval_partition.current_snapshot);
    }

    let merge_delta = Delta::new(
        agent_snapshot.file.clone(),
        merge_diff,
        SourceType::Agent(agent_id.clone()),
    );
    storage
        .store_delta(&merge_delta)
        .map_err(|e| StratumError::Storage(e.into()))?;

    // 创建 merge snapshot
    let new_snapshot = Snapshot::merge(
        vec![&approval_snapshot, &agent_snapshot],
        merge_delta.id,
        PartitionType::Approval(agent_id.clone()).name(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(|e| StratumError::Storage(e.into()))?;

    storage
        .update_pointer(&approval_pid, &new_snapshot.id)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(new_snapshot.id)
}

/// 放弃 Agent 修改（仅切换指针到父 Snapshot）
pub fn discard_agent_edit(
    storage: &SqliteStorage,
    agent_id: &AgentInstanceId,
) -> Result<()> {
    let pid = agent_partition_id(agent_id);
    let partition = storage
        .get_partition(&pid)
        .map_err(|_| StratumError::NotFound(format!("agent partition {} not found", agent_id)))?;

    let current_snapshot = storage
        .get_snapshot(&partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    // 如果存在父快照，回退到父快照
    if let Some(&parent_id) = current_snapshot.parents.first() {
        storage
            .update_pointer(&pid, &parent_id)
            .map_err(|e| StratumError::Storage(e.into()))?;
        Ok(())
    } else {
        Err(StratumError::StateMachine(
            "agent has no parent snapshot to discard to".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::SourceType;
    use crate::storage::sqlite_storage::SqliteStorage;
    use std::sync::Arc;

    fn setup_storage() -> Arc<SqliteStorage> {
        Arc::new(SqliteStorage::new_in_memory().unwrap())
    }

    fn create_initial_snapshot(storage: &SqliteStorage, content: &str) -> SnapshotId {
        let file_node = FileNode::new(std::path::PathBuf::from("test.txt"), content.as_bytes());
        storage.store_file_node(&file_node, content.as_bytes()).unwrap();

        let empty_diff = crate::core::delta::LineDiff::new(vec![]);
        let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Agent("test-agent".into()));
        storage.store_delta(&delta).unwrap();

        let snapshot = Snapshot::new_initial(file_node, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();
        snapshot.id
    }

    #[test]
    fn test_apply_agent_edit() {
        let storage = setup_storage();
        let agent_id = AgentInstanceId("agent-1".into());
        let initial_id = create_initial_snapshot(&storage, "base\n");
        ensure_agent_partition(&storage, &agent_id, initial_id).unwrap();

        let new_id = apply_agent_edit(&storage, &agent_id, "test.txt", "base\nmodified\n").unwrap();
        assert_ne!(new_id, initial_id);
    }

    #[test]
    fn test_discard_agent_edit() {
        let storage = setup_storage();
        let agent_id = AgentInstanceId("agent-2".into());
        let initial_id = create_initial_snapshot(&storage, "original\n");
        ensure_agent_partition(&storage, &agent_id, initial_id).unwrap();

        // 应用编辑
        let edited_id = apply_agent_edit(&storage, &agent_id, "test.txt", "original\nchanged\n").unwrap();
        assert_ne!(edited_id, initial_id);

        // 放弃编辑 — 回退到父快照
        discard_agent_edit(&storage, &agent_id).unwrap();
        let partition = storage.get_partition(&agent_partition_id(&agent_id)).unwrap();
        assert_eq!(partition.current_snapshot, initial_id);
    }

    #[test]
    fn test_agent_isolation() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "shared\n");

        let agent_a = AgentInstanceId("agent-a".into());
        let agent_b = AgentInstanceId("agent-b".into());

        ensure_agent_partition(&storage, &agent_a, initial_id).unwrap();
        ensure_agent_partition(&storage, &agent_b, initial_id).unwrap();

        let a_id = apply_agent_edit(&storage, &agent_a, "test.txt", "shared\na-edit\n").unwrap();
        let b_id = apply_agent_edit(&storage, &agent_b, "test.txt", "shared\nb-edit\n").unwrap();

        assert_ne!(a_id, b_id);

        // 验证各自分区独立
        let pa = storage.get_partition(&agent_partition_id(&agent_a)).unwrap();
        let pb = storage.get_partition(&agent_partition_id(&agent_b)).unwrap();
        assert_eq!(pa.current_snapshot, a_id);
        assert_eq!(pb.current_snapshot, b_id);
    }
}
