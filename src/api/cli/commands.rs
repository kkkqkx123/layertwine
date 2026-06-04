use clap::{Parser, Subcommand};

/// Stratum — lightweight file-edit history storage layer for multi-agent + human collaborative editing
#[derive(Parser, Debug)]
#[command(name = "stratum")]
#[command(about = "Stratum - file-edit history storage layer", long_about = None)]
#[command(version)]
pub struct Cli {
    /// Path to the stratum database file
    #[arg(short = 'd', long = "db", default_value = ".stratum/stratum.db")]
    pub db_path: String,

    /// Path to the Git repository (for sync operations)
    #[arg(short = 'g', long = "git-repo")]
    pub git_repo: Option<String>,

    /// JSON output mode
    #[arg(long = "json", global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize a new stratum repository
    #[command(name = "init")]
    Init {
        /// Git ref to initialize from (e.g. HEAD, branch name, commit hash)
        #[arg(long = "git-ref")]
        git_ref: Option<String>,
    },

    /// Show current status of all layers and partitions
    #[command(name = "status")]
    Status,

    /// Manually edit a file (manual_edit layer)
    #[command(name = "edit")]
    Edit {
        /// File path to edit
        file: String,
        /// New file content (reads from stdin if not provided)
        #[arg(short = 'c', long = "content")]
        content: Option<String>,
    },

    /// Agent operations
    #[command(name = "agent")]
    Agent {
        /// Agent instance ID
        agent_id: String,
        #[command(subcommand)]
        action: AgentCommands,
    },

    /// Approve an agent's changes
    #[command(name = "approve")]
    Approve {
        /// Agent ID to approve
        agent_id: String,
    },

    /// Commit staged changes as a checkpoint
    #[command(name = "commit")]
    Commit {
        /// Commit message
        #[arg(short = 'm', long = "message", required = true)]
        message: String,
        /// Author name
        #[arg(short = 'a', long = "author", default_value = "user")]
        author: String,
    },

    /// View checkpoint history
    #[command(name = "log")]
    Log {
        /// Maximum number of checkpoints to show
        #[arg(long = "count", default_value = "20")]
        count: usize,
    },

    /// Show diff for a target (staged, checkpoint, partition)
    #[command(name = "show")]
    Show {
        /// Target to show diff for
        show_what: String,
        /// Target ID (optional)
        #[arg(short = 'i', long = "id")]
        target_id: Option<String>,
    },

    /// Branch operations
    #[command(name = "branch")]
    Branch {
        #[command(subcommand)]
        action: BranchCommands,
    },

    /// Merge a branch into the current branch
    #[command(name = "merge")]
    Merge {
        /// Source branch name to merge from
        branch: String,
        /// Commit message for the merge
        #[arg(short = 'm', long = "message", default_value = "merge")]
        message: String,
    },

    /// Backup a snapshot
    #[command(name = "backup")]
    Backup {
        /// Snapshot ID to back up
        snapshot_id: String,
        /// Optional label for the backup
        #[arg(long = "label")]
        label: Option<String>,
    },

    /// Restore from a backup
    #[command(name = "restore")]
    Restore {
        /// Backup ID to restore from
        backup_id: String,
    },

    /// Run garbage collection
    #[command(name = "gc")]
    Gc,

    /// Push checkpoints to Git
    #[command(name = "push")]
    Push {
        /// Remote name (default: origin)
        #[arg(long = "remote", default_value = "origin")]
        remote: String,
        /// Commit message for the Git commit
        #[arg(short = 'm', long = "message", default_value = "sync from stratum")]
        message: String,
    },

    /// Fetch and pull from Git
    #[command(name = "pull")]
    Pull {
        /// Remote name (default: origin)
        #[arg(long = "remote", default_value = "origin")]
        remote: String,
        /// Git ref to pull (default: HEAD)
        #[arg(long = "git-ref", default_value = "HEAD")]
        git_ref: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum AgentCommands {
    /// Agent edit a file
    #[command(name = "edit")]
    Edit {
        /// File path to edit
        file: String,
        /// New file content
        #[arg(short = 'c', long = "content")]
        content: Option<String>,
    },
    /// Agent submit changes for review
    #[command(name = "submit")]
    Submit,
}

#[derive(Subcommand, Debug)]
pub enum BranchCommands {
    /// Create a new branch
    #[command(name = "create")]
    Create {
        /// Branch name
        name: String,
    },
    /// Switch to a branch
    #[command(name = "switch")]
    Switch {
        /// Branch name
        name: String,
    },
    /// List all branches
    #[command(name = "list")]
    List,
}

/// Parse CLI arguments and return the Cli struct
pub fn parse_args() -> Cli {
    Cli::parse()
}
