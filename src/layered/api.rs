//! High-level API for common development scenarios
//!
//! This module provides simplified interfaces for common workflows:
//! - Single feature development with one agent
//! - Collaborative feature development with multiple agents
//! - Merging multiple features

use crate::core::types::{AgentInstanceId, SnapshotId};
use crate::error::Result;
use crate::layered::agent;
use crate::layered::integrated;
use crate::layered::staged;
use crate::layered::unified;
use crate::storage::repository::{
    BranchStore, CheckpointStore, DeltaStore, FileNodeStore, MetadataStore, PartitionStore,
    SnapshotStore,
};

/// Development scenario 1: Single feature with single agent
///
/// Complete workflow:
/// 1. Create feature branch from baseline
/// 2. Agent edits and submits
/// 3. Merge to feature
/// 4. Merge feature to unified
/// 5. Merge unified to staged
/// 6. Commit checkpoint
pub fn develop_single_feature<S>(
    storage: &S,
    feature_name: &str,
    agent_id: &AgentInstanceId,
    edit_fn: impl FnOnce(&str) -> Result<String>,
) -> Result<SnapshotId>
where
    S: SnapshotStore
        + DeltaStore
        + FileNodeStore
        + PartitionStore
        + BranchStore
        + MetadataStore
        + CheckpointStore,
{
    // 1. Get current baseline
    let baseline = get_current_baseline(storage)?;

    // 2. Create feature branch
    integrated::create_feature_branch(storage, feature_name, baseline.id)?;

    // 3. Agent edits
    let baseline_text = crate::layered::transition::reconstruct_text(storage, &baseline)?;
    let new_text = edit_fn(&baseline_text)?;
    agent::apply_agent_edit(storage, agent_id, "test.txt", &new_text)?;

    // 4. Move to approval
    agent::move_agent_to_approval(storage, agent_id)?;

    // 5. Merge to feature
    let merge_result = integrated::merge_agent_to_feature(storage, agent_id, feature_name)?;
    if merge_result.has_conflicts() {
        return Err(crate::error::StratumError::General(format!(
            "Merge conflicts detected: {}",
            merge_result.format_conflicts()
        )));
    }

    // 6. Merge feature to unified
    let unified_result = unified::merge_features_to_unified(storage, &[feature_name.to_string()])?;
    if unified_result.has_conflicts() {
        return Err(crate::error::StratumError::General(format!(
            "Merge conflicts detected: {}",
            unified_result.format_conflicts()
        )));
    }

    // 7. Merge unified to staged
    staged::merge_unified_to_staged(storage)?;

    // 8. Return staged snapshot (checkpoint commit would be the next step)
    let staged_partition = storage
        .get_partition(&staged::staged_partition_id())
        .map_err(|_| crate::error::StratumError::NotFound("staged partition not found".into()))?;
    Ok(staged_partition.current_snapshot)
}

/// Development scenario 2: Single feature with multiple agents collaborating
///
/// Each agent edits independently, then their changes are merged into the feature.
pub fn develop_feature_with_collaboration<S>(
    storage: &S,
    feature_name: &str,
    agents: Vec<(AgentInstanceId, impl FnOnce(&str) -> Result<String>)>,
) -> Result<SnapshotId>
where
    S: SnapshotStore
        + DeltaStore
        + FileNodeStore
        + PartitionStore
        + BranchStore
        + MetadataStore
        + CheckpointStore,
{
    if agents.is_empty() {
        return Err(crate::error::StratumError::General(
            "至少需要一个agent".to_string(),
        ));
    }

    // 1. Get current baseline
    let baseline = get_current_baseline(storage)?;

    // 2. Create feature branch
    integrated::create_feature_branch(storage, feature_name, baseline.id)?;

    // 3. Each agent edits and submits
    for (agent_id, edit_fn) in agents {
        let baseline_text = crate::layered::transition::reconstruct_text(storage, &baseline)?;
        let new_text = edit_fn(&baseline_text)?;
        agent::apply_agent_edit(storage, &agent_id, "test.txt", &new_text)?;
        agent::move_agent_to_approval(storage, &agent_id)?;

        // Merge agent's work into feature
        let merge_result = integrated::merge_agent_to_feature(storage, &agent_id, feature_name)?;
        if merge_result.has_conflicts() {
            return Err(crate::error::StratumError::General(format!(
                "Merge conflicts for agent {}: {}",
                agent_id,
                merge_result.format_conflicts()
            )));
        }
    }

    // 4. Merge feature to unified
    let unified_result = unified::merge_features_to_unified(storage, &[feature_name.to_string()])?;
    if unified_result.has_conflicts() {
        return Err(crate::error::StratumError::General(format!(
            "Merge conflicts detected: {}",
            unified_result.format_conflicts()
        )));
    }

    // 5. Merge unified to staged
    staged::merge_unified_to_staged(storage)?;

    // 6. Return staged snapshot
    let staged_partition = storage
        .get_partition(&staged::staged_partition_id())
        .map_err(|_| crate::error::StratumError::NotFound("staged partition not found".into()))?;
    Ok(staged_partition.current_snapshot)
}

