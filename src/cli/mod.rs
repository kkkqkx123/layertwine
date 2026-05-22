pub mod commands;
pub mod output;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::backup::backup_repo::BackupRepo;
use crate::checkpoint::repo::CheckpointRepo;
use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::snapshot::Snapshot;
use crate::core::types::{
    AgentInstanceId, ContentId, PartitionType, SnapshotId, SourceType,
};
use crate::error::{exit_codes, Result, StratumError};
use crate::git_sync::gc::collect_garbage;
use crate::git_sync::git_bridge::GitBridge;
use crate::state_machine::StateMachine;
use crate::storage::repository::{
    BranchStore, CheckpointStore, DagStore, DeltaStore, FileNodeStore, PartitionStore,
    SnapshotStore,
};
use crate::storage::sqlite_storage::SqliteStorage;

use commands::*;
use output::{print_branches, print_log, print_progress, print_status, print_done, OutputFormat};

/// Run the CLI with the given arguments and return an exit code
pub fn run() -> i32 {
    run_with_cli(commands::parse_args())
}

/// Run the CLI with a pre-built Cli struct (useful for testing)
pub fn run_with_cli(cli: Cli) -> i32 {
    let format = if cli.json {
        OutputFormat::Json
    } else {
        OutputFormat::Plain
    };

    // Handle init specially because it creates the database
    match &cli.command {
        Commands::Init { git_ref } => {
            return match execute_init(&cli, git_ref.as_deref(), format) {
                Ok(()) => exit_codes::SUCCESS,
                Err(e) => {
                    eprintln!("{}", e.format_cli());
                    e.exit_code()
                }
            };
        }
        _ => {}
    }

    // For all other commands, open existing database
    let storage = match open_storage(&cli.db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", e.format_cli());
            return e.exit_code();
        }
    };

    let state_machine = StateMachine::new(storage.clone());

    match &cli.command {
        Commands::Init { .. } => unreachable!(), // handled above
        Commands::Status => execute_status(&state_machine, format),
        Commands::Edit { file, content } => execute_edit(storage, file, content.as_deref()),
        Commands::Agent {
            agent_id,
            action,
        } => execute_agent(storage, agent_id, action, format),
        Commands::Approve { agent_id } => execute_approve(storage, agent_id),
        Commands::Commit {
            message,
            author,
        } => execute_commit(storage, message, author),
        Commands::Log { count } => execute_log(storage, *count, format),
        Commands::Branch { action } => execute_branch(storage, action, format),
        Commands::Merge { branch, message } => execute_merge(storage, branch, message),
        Commands::Backup {
            snapshot_id,
            label,
        } => execute_backup(storage, snapshot_id, label.as_deref(), &cli.db_path),
        Commands::Restore { backup_id } => execute_restore(storage, backup_id, &cli.db_path),
        Commands::Gc => execute_gc(storage),
        Commands::Push {
            remote,
            message,
        } => execute_push(storage, &cli, remote, message),
        Commands::Pull {
            remote,
            git_ref,
        } => execute_pull(storage, &cli, remote, git_ref),
    }
}

fn open_storage(db_path: &str) -> Result<Arc<SqliteStorage>> {
    let path = Path::new(db_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| StratumError::General(format!("failed to create db directory: {}", e)))?;
    }
    let storage = SqliteStorage::new_full(path)
        .map_err(|e| StratumError::Storage(e))?;
    Ok(Arc::new(storage))
}

// Helper: build a CheckpointRepo from the SqliteStorage data
fn load_checkpoint_repo(storage: &SqliteStorage) -> Result<CheckpointRepo> {
    let checkpoints = storage
        .list_checkpoints()
        .map_err(|e| StratumError::Storage(e))?;
    let branches = storage
        .list_branches()
        .map_err(|e| StratumError::Storage(e))?;
    let dag = storage
        .load_dag()
        .map_err(|e| StratumError::Storage(e))?;

    let mut repo = CheckpointRepo::new_single(SnapshotId::from_content(b"dummy"));
    repo.branches.clear();
    repo.branches.extend(branches);
    repo.checkpoint_dag = dag;
    repo.checkpoints.clear();
    for cp in checkpoints {
        repo.checkpoints.insert(cp.id, cp);
    }
    if repo.branches.is_empty() {
        repo.branches
            .push(crate::checkpoint::branch::Branch::new(
                "main",
                ContentId::from_content(b"root"),
            ));
    }
    repo.current_branch = 0;
    Ok(repo)
}

