//! approval 层操作
//!
//! Agent 修改经过审核后，可从 Agent Approval 分区迁移到 Integrated 分区，
//! 多个 Integrated 分区可合并到 Unified 分区。同时支持分区间的双向指针切换。

use crate::core::delta::Delta;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{
    AgentInstanceId, PartitionId, PartitionType, SnapshotId, SourceType,
};
use crate::engine::diff::diff_to_line_diff;
use crate::error::{Result, StratumError};
use crate::storage::repository::{DeltaStore, PartitionStore, SnapshotStore};
use crate::storage::sqlite_storage::SqliteStorage;

// ── Partition ID 生成 ──

/// approval 层中 Agent 分区的 ID
pub fn approval_agent_partition_id(agent_id: &AgentInstanceId) -> PartitionId {
    let uuid = uuid::Uuid::from_u128(0x3000_0000_0000_0000_0000_0000_0000_0000);
    let bytes = uuid.as_bytes();
    let agent_bytes = agent_id.0.as_bytes();
    let mut new_bytes = *bytes;
    for (i, b) in agent_bytes.iter().enumerate().take(16) {
        new_bytes[i] = new_bytes[i].wrapping_add(*b);
    }
    uuid::Uuid::from_bytes(new_bytes)
}

/// Integrated 分区的 ID
pub fn integrated_partition_id(name: &str) -> PartitionId {
    let uuid = uuid::Uuid::from_u128(0x4000_0000_0000_0000_0000_0000_0000_0000);
    let bytes = uuid.as_bytes();
    let name_bytes = name.as_bytes();
    let mut new_bytes = *bytes;
    for (i, b) in name_bytes.iter().enumerate().take(16) {
        new_bytes[i] = new_bytes[i].wrapping_add(*b);
    }
    uuid::Uuid::from_bytes(new_bytes)
}

/// Unified 分区的固定 ID
pub fn unified_partition_id() -> PartitionId {
    uuid::Uuid::from_u128(0x5000_0000_0000_0000_0000_0000_0000_0000)
}

// ── 分区创建 ──

/// 获取或创建 approval 层的 Agent 分区
pub fn ensure_approval_agent_partition(
    storage: &SqliteStorage,
    agent_id: &AgentInstanceId,
    initial_snapshot_id: SnapshotId,
) -> Result<Partition> {
    let pid = approval_agent_partition_id(agent_id);
    match storage.get_partition(&pid) {
        Ok(p) => Ok(p),
        Err(_) => {
            let partition = Partition::new(
                format!("approval/{}", agent_id),
                PartitionType::Approval(agent_id.clone()),
                initial_snapshot_id,
            );
            storage
                .create_partition(&partition)
                .map_err(|e| StratumError::Storage(e.into()))?;
            Ok(partition)
        }
    }
}

/// 获取或创建 Integrated 分区
pub fn ensure_integrated_partition(
    storage: &SqliteStorage,
    name: &str,
    initial_snapshot_id: SnapshotId,
) -> Result<Partition> {
    let pid = integrated_partition_id(name);
    match storage.get_partition(&pid) {
        Ok(p) => Ok(p),
        Err(_) => {
            let partition = Partition::new(
                format!("integrated/{}", name),
                PartitionType::Integrated(name.to_string()),
                initial_snapshot_id,
            );
            storage
                .create_partition(&partition)
                .map_err(|e| StratumError::Storage(e.into()))?;
            Ok(partition)
        }
    }
}

/// 获取或创建 Unified 分区
pub fn ensure_unified_partition(
    storage: &SqliteStorage,
    initial_snapshot_id: SnapshotId,
) -> Result<Partition> {
    let pid = unified_partition_id();
    match storage.get_partition(&pid) {
        Ok(p) => Ok(p),
        Err(_) => {
            let partition = Partition::new(
                "unified".to_string(),
                PartitionType::Unified,
                initial_snapshot_id,
            );
            storage
                .create_partition(&partition)
                .map_err(|e| StratumError::Storage(e.into()))?;
            Ok(partition)
        }
    }
}

// ── 正向迁移操作 ──

