//! Common test utilities and helpers

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use stratum::api::cli::commands::{AgentCommands, BranchCommands, Cli, Commands};
use stratum::api::cli::run_with_cli;
use stratum::core::delta::Delta;
use stratum::core::file_node::FileNode;
use stratum::core::partition::Partition;
use stratum::core::snapshot::Snapshot;
use stratum::core::types::LineDiff;
use stratum::core::types::{ContentId, SnapshotId, SourceType};
use stratum::engine::merge::apply_deltas;
use stratum::storage::migrations;
use stratum::storage::repository::{DeltaStore, FileNodeStore, PartitionStore, SnapshotStore};
use stratum::storage::SqliteStorage;
use tempfile::TempDir;

/// Create an in-memory SqliteStorage with P1 tables only.
pub fn create_storage() -> SqliteStorage {
    SqliteStorage::new_in_memory().unwrap()
}

/// Create an in-memory SqliteStorage with full tables (P1 + checkpoint).
pub fn create_full_storage() -> SqliteStorage {
    let conn = Connection::open_in_memory().unwrap();
    migrations::initialize_full(&conn).unwrap();
    let conn = Arc::new(Mutex::new(conn));
    SqliteStorage::new_with_connection_arc(&conn)
}

/// Create an initial Snapshot for a file with the given content.
/// Returns the SnapshotId.
pub fn create_initial_snapshot(storage: &SqliteStorage, path: &str, content: &str) -> SnapshotId {
    let file_node = FileNode::new(PathBuf::from(path), content.as_bytes());
    storage
        .store_file_node(&file_node, content.as_bytes())
        .unwrap();
    let empty_diff = LineDiff::new(vec![]);
    let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Manual);
    storage.store_delta(&delta).unwrap();
    let snapshot = Snapshot::new_initial(file_node, delta.id);
    storage.store_snapshot(&snapshot, b"").unwrap();
    snapshot.id
}

/// Initialize a minimal repository with manual and staged partitions.
/// Returns (initial_snapshot_id, manual_partition, staged_partition).
pub fn init_repo(
    storage: &SqliteStorage,
    path: &str,
    content: &str,
) -> (SnapshotId, Partition, Partition) {
    let snapshot_id = create_initial_snapshot(storage, path, content);
    let manual = stratum::layered::manual::ensure_manual_partition(storage, snapshot_id).unwrap();
    let staged = stratum::layered::staged::ensure_staged_partition(storage, snapshot_id).unwrap();
    (snapshot_id, manual, staged)
}

/// Reconstruct the full text content of a Snapshot by resolving its Delta chain.
pub fn reconstruct_text(storage: &SqliteStorage, snapshot_id: &SnapshotId) -> String {
    let snapshot = storage.get_snapshot(snapshot_id).unwrap();
    let file_content = storage
        .get_file_content(snapshot.file.path_str(), &snapshot.file.base_hash)
        .unwrap();
    let content_str = String::from_utf8_lossy(&file_content).to_string();
    let deltas = storage.get_deltas(&snapshot.deltas).unwrap();
    apply_deltas(&content_str, &deltas).unwrap()
}

/// Create a simple non-empty Delta (single insert op).
pub fn make_insert_delta(file: &FileNode, line: &str) -> Delta {
    let hunk = stratum::core::types::Hunk {
        old_start: 1,
        old_len: 0,
        new_start: 1,
        new_len: 1,
        ops: vec![stratum::core::types::DiffOp::Insert {
            new_start: 1,
            lines: vec![line.to_string()],
        }],
    };
    let diff = LineDiff::new(vec![hunk]);
    Delta::new(file.clone(), diff, SourceType::Manual)
}

/// Create a checkpoint for use in tests that need checkpoint tables.
pub fn make_checkpoint_id(data: &[u8]) -> stratum::core::types::CheckpointId {
    ContentId::from_content(data)
}

/// E2E test fixture: manages a temporary directory and database path.
pub struct E2eFixture {
    dir: TempDir,
    db_path: PathBuf,
}