fn execute_init(cli: &Cli, git_ref: Option<&str>, format: OutputFormat) -> Result<()> {
    print_progress("Initializing stratum repository");
    let storage = open_storage(&cli.db_path)?;

    if let Some(git_repo_path) = &cli.git_repo {
        let git_path = Path::new(git_repo_path);
        let ref_name = git_ref.unwrap_or("HEAD");
        print_done();

        print_progress("Importing from Git repository");
        let initial_snapshot = ContentId::from_content(b"stratum-git-init-placeholder");
        let mut checkpoint_repo = CheckpointRepo::new_single(initial_snapshot);

        GitBridge::init_from_git(git_path, &*storage, &mut checkpoint_repo, ref_name)?;
        // Persist the checkpoint repo data back to storage
        for cp in checkpoint_repo.checkpoints.values() {
            storage
                .store_checkpoint(cp)
                .map_err(|e| StratumError::Storage(e))?;
        }
        for branch in &checkpoint_repo.branches {
            storage
                .store_branch(branch)
                .map_err(|e| StratumError::Storage(e))?;
        }
        storage
            .store_dag(&checkpoint_repo.checkpoint_dag)
            .map_err(|e| StratumError::Storage(e))?;
        print_done();

        if format == OutputFormat::Plain {
            println!(
                "Initialized stratum repository from Git ref '{}'",
                ref_name
            );
        }
    } else {
        // Create initial empty state
        let file_node = FileNode::new(PathBuf::from(".stratum/init"), b"");
        storage
            .store_file_node(&file_node, b"")
            .map_err(|e| StratumError::Storage(e))?;
        let empty_diff = Delta::new(
            file_node.clone(),
            crate::core::delta::LineDiff::new(vec![]),
            SourceType::Manual,
        );
        storage
            .store_delta(&empty_diff)
            .map_err(|e| StratumError::Storage(e))?;
        let initial_snapshot = Snapshot::new_initial(file_node, empty_diff.id);
        storage
            .store_snapshot(&initial_snapshot, b"")
            .map_err(|e| StratumError::Storage(e))?;

        // Create default partitions
        let manual_partition =
            crate::state_machine::manual::ensure_manual_partition(storage.as_ref(), initial_snapshot.id)?;
        let staged_partition =
            crate::state_machine::staged::ensure_staged_partition(storage.as_ref(), initial_snapshot.id)?;

        // Initialize branch in storage
        let branch = crate::checkpoint::branch::Branch::new(
            "main",
            ContentId::from_content(b"stratum-root"),
        );
        storage
            .store_branch(&branch)
            .map_err(|e| StratumError::Storage(e))?;

        // Initialize DAG
        let dag = crate::checkpoint::dag::CheckpointDag::new();
        storage
            .store_dag(&dag)
            .map_err(|e| StratumError::Storage(e))?;

        print_done();

        if format == OutputFormat::Plain {
            println!("Initialized empty stratum repository at '{}'", cli.db_path);
            println!("  Manual partition: {}", manual_partition.id);
            println!("  Staged partition: {}", staged_partition.id);
            println!("  Branch: main");
        }
    }

    Ok(())
}

fn execute_status(state_machine: &StateMachine, format: OutputFormat) -> i32 {
    print_status(state_machine, format);
    exit_codes::SUCCESS
}

