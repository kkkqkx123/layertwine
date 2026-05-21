//! Inter-layer flow logic
//!
//! Define all allowed forward/reverse flow operations, and state machine irony checks.
//! Flow rules reference architecture/03-hierarchical-state-machines.md §3.4 Iron laws of state machines.

use crate::core::snapshot::Snapshot;
use crate::core::types::{LayerType, PartitionId, SnapshotId};
use crate::engine::merge::apply_deltas;
use crate::error::{Result, StratumError};
use crate::storage::repository::{DeltaStore, FileNodeStore, PartitionStore, SnapshotStore};
use crate::storage::sqlite_storage::SqliteStorage;

// ===== Allowable Direction of Flow =====

/// Positive flow type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardTransition {
    /// manual_edit → staged
    ManualToStaged,
    /// agent_edit → approval (Agent Raw → Agent Approval)
    AgentToApproval,
    /// approval → integrated
    ApprovalToIntegrated,
    /// integrated → unified
    IntegratedToUnified,
    /// approval → staged
    ApprovalToStaged,
}

/// Type of reverse flow
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollbackTransition {
    /// staged → manual_edit
    StagedToManual,
    /// staged → approval
    StagedToApproval,
    /// staged → agent_edit
    StagedToAgentRaw,
    /// approval → agent_edit
    ApprovalToAgentRaw,
}

// ===== Iron Law Check =====

/// Check if positive flow is allowed
///
/// Iron rule 1: No cross-layer flows - all flows must pass through neighboring layers
pub fn check_forward_valid(from: &LayerType, to: &LayerType) -> Result<()> {
    let valid = match (from, to) {
        (LayerType::ManualEdit, LayerType::Staged) => true,
        (LayerType::AgentEdit, LayerType::Approval) => true,
        (LayerType::Approval, LayerType::Staged) => true,
        _ => false,
    };

    if !valid {
        return Err(StratumError::StateMachine(format!(
            "Ironclad check failed: impermissible cross-layer flow {:?} → {:?}",
            from, to
        )));
    }
    Ok(())
}

/// Check if reverse fallback is allowed
///
/// Ironclad Rule #2: No Reverse Writes - Fallback Only Toggles Pointer
pub fn check_rollback_valid(from: &LayerType, to: &LayerType) -> Result<()> {
    let valid = match (from, to) {
        (LayerType::Staged, LayerType::ManualEdit) => true,
        (LayerType::Staged, LayerType::AgentEdit) => true,
        (LayerType::Staged, LayerType::Approval) => true,
        (LayerType::Approval, LayerType::AgentEdit) => true,
        _ => false,
    };

    if !valid {
        return Err(StratumError::StateMachine(format!(
            "Ironclad check failed: impermissible cross-level fallback {:?} → {:?}",
            from, to
        )));
    }
    Ok(())
}

// ===== Forward transitions =====

/// Implementation of positive flow
///
/// Automatically schedules operation functions to each layer based on the ForwardTransition type.
pub fn execute_forward(
    storage: &SqliteStorage,
    transition: ForwardTransition,
    params: &[&str], // Optional parameters: agent_id, integrated_name, etc.
) -> Result<SnapshotId> {
    match transition {
        ForwardTransition::ManualToStaged => {
            check_forward_valid(&LayerType::ManualEdit, &LayerType::Staged)?;
            crate::state_machine::manual::merge_manual_to_staged(storage)
        }
        ForwardTransition::AgentToApproval => {
            check_forward_valid(&LayerType::AgentEdit, &LayerType::Approval)?;
            let agent_id = params.first().ok_or_else(|| {
                StratumError::StateMachine("AgentToApproval requires agent_id parameter".into())
            })?;
            crate::state_machine::agent::move_agent_to_approval(
                storage,
                &crate::core::types::AgentInstanceId(agent_id.to_string()),
            )
        }
        ForwardTransition::ApprovalToIntegrated => {
            let agent_id = params.first().ok_or_else(|| {
                StratumError::StateMachine("ApprovalToIntegrated requires agent_id parameter".into())
            })?;
            let integrated_name = params.get(1).ok_or_else(|| {
                StratumError::StateMachine("ApprovalToIntegrated requires integrated_name parameter".into())
            })?;
            crate::state_machine::approval::move_approval_to_integrated(
                storage,
                &crate::core::types::AgentInstanceId(agent_id.to_string()),
                integrated_name,
            )
        }
        ForwardTransition::IntegratedToUnified => {
            // integrated_names passed in via params, separated by commas
            let names_str = params.first().ok_or_else(|| {
                StratumError::StateMachine("IntegratedToUnified requires integrated_names parameter".into())
            })?;
            let names: Vec<String> = names_str
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();
            crate::state_machine::approval::move_integrated_to_unified(storage, &names)
        }
        ForwardTransition::ApprovalToStaged => {
            check_forward_valid(&LayerType::Approval, &LayerType::Staged)?;
            let approval_partition_id_str = params.first().ok_or_else(|| {
                StratumError::StateMachine("ApprovalToStaged requires approval_partition_id parameter".into())
            })?;
            // Parsing UUIDs
            let pid = uuid::Uuid::parse_str(approval_partition_id_str)
                .map_err(|_| StratumError::StateMachine("invalid partition_id UUID".into()))?;
            crate::state_machine::staged::merge_approval_to_staged(storage, &pid)
        }
    }
}