/// 将 Agent Approval 分区的内容迁移到 Integrated 分区
///
/// 从 approval 层的 Agent 分区取当前快照，合并到指定名称的 Integrated 分区。
pub fn move_approval_to_integrated(
    storage: &SqliteStorage,
    agent_id: &AgentInstanceId,
    integrated_name: &str,
) -> Result<SnapshotId> {
    let approval_pid = approval_agent_partition_id(agent_id);
    let integrated_pid = integrated_partition_id(integrated_name);

    let approval_partition = storage
        .get_partition(&approval_pid)
        .map_err(|_| StratumError::NotFound(format!("approval agent partition {} not found", agent_id)))?;
    let integrated_partition = storage
        .get_partition(&integrated_pid)
        .map_err(|_| StratumError::NotFound(format!("integrated partition {} not found", integrated_name)))?;

    let approval_snapshot = storage
        .get_snapshot(&approval_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;
    let integrated_snapshot = storage
        .get_snapshot(&integrated_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    // 合并
    let approval_text =
        crate::state_machine::transition::reconstruct_text(storage, &approval_snapshot)?;
    let integrated_text =
        crate::state_machine::transition::reconstruct_text(storage, &integrated_snapshot)?;

    let merge_diff = diff_to_line_diff(&integrated_text, &approval_text);
    if merge_diff.is_empty() {
        return Ok(integrated_partition.current_snapshot);
    }

    let merge_delta = Delta::new(
        approval_snapshot.file.clone(),
        merge_diff,
        SourceType::Agent(agent_id.clone()),
    );
    storage
        .store_delta(&merge_delta)
        .map_err(|e| StratumError::Storage(e.into()))?;

    let new_snapshot = Snapshot::merge(
        vec![&integrated_snapshot, &approval_snapshot],
        merge_delta.id,
        PartitionType::Integrated(integrated_name.to_string()).name(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(|e| StratumError::Storage(e.into()))?;

    storage
        .update_pointer(&integrated_pid, &new_snapshot.id)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(new_snapshot.id)
}

/// 将多个 Integrated 分区合并到 Unified 分区
pub fn move_integrated_to_unified(
    storage: &SqliteStorage,
    integrated_names: &[String],
) -> Result<SnapshotId> {
    let unified_pid = unified_partition_id();
    let unified_partition = storage
        .get_partition(&unified_pid)
        .map_err(|_| StratumError::NotFound("unified partition not found".into()))?;
    let unified_snapshot = storage
        .get_snapshot(&unified_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    let unified_text =
        crate::state_machine::transition::reconstruct_text(storage, &unified_snapshot)?;
    let mut merged_text = unified_text.clone();
    let mut parent_snapshots_owned: Vec<Snapshot> = Vec::new();

    for name in integrated_names {
        let pid = integrated_partition_id(name);
        let part = storage
            .get_partition(&pid)
            .map_err(|_| StratumError::NotFound(format!("integrated partition {} not found", name)))?;
        let snap = storage
            .get_snapshot(&part.current_snapshot)
            .map_err(|e| StratumError::Storage(e.into()))?;
        let text = crate::state_machine::transition::reconstruct_text(storage, &snap)?;

        // 累加合并
        let diff = diff_to_line_diff(&merged_text, &text);
        if !diff.is_empty() {
            // 应用 diff 到 merged_text
            let delta = Delta::new(
                snap.file.clone(),
                diff,
                SourceType::Manual,
            );
            storage
                .store_delta(&delta)
                .map_err(|e| StratumError::Storage(e.into()))?;
            // 使用 apply_deltas 更新 merged_text
            merged_text = crate::engine::merge::apply_deltas(&merged_text, &[delta])
                .map_err(|e| StratumError::Engine(e.to_string()))?;
        }
        parent_snapshots_owned.push(snap);
    }

    // 如果没有变化，返回当前 unified 快照
    if parent_snapshots_owned.is_empty() {
        return Ok(unified_partition.current_snapshot);
    }

    // 构建引用集合用于 merge
    let mut all_parents: Vec<&Snapshot> = vec![&unified_snapshot];
    for snap in &parent_snapshots_owned {
        all_parents.push(snap);
    }

    // 创建最终合并 diff
    let final_diff = diff_to_line_diff(
        &crate::state_machine::transition::reconstruct_text(storage, &unified_snapshot)?,
        &merged_text,
    );
    let merge_delta = Delta::new(
        unified_snapshot.file.clone(),
        final_diff,
        SourceType::Manual,
    );
    storage
        .store_delta(&merge_delta)
        .map_err(|e| StratumError::Storage(e.into()))?;

    let new_snapshot = Snapshot::merge(
        all_parents,
        merge_delta.id,
        PartitionType::Unified.name(),
    );
    storage
        .store_snapshot(&new_snapshot, b"")
        .map_err(|e| StratumError::Storage(e.into()))?;

    storage
        .update_pointer(&unified_pid, &new_snapshot.id)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(new_snapshot.id)
}

/// AGENT_RAW ↔ INTEGRATED ↔ UNIFIED 双向迁移（仅切换指针）
///
/// 将 from 分区的 current_snapshot 指针复制到 to 分区。
pub fn migrate_between_partitions(
    storage: &SqliteStorage,
    from_partition_id: &PartitionId,
    to_partition_id: &PartitionId,
) -> Result<()> {
    let from_partition = storage
        .get_partition(from_partition_id)
        .map_err(|_| StratumError::NotFound("source partition not found".into()))?;

    storage
        .update_pointer(to_partition_id, &from_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::Delta;
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
        let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        let snapshot = Snapshot::new_initial(file_node, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();
        snapshot.id
    }

    #[test]
    fn test_migrate_between_partitions() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        let agent_id = AgentInstanceId("test-agent".into());
        let approval_pid = approval_agent_partition_id(&agent_id);
        let integrated_name = "integrated-1";
        let integrated_pid = integrated_partition_id(integrated_name);

        // 创建分区
        let approval_part = Partition::new(
            "approval/test-agent".into(),
            PartitionType::Approval(agent_id.clone()),
            initial_id,
        );
        storage.create_partition(&approval_part).unwrap();

        let integrated_part = Partition::new(
            format!("integrated/{}", integrated_name),
            PartitionType::Integrated(integrated_name.to_string()),
            initial_id,
        );
        storage.create_partition(&integrated_part).unwrap();

        // 创建新快照用于 approval
        let new_snap = {
            let snap = storage.get_snapshot(&initial_id).unwrap();
            let s = Snapshot::from_parent(
                &snap,
                Delta::new(
                    FileNode::new(std::path::PathBuf::from("test.txt"), b"base\n"),
                    crate::core::delta::LineDiff::new(vec![]),
                    SourceType::Manual,
                )
                .id,
                PartitionType::Approval(agent_id.clone()).name(),
            );
            storage.store_snapshot(&s, b"").unwrap();
            s
        };
        storage.update_pointer(&approval_pid, &new_snap.id).unwrap();

        // 迁移指针
        migrate_between_partitions(&storage, &approval_pid, &integrated_pid).unwrap();
        let integrated = storage.get_partition(&integrated_pid).unwrap();
        assert_eq!(integrated.current_snapshot, new_snap.id);
    }
}