fn execute_edit(
    storage: Arc<SqliteStorage>,
    file: &str,
    content: Option<&str>,
) -> i32 {
    let has_content = content.is_some();
    let content = content.unwrap_or("");
    let result = crate::state_machine::manual::apply_manual_edit(
        storage.as_ref(),
        file,
        content,
    );
    match result {
        Ok(snapshot_id) => {
            if has_content {
                println!(
                    "Edited '{}' -> new snapshot {}",
                    file,
                    &snapshot_id.to_hex()[..12]
                );
            }
            // Merge manual to staged automatically
            match crate::state_machine::manual::merge_manual_to_staged(storage.as_ref()) {
                Ok(staged_id) => {
                    println!(
                        "Merged to staged -> snapshot {}",
                        &staged_id.to_hex()[..12]
                    );
                }
                Err(e) => {
                    eprintln!("Warning: failed to merge to staged: {}", e);
                }
            }
            exit_codes::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.format_cli());
            e.exit_code()
        }
    }
}

fn execute_agent(
    storage: Arc<SqliteStorage>,
    agent_id: &str,
    action: &AgentCommands,
    format: OutputFormat,
) -> i32 {
    let agent_instance = AgentInstanceId(agent_id.to_string());

    match action {
        AgentCommands::Edit { file, content } => {
            let has_content = content.is_some();
            let content = content.as_deref().unwrap_or("");

            // Ensure agent partition exists
            let initial_snapshot = {
                let staged_pid = crate::state_machine::staged::staged_partition_id();
                match storage.get_partition(&staged_pid) {
                    Ok(p) => p.current_snapshot,
                    Err(_) => {
                        // Fallback: create a dummy snapshot
                        let file_node = FileNode::new(
                            PathBuf::from(file),
                            content.as_bytes(),
                        );
                        storage
                            .store_file_node(&file_node, content.as_bytes())
                            .map_err(|e| StratumError::Storage(e))
                            .unwrap();
                        let delta = Delta::new(file_node, crate::core::delta::LineDiff::new(vec![]), SourceType::Agent(agent_instance.clone()));
                        storage.store_delta(&delta).map_err(|e| StratumError::Storage(e)).unwrap();
                        let snapshot = Snapshot::new_initial(
                            crate::core::file_node::FileNode::new(
                                PathBuf::from(file),
                                content.as_bytes(),
                            ),
                            delta.id,
                        );
                        storage.store_snapshot(&snapshot, content.as_bytes()).map_err(|e| StratumError::Storage(e)).unwrap();
                        snapshot.id
                    }
                }
            };

            let _ = crate::state_machine::agent::ensure_agent_partition(
                storage.as_ref(),
                &agent_instance,
                initial_snapshot,
            );

            match crate::state_machine::agent::apply_agent_edit(
                storage.as_ref(),
                &agent_instance,
                file,
                content,
            ) {
                Ok(snapshot_id) => {
                    if has_content {
                        match format {
                            OutputFormat::Plain => {
                                println!(
                                    "Agent '{}' edited '{}' -> snapshot {}",
                                    agent_id,
                                    file,
                                    &snapshot_id.to_hex()[..12]
                                );
                            }
                            OutputFormat::Json => {
                                let json = serde_json::json!({
                                    "action": "agent_edit",
                                    "agent_id": agent_id,
                                    "file": file,
                                    "snapshot": snapshot_id.to_hex(),
                                });
                                println!("{}", serde_json::to_string_pretty(&json).unwrap());
                            }
                        }
                    }
                    exit_codes::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e.format_cli());
                    e.exit_code()
                }
            }
        }
        AgentCommands::Submit => {
            // Ensure approval partition exists
            let staged_pid = crate::state_machine::staged::staged_partition_id();
            let base_snapshot = match storage.get_partition(&staged_pid) {
                Ok(p) => p.current_snapshot,
                Err(_) => {
                    eprintln!("error: no staged partition found. Make edits first.");
                    return exit_codes::GENERAL_ERROR;
                }
            };
            let _ = crate::state_machine::approval::ensure_approval_agent_partition(
                storage.as_ref(),
                &agent_instance,
                base_snapshot,
            );

            match crate::state_machine::agent::move_agent_to_approval(
                storage.as_ref(),
                &agent_instance,
            ) {
                Ok(snapshot_id) => {
                    match format {
                        OutputFormat::Plain => {
                            println!(
                                "Agent '{}' submitted for approval -> snapshot {}",
                                agent_id,
                                &snapshot_id.to_hex()[..12]
                            );
                        }
                        OutputFormat::Json => {
                            let json = serde_json::json!({
                                "action": "agent_submit",
                                "agent_id": agent_id,
                                "snapshot": snapshot_id.to_hex(),
                            });
                            println!("{}", serde_json::to_string_pretty(&json).unwrap());
                        }
                    }
                    exit_codes::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e.format_cli());
                    e.exit_code()
                }
            }
        }
    }
}