impl E2eFixture {
    pub fn new() -> Self {
        let dir = TempDir::new().expect("failed to create temp dir");
        let db_path = dir.path().join(".stratum").join("stratum.db");
        Self { dir, db_path }
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn db_path_str(&self) -> &str {
        self.db_path.to_str().unwrap()
    }

    pub fn workspace_path(&self) -> &Path {
        self.dir.path()
    }

    /// Open the storage for state verification.
    pub fn open_storage(&self) -> Arc<SqliteStorage> {
        let storage = SqliteStorage::new_full(self.db_path())
            .expect("failed to open storage for verification");
        Arc::new(storage)
    }

    /// Create a StateMachine for state verification.
    pub fn open_state_machine(
        &self,
    ) -> stratum::layered::StateMachine<stratum::storage::SqliteStorage> {
        stratum::layered::StateMachine::new(self.open_storage())
    }
}

/// Run a single CLI command against the given database path.
/// Returns the exit code.
pub fn run_cmd(db_path: &str, command: Commands) -> i32 {
    let cli = Cli {
        db_path: db_path.to_string(),
        git_repo: None,
        json: false,
        command,
    };
    run_with_cli(cli)
}

/// Run a CLI command with JSON output mode.
/// Returns the exit code.
pub fn run_cmd_json(db_path: &str, command: Commands) -> i32 {
    let cli = Cli {
        db_path: db_path.to_string(),
        git_repo: None,
        json: true,
        command,
    };
    run_with_cli(cli)
}

/// Run a CLI command with optional git-repo path.
pub fn run_cmd_with_git(db_path: &str, git_repo: &str, command: Commands) -> i32 {
    let cli = Cli {
        db_path: db_path.to_string(),
        git_repo: Some(git_repo.to_string()),
        json: false,
        command,
    };
    run_with_cli(cli)
}

/// Get the staged partition current snapshot.
pub fn staged_snapshot_id(storage: &SqliteStorage) -> Option<SnapshotId> {
    let staged_pid = stratum::layered::staged::staged_partition_id();
    match storage.get_partition(&staged_pid) {
        Ok(p) => Some(p.current_snapshot),
        Err(_) => None,
    }
}

/// Get the staged partition current snapshot hex string.
pub fn staged_snapshot_hex(storage: &SqliteStorage) -> Option<String> {
    staged_snapshot_id(storage).map(|id| id.to_hex())
}

pub mod cli {
    use super::*;

    pub fn cmd_init() -> Commands {
        Commands::Init { git_ref: None }
    }

    pub fn cmd_status() -> Commands {
        Commands::Status
    }

    pub fn cmd_edit(file: &str, content: &str) -> Commands {
        Commands::Edit {
            file: file.to_string(),
            content: Some(content.to_string()),
        }
    }

    pub fn cmd_agent_edit(agent_id: &str, file: &str, content: &str) -> Commands {
        Commands::Agent {
            agent_id: agent_id.to_string(),
            action: AgentCommands::Edit {
                file: file.to_string(),
                content: Some(content.to_string()),
            },
        }
    }

    pub fn cmd_agent_submit(agent_id: &str) -> Commands {
        Commands::Agent {
            agent_id: agent_id.to_string(),
            action: AgentCommands::Submit,
        }
    }

    pub fn cmd_approve(agent_id: &str) -> Commands {
        Commands::Approve {
            agent_id: agent_id.to_string(),
        }
    }

    pub fn cmd_commit(message: &str, author: &str) -> Commands {
        Commands::Commit {
            message: message.to_string(),
            author: author.to_string(),
        }
    }

    pub fn cmd_log() -> Commands {
        Commands::Log { count: 20 }
    }

    pub fn cmd_branch_create(name: &str) -> Commands {
        Commands::Branch {
            action: BranchCommands::Create {
                name: name.to_string(),
            },
        }
    }

    pub fn cmd_branch_switch(name: &str) -> Commands {
        Commands::Branch {
            action: BranchCommands::Switch {
                name: name.to_string(),
            },
        }
    }

    pub fn cmd_branch_list() -> Commands {
        Commands::Branch {
            action: BranchCommands::List,
        }
    }

    pub fn cmd_merge(branch: &str, message: &str) -> Commands {
        Commands::Merge {
            branch: branch.to_string(),
            message: message.to_string(),
        }
    }

    pub fn cmd_backup(snapshot_id: &str, label: Option<&str>) -> Commands {
        Commands::Backup {
            snapshot_id: snapshot_id.to_string(),
            label: label.map(|s| s.to_string()),
        }
    }

    pub fn cmd_restore(backup_id: &str) -> Commands {
        Commands::Restore {
            backup_id: backup_id.to_string(),
        }
    }
}