// ===== Rollback operations =====

/// Partition itself back: current = history.pop()
///
/// Corresponds to rollback_partition in architecture document §3.3.
/// Only switches pointers and does not modify any immutable data (Iron Law 2).
pub fn rollback_partition(
    storage: &SqliteStorage,
    partition_id: &PartitionId,
) -> Result<SnapshotId> {
    let partition = storage
        .get_partition(partition_id)
        .map_err(|_| StratumError::NotFound("partition not found".into()))?;

    if partition.history.len() <= 1 {
        return Err(StratumError::StateMachine(
            "cannot rollback: only one snapshot in history".into(),
        ));
    }

    let prev_id = partition.history[partition.history.len() - 2];
    storage
        .update_pointer(partition_id, &prev_id)
        .map_err(|e| StratumError::Storage(e.into()))?;

    Ok(prev_id)
}

/// Fallback staged to the specified layer
///
/// Finds the target layer source from the parents of the staged current snapshot.
/// Corresponds to rollback_staged_to_source in architecture document §3.3.
pub fn rollback_staged_to_layer(
    storage: &SqliteStorage,
    target_layer: LayerType,
) -> Result<SnapshotId> {
    let staged_pid = crate::state_machine::staged::staged_partition_id();
    let staged_partition = storage
        .get_partition(&staged_pid)
        .map_err(|_| StratumError::NotFound("staged partition not found".into()))?;

    let staged_snapshot = storage
        .get_snapshot(&staged_partition.current_snapshot)
        .map_err(|e| StratumError::Storage(e.into()))?;

    // Find the source of the target layer from staged parents
    let target_partition_type = match target_layer {
        LayerType::ManualEdit => "manual",
        LayerType::AgentEdit => "agent",
        LayerType::Approval => "approval",
        LayerType::Staged => "staged",
    };

    for parent_id in &staged_snapshot.parents {
        let parent_snapshot = storage.get_snapshot(parent_id)
            .map_err(|e| StratumError::Storage(e.into()))?;
        if parent_snapshot.partition_type.contains(target_partition_type) {
            // Switch the staged pointer to this parent
            storage
                .update_pointer(&staged_pid, parent_id)
                .map_err(|e| StratumError::Storage(e.into()))?;
            return Ok(*parent_id);
        }
    }

    Err(StratumError::NotFound(format!(
        "no parent found for target layer {:?} in staged snapshot parents",
        target_layer
    )))
}

/// Merge backup snapshots into staged (placeholder, P5 implements specific logic)
pub fn merge_backup_to_staged(
    _storage: &SqliteStorage,
    _backup_snapshot_id: &SnapshotId,
) -> Result<SnapshotId> {
    Err(StratumError::StateMachine(
        "backup merge not yet implemented in P3, see P5".into(),
    ))
}

// ===== Utility functions =====

/// Reconstructs the complete text content from Snapshot's delta chains
///
/// Read the original content from file_node and apply all deltas in turn.
pub fn reconstruct_text(
    storage: &SqliteStorage,
    snapshot: &Snapshot,
) -> Result<String> {
    let file_content = storage
        .get_file_content(&snapshot.file)
        .map_err(|e| StratumError::Storage(e.into()))?;
    let content_str = String::from_utf8_lossy(&file_content).to_string();

    let deltas = storage
        .get_deltas(&snapshot.deltas)
        .map_err(|e| StratumError::Storage(e.into()))?;

    apply_deltas(&content_str, &deltas)
        .map_err(|e| StratumError::Engine(e.to_string()))
}