fn execute_approve(
    storage: Arc<SqliteStorage>,
    agent_id: &str,
) -> i32 {
    let agent_instance = AgentInstanceId(agent_id.to_string());

    // Step 1: Move from approval agent partition to integrated
    match crate::state_machine::approval::move_approval_to_integrated(
        storage.as_ref(),
        &agent_instance,
        agent_id,
    ) {
        Ok(integrated_id) => {
            println!(
                "Approved agent '{}' -> integrated snapshot {}",
                agent_id,
                &integrated_id.to_hex()[..12]
            );
        }
        Err(e) => {
            eprintln!("{}", e.format_cli());
            return e.exit_code();
        }
    }

    // Step 2: Collect all integrated partition names and merge to unified
    let integration_names: Vec<String> = storage
        .list_partitions()
        .ok()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| match &p.partition_type {
            PartitionType::Integrated(name) => Some(name.clone()),
            _ => None,
        })
        .collect();

    if !integration_names.is_empty() {
        if let Err(e) = crate::state_machine::approval::move_integrated_to_unified(
            storage.as_ref(),
            &integration_names,
        ) {
            eprintln!("{}", e.format_cli());
            return e.exit_code();
        }
    }

    // Step 3: Merge unified to staged
    match crate::state_machine::staged::merge_unified_to_staged(storage.as_ref()) {
        Ok(staged_id) => {
            println!(
                "Merged to staged -> snapshot {}",
                &staged_id.to_hex()[..12]
            );
        }
        Err(e) => {
            eprintln!("{}", e.format_cli());
            return e.exit_code();
        }
    }

    exit_codes::SUCCESS
}

fn execute_commit(storage: Arc<SqliteStorage>, message: &str, author: &str) -> i32 {
    match crate::state_machine::staged::commit_staged_to_checkpoint(
        storage.as_ref(),
        message,
        author,
    ) {
        Ok(cp_id) => {
            println!(
                "Committed checkpoint {}: {}",
                &cp_id.to_hex()[..12],
                message
            );
            exit_codes::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.format_cli());
            e.exit_code()
        }
    }
}

fn execute_log(storage: Arc<SqliteStorage>, count: usize, format: OutputFormat) -> i32 {
    match storage.list_checkpoints() {
        Ok(mut checkpoints) => {
            checkpoints.truncate(count);
            print_log(&checkpoints, format);
            exit_codes::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", StratumError::Storage(e).format_cli());
            exit_codes::GENERAL_ERROR
        }
    }
}

