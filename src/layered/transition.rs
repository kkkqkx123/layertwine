//! Inter-layer flow logic
//!
//! Define all allowed forward/reverse flow operations, and state machine irony checks.
//! Flow rules reference architecture/03-hierarchical-state-machines.md §3.4 Iron laws of state machines.

use crate::backup::backup_repo::BackupRepo;
use crate::core::snapshot::Snapshot;
use crate::core::types::{BackupId, LayerType, PartitionId, SnapshotId};
use crate::engine::merge::apply_deltas;
use crate::error::{Result, StratumError};
use crate::storage::repository::{DeltaStore, FileNodeStore, PartitionStore, SnapshotStore};

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
    let valid = matches!(
        (from, to),
        (LayerType::ManualEdit, LayerType::Staged)
            | (LayerType::AgentEdit, LayerType::Approval)
            | (LayerType::Approval, LayerType::Staged)
    );

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
    let valid = matches!(
        (from, to),
        (LayerType::Staged, LayerType::ManualEdit)
            | (LayerType::Staged, LayerType::AgentEdit)
            | (LayerType::Staged, LayerType::Approval)
            | (LayerType::Approval, LayerType::AgentEdit)
    );

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
pub fn execute_forward<S>(
    storage: &S,
    transition: ForwardTransition,
    params: &[&str], // Optional parameters: agent_id, integrated_name, etc.
) -> Result<SnapshotId>
where
    S: SnapshotStore + DeltaStore + FileNodeStore + PartitionStore,
{
    match transition {
        ForwardTransition::ManualToStaged => {
            check_forward_valid(&LayerType::ManualEdit, &LayerType::Staged)?;
            crate::layered::manual::merge_manual_to_staged(storage)
        }
        ForwardTransition::AgentToApproval => {
            check_forward_valid(&LayerType::AgentEdit, &LayerType::Approval)?;
            let agent_id = params.first().ok_or_else(|| {
                StratumError::StateMachine("AgentToApproval requires agent_id parameter".into())
            })?;
            crate::layered::agent::move_agent_to_approval(
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
            crate::layered::integrated::move_approval_to_integrated(
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
            crate::layered::integrated::move_integrated_to_unified(storage, &names)
        }
        ForwardTransition::ApprovalToStaged => {
            check_forward_valid(&LayerType::Approval, &LayerType::Staged)?;
            let approval_partition_id_str = params.first().ok_or_else(|| {
                StratumError::StateMachine("ApprovalToStaged requires approval_partition_id parameter".into())
            })?;
            // Parsing UUIDs
            let pid = uuid::Uuid::parse_str(approval_partition_id_str)
                .map_err(|_| StratumError::StateMachine("invalid partition_id UUID".into()))?;
            crate::layered::staged::merge_approval_to_staged(storage, &pid)
        }
    }
}

// ===== Rollback operations =====

/// Partition itself back: current = history.pop()
///
/// Corresponds to rollback_partition in architecture document §3.3.
/// Only switches pointers and does not modify any immutable data (Iron Law 2).
pub fn rollback_partition<S: PartitionStore>(
    storage: &S,
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
        .map_err(StratumError::Storage)?;

    Ok(prev_id)
}

/// Fallback staged to the specified layer
///
/// Finds the target layer source from the parents of the staged current snapshot.
/// Corresponds to rollback_staged_to_source in architecture document §3.3.
pub fn rollback_staged_to_layer<S>(
    storage: &S,
    target_layer: LayerType,
) -> Result<SnapshotId>
where
    S: SnapshotStore + PartitionStore,
{
    let staged_pid = crate::layered::staged::staged_partition_id();
    let staged_partition = storage
        .get_partition(&staged_pid)
        .map_err(|_| StratumError::NotFound("staged partition not found".into()))?;

    let staged_snapshot = storage
        .get_snapshot(&staged_partition.current_snapshot)
        .map_err(StratumError::Storage)?;

    // Find the source of the target layer from staged parents
    let target_partition_type = match target_layer {
        LayerType::ManualEdit => "manual",
        LayerType::AgentEdit => "agent",
        LayerType::Approval => "approval",
        LayerType::Staged => "staged",
    };

    for parent_id in &staged_snapshot.parents {
        let parent_snapshot = storage.get_snapshot(parent_id)
            .map_err(StratumError::Storage)?;
        if parent_snapshot.partition_type.contains(target_partition_type) {
            // Switch the staged pointer to this parent
            storage
                .update_pointer(&staged_pid, parent_id)
                .map_err(StratumError::Storage)?;
            return Ok(*parent_id);
        }
    }

    Err(StratumError::NotFound(format!(
        "no parent found for target layer {:?} in staged snapshot parents",
        target_layer
    )))
}

/// Merge backup snapshots into staged
///
/// Uses BackupRepo to restore a backup into the staged partition.
pub fn merge_backup_to_staged<S>(
    storage: &S,
    backup_repo: &BackupRepo,
    backup_id: &BackupId,
) -> Result<SnapshotId>
where
    S: SnapshotStore + DeltaStore + FileNodeStore + PartitionStore,
{
    backup_repo.merge_to_staged(backup_id, storage)
}

// ===== Utility functions =====

/// Reconstructs the complete text content from Snapshot's delta chains
///
/// Read the original content from file_node and apply all deltas in turn.
pub fn reconstruct_text<S>(
    storage: &S,
    snapshot: &Snapshot,
) -> Result<String>
where
    S: FileNodeStore + DeltaStore,
{
    let file_content = storage
        .get_file_content(snapshot.file.path_str(), &snapshot.file.base_hash)
        .map_err(StratumError::Storage)?;
    let content_str = String::from_utf8_lossy(&file_content).to_string();

    let deltas = storage
        .get_deltas(&snapshot.deltas)
        .map_err(StratumError::Storage)?;

    apply_deltas(&content_str, &deltas)
        .map_err(|e| StratumError::Engine(e.to_string()))
}

/// Checks if the snapshot contains a parent of the specified partition_type.
pub fn has_parent_of_type<S: SnapshotStore>(
    storage: &S,
    snapshot: &Snapshot,
    partition_type_prefix: &str,
) -> Result<bool> {
    for parent_id in &snapshot.parents {
        let parent = storage
            .get_snapshot(parent_id)
            .map_err(StratumError::Storage)?;
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
    use crate::core::snapshot::Snapshot;
    use crate::core::types::{PartitionType, SourceType, AgentInstanceId};
    use crate::engine::diff::diff_to_line_diff;
    use crate::storage::repository::{FileNodeStore, SnapshotStore, DeltaStore, PartitionStore};
    use crate::storage::sqlite_storage::SqliteStorage;
    use std::path::PathBuf;

    fn setup_storage() -> SqliteStorage {
        SqliteStorage::new_in_memory().unwrap()
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
    fn test_check_forward_invalid_all() {
        let valid_pairs = [
            (LayerType::ManualEdit, LayerType::Staged),
            (LayerType::AgentEdit, LayerType::Approval),
            (LayerType::Approval, LayerType::Staged),
        ];

        let all_pairs = [
            (LayerType::ManualEdit, LayerType::ManualEdit),
            (LayerType::ManualEdit, LayerType::AgentEdit),
            (LayerType::ManualEdit, LayerType::Approval),
            (LayerType::AgentEdit, LayerType::ManualEdit),
            (LayerType::AgentEdit, LayerType::AgentEdit),
            (LayerType::AgentEdit, LayerType::Staged),
            (LayerType::Approval, LayerType::ManualEdit),
            (LayerType::Approval, LayerType::AgentEdit),
            (LayerType::Approval, LayerType::Approval),
            (LayerType::Staged, LayerType::ManualEdit),
            (LayerType::Staged, LayerType::AgentEdit),
            (LayerType::Staged, LayerType::Approval),
            (LayerType::Staged, LayerType::Staged),
        ];

        for (from, to) in &all_pairs {
            let is_valid = valid_pairs.contains(&(from.clone(), to.clone()));
            let result = check_forward_valid(from, to);
            if is_valid {
                assert!(result.is_ok(), "expected OK for {:?} -> {:?}", from, to);
            } else {
                assert!(result.is_err(), "expected Err for {:?} -> {:?}", from, to);
            }
        }
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
    fn test_check_rollback_invalid_all() {
        let valid_pairs = [
            (LayerType::Staged, LayerType::ManualEdit),
            (LayerType::Staged, LayerType::AgentEdit),
            (LayerType::Staged, LayerType::Approval),
            (LayerType::Approval, LayerType::AgentEdit),
        ];

        let all_pairs = [
            (LayerType::ManualEdit, LayerType::ManualEdit),
            (LayerType::ManualEdit, LayerType::AgentEdit),
            (LayerType::ManualEdit, LayerType::Approval),
            (LayerType::ManualEdit, LayerType::Staged),
            (LayerType::AgentEdit, LayerType::ManualEdit),
            (LayerType::AgentEdit, LayerType::AgentEdit),
            (LayerType::AgentEdit, LayerType::Approval),
            (LayerType::AgentEdit, LayerType::Staged),
            (LayerType::Approval, LayerType::ManualEdit),
            (LayerType::Approval, LayerType::Approval),
            (LayerType::Approval, LayerType::Staged),
            (LayerType::Staged, LayerType::Staged),
        ];

        for (from, to) in &all_pairs {
            let is_valid = valid_pairs.contains(&(from.clone(), to.clone()));
            let result = check_rollback_valid(from, to);
            if is_valid {
                assert!(result.is_ok(), "expected OK for {:?} -> {:?}", from, to);
            } else {
                assert!(result.is_err(), "expected Err for {:?} -> {:?}", from, to);
            }
        }
    }

    #[test]
    fn test_rollback_partition() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "v1\n");
        let pid = crate::layered::staged::staged_partition_id();
        let partition = Partition {
            id: pid,
            name: "test".into(),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Staged,
        };
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
    fn test_rollback_partition_error() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "v1\n");
        let pid = crate::layered::staged::staged_partition_id();
        let partition = Partition {
            id: pid,
            name: "test".into(),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Staged,
        };
        storage.create_partition(&partition).unwrap();

        // Try rollback when history has only one entry
        let result = rollback_partition(&storage, &pid);
        assert!(result.is_err(), "should error when only one snapshot in history");
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

    #[test]
    fn test_execute_forward_manual_to_staged() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        crate::layered::manual::ensure_manual_partition(&storage, initial_id).unwrap();
        crate::layered::staged::ensure_staged_partition(&storage, initial_id).unwrap();
        crate::layered::manual::apply_manual_edit(&storage, "test.txt", "base\nmodified\n").unwrap();

        let result = execute_forward(
            &storage,
            ForwardTransition::ManualToStaged,
            &[],
        );
        assert!(result.is_ok());

        let staged = storage.get_partition(&crate::layered::staged::staged_partition_id()).unwrap();
        assert_ne!(staged.current_snapshot, initial_id);
    }

    #[test]
    fn test_has_parent_of_type() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        let file_node = FileNode::new(PathBuf::from("test.txt"), b"base\nmodified\n");
        storage.store_file_node(&file_node, b"base\nmodified\n").unwrap();
        let diff = diff_to_line_diff("base\n", "base\nmodified\n");
        let delta = Delta::new(file_node, diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();

        let parent_snap = storage.get_snapshot(&initial_id).unwrap();

        // Create a chain: parent_snap (type="") → manual_snap (type="manual_edit") → staged_snap (type="staged")
        let manual_snap = Snapshot::from_parent(&parent_snap, delta.id, "manual_edit".to_string());
        storage.store_snapshot(&manual_snap, b"").unwrap();

        let file_node2 = FileNode::new(PathBuf::from("test.txt"), b"base\n");
        storage.store_file_node(&file_node2, b"base\n").unwrap();
        let diff2 = diff_to_line_diff("base\nmodified\n", "base\n");
        let delta2 = Delta::new(file_node2, diff2, SourceType::Manual);
        storage.store_delta(&delta2).unwrap();
        let staged_snap = Snapshot::from_parent(&manual_snap, delta2.id, "staged".to_string());
        storage.store_snapshot(&staged_snap, b"").unwrap();

        // staged_snap has manual_snap as parent with "manual_edit" type
        let has_manual = has_parent_of_type(&storage, &staged_snap, "manual").unwrap();
        assert!(has_manual, "staged snapshot should have parent with 'manual'");

        // Initial snapshot (no partition type) should not match
        let has = has_parent_of_type(&storage, &parent_snap, "manual").unwrap();
        assert!(!has, "initial snapshot has no parent");
    }

    #[test]
    fn test_execute_forward_agent_to_approval() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");
        let agent_id = AgentInstanceId("test-agent".into());

        // Setup agent partition
        crate::layered::agent::ensure_agent_partition(&storage, &agent_id, initial_id).unwrap();
        crate::layered::agent::apply_agent_edit(&storage, &agent_id, "test.txt", "base\nmodified\n").unwrap();

        // Setup approval partition
        let approval_pid = crate::layered::approval::approval_agent_partition_id(&agent_id);
        let approval_part = Partition {
            id: approval_pid,
            name: format!("approval/{}", agent_id),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Approval(agent_id.clone()),
        };
        storage.create_partition(&approval_part).unwrap();

        let result = execute_forward(
            &storage,
            ForwardTransition::AgentToApproval,
            &["test-agent"],
        );
        assert!(result.is_ok());

        let approval = storage.get_partition(&approval_pid).unwrap();
        assert_ne!(approval.current_snapshot, initial_id);
    }

    #[test]
    fn test_rollback_staged_to_layer_not_found() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");
        crate::layered::staged::ensure_staged_partition(&storage, initial_id).unwrap();

        let result = rollback_staged_to_layer(&storage, LayerType::ManualEdit);
        assert!(result.is_err(), "should error when no suitable parent found");
    }

    #[test]
    fn test_execute_forward_approval_to_staged() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");
        crate::layered::staged::ensure_staged_partition(&storage, initial_id).unwrap();

        // Create approval partition with content change
        let agent_id = AgentInstanceId("test-agent".into());
        let approval_pid = crate::layered::approval::approval_agent_partition_id(&agent_id);
        let approval_part = Partition {
            id: approval_pid,
            name: format!("approval/{}", agent_id),
            current_snapshot: initial_id,
            history: vec![initial_id],
            partition_type: PartitionType::Approval(agent_id),
        };
        storage.create_partition(&approval_part).unwrap();

        // Create a new snapshot for the approval partition
        let file_node = FileNode::new(PathBuf::from("test.txt"), b"base\nmodified\n");
        storage.store_file_node(&file_node, b"base\nmodified\n").unwrap();
        let diff = diff_to_line_diff("base\n", "base\nmodified\n");
        let delta = Delta::new(file_node, diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        let snap = storage.get_snapshot(&initial_id).unwrap();
        let new_snap = Snapshot::from_parent(&snap, delta.id, "approval".to_string());
        storage.store_snapshot(&new_snap, b"").unwrap();
        storage.update_pointer(&approval_pid, &new_snap.id).unwrap();

        let result = execute_forward(
            &storage,
            ForwardTransition::ApprovalToStaged,
            &[&approval_pid.to_string()],
        );
        assert!(result.is_ok());
    }
}