/// Development scenario 3: Merge multiple features
///
/// Multiple features are merged into unified, then to staged.
pub fn merge_multiple_features<S>(storage: &S, feature_names: &[String]) -> Result<SnapshotId>
where
    S: SnapshotStore
        + DeltaStore
        + FileNodeStore
        + PartitionStore
        + BranchStore
        + MetadataStore
        + CheckpointStore,
{
    if feature_names.is_empty() {
        return Err(crate::error::StratumError::General(
            "至少需要一个feature".to_string(),
        ));
    }

    // 1. Merge features to unified
    let unified_result = unified::merge_features_to_unified(storage, feature_names)?;
    if unified_result.has_conflicts() {
        return Err(crate::error::StratumError::General(format!(
            "Merge conflicts detected: {}",
            unified_result.format_conflicts()
        )));
    }

    // 2. Merge unified to staged
    staged::merge_unified_to_staged(storage)?;

    // 3. Return staged snapshot
    let staged_partition = storage
        .get_partition(&staged::staged_partition_id())
        .map_err(|_| crate::error::StratumError::NotFound("staged partition not found".into()))?;
    Ok(staged_partition.current_snapshot)
}

/// Get the current baseline snapshot
/// This is the snapshot that should be used as the base for new features
fn get_current_baseline<S>(storage: &S) -> Result<crate::core::snapshot::Snapshot>
where
    S: SnapshotStore + PartitionStore,
{
    // Try to get the staged partition first
    match storage.get_partition(&staged::staged_partition_id()) {
        Ok(staged) => storage
            .get_snapshot(&staged.current_snapshot)
            .map_err(crate::error::StratumError::Storage),
        Err(_) => {
            // If staged doesn't exist, try to get unified
            match storage.get_partition(&unified::unified_partition_id()) {
                Ok(unified) => storage
                    .get_snapshot(&unified.current_snapshot)
                    .map_err(crate::error::StratumError::Storage),
                Err(_) => Err(crate::error::StratumError::NotFound(
                    "No baseline found. Please initialize staged or unified first.".to_string(),
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::file_node::FileNode;
    use crate::core::types::SourceType;
    use crate::storage::SqliteStorage;

    fn setup_storage() -> SqliteStorage {
        SqliteStorage::new_in_memory().unwrap()
    }

    fn create_initial_snapshot(storage: &SqliteStorage, content: &str) -> SnapshotId {
        let file_node = FileNode::new(std::path::PathBuf::from("test.txt"), content.as_bytes());
        storage
            .store_file_node(&file_node, content.as_bytes())
            .unwrap();
        let empty_diff = crate::core::types::LineDiff::new(vec![]);
        let delta =
            crate::core::delta::Delta::new(file_node.clone(), empty_diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        let snapshot = crate::core::snapshot::Snapshot::new_initial(file_node, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();
        snapshot.id
    }

    #[test]
    fn test_develop_single_feature() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\n");

        staged::ensure_staged_partition(&storage, initial_id).unwrap();

        let agent_id = AgentInstanceId("test-agent".into());

        // Ensure agent partition exists
        agent::ensure_agent_partition(&storage, &agent_id, initial_id).unwrap();

        // Ensure approval partition exists
        crate::layered::approval::ensure_approval_agent_partition(&storage, &agent_id, initial_id)
            .unwrap();

        let feature_name = "test-feature";

        let result = develop_single_feature(&storage, feature_name, &agent_id, |base| {
            Ok(format!("{}\nmodified\n", base))
        });

        if let Err(e) = &result {
            eprintln!("Error: {:?}", e);
        }
        assert!(result.is_ok());
    }

    #[test]
    fn test_develop_feature_with_collaboration() {
        let storage = setup_storage();
        let initial_id = create_initial_snapshot(&storage, "base\nmiddle\nend\n");

        staged::ensure_staged_partition(&storage, initial_id).unwrap();

        let agent_a = AgentInstanceId("agent-a".into());
        let agent_b = AgentInstanceId("agent-b".into());

        // Ensure agent partitions exist
        agent::ensure_agent_partition(&storage, &agent_a, initial_id).unwrap();
        agent::ensure_agent_partition(&storage, &agent_b, initial_id).unwrap();

        // Ensure approval partitions exist
        crate::layered::approval::ensure_approval_agent_partition(&storage, &agent_a, initial_id)
            .unwrap();
        crate::layered::approval::ensure_approval_agent_partition(&storage, &agent_b, initial_id)
            .unwrap();

        let feature_name = "collab-feature";

        type AgentFn = Box<dyn FnOnce(&str) -> Result<String>>;
        let agents: Vec<(AgentInstanceId, AgentFn)> = vec![
            (
                agent_a.clone(),
                Box::new(|base| {
                    // Agent A modifies the first line
                    let lines: Vec<&str> = base.lines().collect();
                    Ok(format!("modified by A\n{}\n{}\n", lines[1], lines[2]))
                }),
            ),
            (
                agent_b.clone(),
                Box::new(|base| {
                    // Agent B modifies the last line
                    let lines: Vec<&str> = base.lines().collect();
                    Ok(format!("{}\n{}\nmodified by B\n", lines[0], lines[1]))
                }),
            ),
        ];

        let result = develop_feature_with_collaboration(&storage, feature_name, agents);

        if let Err(e) = &result {
            eprintln!("Error: {:?}", e);
        }
        assert!(result.is_ok());
    }
}