fn execute_branch(
    storage: Arc<SqliteStorage>,
    action: &BranchCommands,
    format: OutputFormat,
) -> i32 {
    match action {
        BranchCommands::List => {
            match storage.list_branches() {
                Ok(branches) => {
                    print_branches(&branches, None, format);
                    exit_codes::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", StratumError::Storage(e).format_cli());
                    exit_codes::GENERAL_ERROR
                }
            }
        }
        BranchCommands::Create { name } => {
            // Create a new branch at the current head
            match storage.list_branches() {
                Ok(branches) => {
                    if branches.iter().any(|b| b.name == *name) {
                        eprintln!("error: branch '{}' already exists", name);
                        return exit_codes::USAGE_ERROR;
                    }
                    // Find the latest checkpoint or use a dummy
                    let head = match storage.list_checkpoints() {
                        Ok(cps) if !cps.is_empty() => cps[0].id,
                        _ => {
                            eprintln!("error: no checkpoints yet. Make a commit first.");
                            return exit_codes::GENERAL_ERROR;
                        }
                    };
                    let branch = crate::checkpoint::branch::Branch::new(name, head);
                    match storage.store_branch(&branch) {
                        Ok(_) => {
                            println!("Created branch '{}' at {}", name, &head.to_hex()[..12]);
                            exit_codes::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("{}", StratumError::Storage(e).format_cli());
                            exit_codes::GENERAL_ERROR
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{}", StratumError::Storage(e).format_cli());
                    exit_codes::GENERAL_ERROR
                }
            }
        }
        BranchCommands::Switch { name } => {
            match storage.get_branch(name) {
                Ok(_branch) => {
                    let state_machine = StateMachine::new(storage.clone());
                    match state_machine.switch_branch(name) {
                        Ok(cp_id) => {
                            println!(
                                "Switched to branch '{}' at checkpoint {}",
                                name,
                                &cp_id.to_hex()[..12]
                            );
                            exit_codes::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("{}", e.format_cli());
                            e.exit_code()
                        }
                    }
                }
                Err(_) => {
                    eprintln!("error: branch '{}' not found", name);
                    exit_codes::USAGE_ERROR
                }
            }
        }
    }
}

fn execute_merge(storage: Arc<SqliteStorage>, branch: &str, message: &str) -> i32 {
    // Use CheckpointRepo for merging
    let mut repo = match load_checkpoint_repo(storage.as_ref()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", e.format_cli());
            return e.exit_code();
        }
    };

    let current_name = repo.current_branch_name().to_string();

    // Get the staged snapshot IDs for the merge
    let staged_pid = crate::state_machine::staged::staged_partition_id();
    let snapshot_ids = match storage.get_partition(&staged_pid) {
        Ok(p) => vec![p.current_snapshot],
        Err(_) => {
            eprintln!("error: staged partition not found");
            return exit_codes::GENERAL_ERROR;
        }
    };

    match repo.merge_branches(branch, snapshot_ids, message, "user") {
        Ok(cp_id) => {
            // Persist the updated repo
            for cp in repo.checkpoints.values() {
                let _ = storage.store_checkpoint(cp);
            }
            for branch in &repo.branches {
                let _ = storage.store_branch(branch);
            }
            let _ = storage.store_dag(&repo.checkpoint_dag);
            println!(
                "Merged '{}' into '{}' -> checkpoint {}",
                branch,
                current_name,
                &cp_id.to_hex()[..12]
            );
            exit_codes::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.format_cli());
            e.exit_code()
        }
    }
}

fn execute_backup(
    storage: Arc<SqliteStorage>,
    snapshot_id_hex: &str,
    label: Option<&str>,
    db_path: &str,
) -> i32 {
    let snapshot_id = match ContentId::from_hex(snapshot_id_hex) {
        Some(id) => id,
        None => {
            eprintln!("error: invalid snapshot ID '{}'", snapshot_id_hex);
            return exit_codes::USAGE_ERROR;
        }
    };

    let backup_path = Path::new(db_path).parent().unwrap_or(Path::new("."));
    let backup_db_path = backup_path.join("stratum-backup.db");
    let backup_repo = match BackupRepo::new(&backup_db_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", StratumError::Storage(e).format_cli());
            return exit_codes::GENERAL_ERROR;
        }
    };

    match backup_repo.backup_snapshot(storage.as_ref(), snapshot_id, label.map(|s| s.to_string())) {
        Ok(backup_id) => {
            if label.is_some() {
                println!(
                    "Backup {} created for snapshot {} (label: {})",
                    &backup_id.to_hex()[..12],
                    &snapshot_id.to_hex()[..12],
                    label.unwrap()
                );
            } else {
                println!(
                    "Backup {} created for snapshot {}",
                    &backup_id.to_hex()[..12],
                    &snapshot_id.to_hex()[..12]
                );
            }
            exit_codes::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.format_cli());
            e.exit_code()
        }
    }
}

