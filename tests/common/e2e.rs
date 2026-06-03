use std::path::{Path, PathBuf};
use std::sync::Arc;

use stratum::api::cli::commands::{Cli, Commands, AgentCommands, BranchCommands};
use stratum::api::cli::run_with_cli;
use stratum::core::types::SnapshotId;
use stratum::layered::StateMachine;
use stratum::storage::repository::PartitionStore;
use stratum::storage::sqlite_storage::SqliteStorage;

use tempfile::TempDir;

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
    pub fn state_machine(&self) -> StateMachine {
        StateMachine::new(self.open_storage())
    }
}

// ===== Helper functions =====

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

// ===== Common command builders =====

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

/// Reconstruct the full text content of a snapshot from storage.
pub fn reconstruct_snapshot_text(storage: &SqliteStorage, snapshot_id: &stratum::core::types::SnapshotId) -> String {
    super::reconstruct_text(storage, snapshot_id)
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