/// Checks if the snapshot contains a parent of the specified partition_type.
pub fn has_parent_of_type(
    storage: &SqliteStorage,
    snapshot: &Snapshot,
    partition_type_prefix: &str,
) -> Result<bool> {
    for parent_id in &snapshot.parents {
        let parent = storage
            .get_snapshot(parent_id)
            .map_err(|e| StratumError::Storage(e.into()))?;
        if parent.partition_type.contains(partition_type_prefix) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::Delta;
    use crate::core::file_node::FileNode;
    use crate::core::partition::Partition;
    use crate::core::types::{PartitionType, SourceType};
    use crate::storage::sqlite_storage::SqliteStorage;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn setup_storage() -> Arc<SqliteStorage> {
        Arc::new(SqliteStorage::new_in_memory().unwrap())
    }

    fn create_initial_snapshot(storage: &SqliteStorage, content: &str) -> SnapshotId {
        let file_node = FileNode::new(PathBuf::from("test.txt"), content.as_bytes());
        storage.store_file_node(&file_node, content.as_bytes()).unwrap();
        let empty_diff = crate::core::delta::LineDiff::new(vec![]);
        let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        let snapshot = Snapshot::new_initial(file_node, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();
        snapshot.id
    }

    #[test]
    fn test_check_forward_valid() {
        // Permitted flows
        assert!(check_forward_valid(&LayerType::ManualEdit, &LayerType::Staged).is_ok());
        assert!(check_forward_valid(&LayerType::AgentEdit, &LayerType::Approval).is_ok());
        assert!(check_forward_valid(&LayerType::Approval, &LayerType::Staged).is_ok());

        // Prohibition of cross-layering
        assert!(check_forward_valid(&LayerType::AgentEdit, &LayerType::Staged).is_err());
        assert!(check_forward_valid(&LayerType::ManualEdit, &LayerType::Approval).is_err());
    }

    #[test]
    fn test_check_rollback_valid() {
        assert!(check_rollback_valid(&LayerType::Staged, &LayerType::ManualEdit).is_ok());
        assert!(check_rollback_valid(&LayerType::Staged, &LayerType::AgentEdit).is_ok());
        assert!(check_rollback_valid(&LayerType::Staged, &LayerType::Approval).is_ok());
        assert!(check_rollback_valid(&LayerType::Approval, &LayerType::AgentEdit).is_ok());

        assert!(check_rollback_valid(&LayerType::Approval, &LayerType::Staged).is_err());
    }

    #[test]
    fn test_rollback_partition() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "v1\n");
        let pid = crate::state_machine::staged::staged_partition_id();
        let partition = Partition::new("test".into(), PartitionType::Staged, initial_id);
        storage.create_partition(&partition).unwrap();

        // advance
        let file_node = FileNode::new(PathBuf::from("test.txt"), b"v1\n");
        let diff = crate::core::delta::LineDiff::new(vec![]);
        let delta = Delta::new(file_node, diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        let snap = storage.get_snapshot(&initial_id).unwrap();
        let s2 = Snapshot::from_parent(&snap, delta.id, "staged".to_string());
        storage.store_snapshot(&s2, b"").unwrap();
        storage.update_pointer(&pid, &s2.id).unwrap();

        // rollback
        let prev = rollback_partition(&storage, &pid).unwrap();
        assert_eq!(prev, initial_id);
    }

    #[test]
    fn test_reconstruct_text() {
        let storage = setup_storage();
        let file_node = FileNode::new(PathBuf::from("test.txt"), b"hello\n");
        storage.store_file_node(&file_node, b"hello\n").unwrap();

        // Create delta: modify "hello" to "hello world"
        let diff = crate::engine::diff::diff_to_line_diff("hello\n", "hello world\n");
        let delta = Delta::new(file_node.clone(), diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();

        let snapshot = Snapshot::new_initial(file_node, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();

        let text = reconstruct_text(&storage, &snapshot).unwrap();
        assert_eq!(text, "hello world");
    }
}