fn execute_restore(storage: Arc<SqliteStorage>, backup_id_hex: &str, db_path: &str) -> i32 {
    let backup_id = match ContentId::from_hex(backup_id_hex) {
        Some(id) => id,
        None => {
            eprintln!("error: invalid backup ID '{}'", backup_id_hex);
            return exit_codes::USAGE_ERROR;
        }
    };

    let backup_path = Path::new(db_path).parent().unwrap_or(Path::new("."));
    let backup_db_path = backup_path.join("stratum-backup.db");
    let backup_repo = match BackupRepo::new(&backup_db_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", StratumError::Storage(e).format_cli());
            return exit_codes::GENERAL_ERROR;
        }
    };

    match backup_repo.get_backup(&backup_id) {
        Ok(backup) => {
            // Restore the backup: store deltas and create snapshots
            for delta in &backup.deltas {
                let _ = storage.store_delta(delta);
            }
            println!(
                "Restored backup {} -> file: {}",
                &backup_id.to_hex()[..12],
                backup.file.path_str()
            );
            exit_codes::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.format_cli());
            e.exit_code()
        }
    }
}

fn execute_gc(storage: Arc<SqliteStorage>) -> i32 {
    let mut repo = match load_checkpoint_repo(storage.as_ref()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", e.format_cli());
            return e.exit_code();
        }
    };

    print_progress("Running garbage collection");
    match collect_garbage(&mut repo) {
        Ok(stats) => {
            // Persist the updated DAG
            let _ = storage.store_dag(&repo.checkpoint_dag);
            print_done();
            println!(
                "GC complete: {} checkpoints removed, {} snapshots freed, {} bytes",
                stats.removed_checkpoints, stats.removed_snapshots, stats.freed_bytes
            );
            if stats.delta_chain_depth_triggered {
                println!("  Note: delta chain depth exceeded threshold ({})", stats.max_chain_depth);
            }
            exit_codes::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.format_cli());
            e.exit_code()
        }
    }
}

fn execute_push(
    storage: Arc<SqliteStorage>,
    cli: &Cli,
    remote: &str,
    message: &str,
) -> i32 {
    let git_repo_path = match &cli.git_repo {
        Some(p) => p.clone(),
        None => {
            eprintln!("error: --git-repo path is required for push");
            return exit_codes::USAGE_ERROR;
        }
    };

    let repo = match load_checkpoint_repo(storage.as_ref()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", e.format_cli());
            return e.exit_code();
        }
    };

    print_progress("Pushing to Git");
    match GitBridge::push_to_remote(
        storage.as_ref(),
        Path::new(&git_repo_path),
        &repo,
        repo.current_branch_name(),
        remote,
        message,
    ) {
        Ok(git_hash) => {
            print_done();
            println!("Pushed to remote '{}' (commit: {})", remote, git_hash);
            exit_codes::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.format_cli());
            e.exit_code()
        }
    }
}

fn execute_pull(
    storage: Arc<SqliteStorage>,
    cli: &Cli,
    remote: &str,
    git_ref: &str,
) -> i32 {
    let git_repo_path = match &cli.git_repo {
        Some(p) => p.clone(),
        None => {
            eprintln!("error: --git-repo path is required for pull");
            return exit_codes::USAGE_ERROR;
        }
    };

    print_progress("Fetching from Git remote");
    match GitBridge::fetch_from_remote(Path::new(&git_repo_path), remote) {
        Ok(_) => {
            print_done();
        }
        Err(e) => {
            eprintln!("{}", e.format_cli());
            return e.exit_code();
        }
    }

    // Now init from the fetched remote tracking branch or the specified ref
    print_progress("Importing from Git");
    let mut repo = match load_checkpoint_repo(storage.as_ref()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("warning: failed to load existing checkpoint repo: {}", e);
            eprintln!("  hint: creating a fresh checkpoint repo for the pull");
            let dummy_id = ContentId::from_content(b"stratum-pull");
            CheckpointRepo::new_single(dummy_id)
        }
    };

    match GitBridge::init_from_git(
        Path::new(&git_repo_path),
        storage.as_ref(),
        &mut repo,
        git_ref,
    ) {
        Ok(_) => {
            // Persist
            for cp in repo.checkpoints.values() {
                let _ = storage.store_checkpoint(cp);
            }
            for branch in &repo.branches {
                let _ = storage.store_branch(branch);
            }
            let _ = storage.store_dag(&repo.checkpoint_dag);
            print_done();
            println!("Pulled from remote '{}' ref '{}'", remote, git_ref);
            exit_codes::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", e.format_cli());
            e.exit_code()
        }
    }
}