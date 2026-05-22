use crate::checkpoint::branch::Branch;
use crate::checkpoint::checkpoint::Checkpoint;
use crate::core::delta::{Delta, LineDiff};
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{DiffOp, PartitionType};
use crate::state_machine::StateMachine;
use crate::storage::repository::PartitionStore;
use std::io::{self, Write};

/// Output format mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Plain,
    Json,
}

/// Print status of all partition layers
pub fn print_status(state_machine: &StateMachine, format: OutputFormat) {
    let storage = state_machine.storage();
    let mut output = StatusOutput::new();

    match storage.list_partitions() {
        Ok(partitions) => {
            for partition in &partitions {
                let layer_name = layer_name_for_partition(&partition.partition_type);
                output.add_entry(
                    layer_name,
                    partition.name.clone(),
                    partition.current_snapshot.to_hex(),
                    partition.history.len(),
                );
            }
        }
        Err(e) => {
            output.error = Some(e.to_string());
        }
    }

    match format {
        OutputFormat::Plain => output.print_plain(),
        OutputFormat::Json => output.print_json(),
    }
}

struct StatusOutput {
    entries: Vec<StatusEntry>,
    error: Option<String>,
}

struct StatusEntry {
    layer: String,
    partition: String,
    current_snapshot: String,
    history_len: usize,
}

impl StatusOutput {
    fn new() -> Self {
        StatusOutput {
            entries: Vec::new(),
            error: None,
        }
    }

    fn add_entry(&mut self, layer: String, partition: String, snapshot: String, history: usize) {
        self.entries.push(StatusEntry {
            layer,
            partition,
            current_snapshot: snapshot,
            history_len: history,
        });
    }

    fn print_plain(&self) {
        if let Some(ref err) = self.error {
            eprintln!("error reading status: {}", err);
            return;
        }
        if self.entries.is_empty() {
            println!("No partitions found. Run 'stratum init' to initialize.");
            return;
        }
        println!("{:-<72}", "");
        println!(
            "{:<16} {:<24} {:<20} {}",
            "Layer", "Partition", "Current Snapshot", "History"
        );
        println!("{:-<72}", "");
        for entry in &self.entries {
            let short_hash = if entry.current_snapshot.len() > 12 {
                &entry.current_snapshot[..12]
            } else {
                &entry.current_snapshot
            };
            println!(
                "{:<16} {:<24} {:<20} {} snapshots",
                entry.layer, entry.partition, short_hash, entry.history_len
            );
        }
        println!("{:-<72}", "");
    }

    fn print_json(&self) {
        let json = serde_json::json!({
            "status": if self.error.is_some() { "error" } else { "ok" },
            "partitions": self.entries.iter().map(|e| {
                serde_json::json!({
                    "layer": e.layer,
                    "partition": e.partition,
                    "current_snapshot": e.current_snapshot,
                    "history_len": e.history_len,
                })
            }).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&json).unwrap());
    }
}

fn layer_name_for_partition(pt: &PartitionType) -> String {
    match pt {
        PartitionType::Manual => "manual_edit".to_string(),
        PartitionType::Agent(_) => "agent_edit".to_string(),
        PartitionType::Approval(_) => "approval".to_string(),
        PartitionType::Integrated(_) => "approval".to_string(),
        PartitionType::Unified => "approval".to_string(),
        PartitionType::Staged => "staged".to_string(),
    }
}

/// Print checkpoint history as a table
pub fn print_log(checkpoints: &[Checkpoint], format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            if checkpoints.is_empty() {
                println!("No checkpoints found.");
                return;
            }
            println!("{:-<100}", "");
            println!(
                "{:<20} {:<16} {:<12} {:<12} {:<30}",
                "Checkpoint ID", "Author", "Parents", "Snapshots", "Message"
            );
            println!("{:-<100}", "");
            for cp in checkpoints.iter().rev() {
                let short_id = &cp.id.to_hex()[..12];
                let short_msg = if cp.metadata.message.len() > 27 {
                    format!("{}...", &cp.metadata.message[..27])
                } else {
                    cp.metadata.message.clone()
                };
                let git_tag = if cp.metadata.git_anchor.is_some() {
                    " [git]"
                } else {
                    ""
                };
                println!(
                    "{:<20} {:<16} {:<12} {:<12} {:<30}",
                    format!("{}{}", short_id, git_tag),
                    cp.metadata.author,
                    cp.parents.len(),
                    cp.baseline_snapshots.len(),
                    short_msg,
                );
            }
            println!("{:-<100}", "");
            println!(
                "Total: {} checkpoint(s)",
                checkpoints.len()
            );
        }
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = checkpoints
                .iter()
                .map(|cp| {
                    serde_json::json!({
                        "id": cp.id.to_hex(),
                        "author": cp.metadata.author,
                        "message": cp.metadata.message,
                        "parents": cp.parents.iter().map(|p| p.to_hex()).collect::<Vec<_>>(),
                        "snapshots": cp.baseline_snapshots.iter().map(|s| s.to_hex()).collect::<Vec<_>>(),
                        "created_at": cp.created_at,
                        "git_anchor": cp.metadata.git_anchor,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
    }
}

/// Print branch list
pub fn print_branches(
    branches: &[Branch],
    current_name: Option<&str>,
    format: OutputFormat,
) {
    match format {
        OutputFormat::Plain => {
            if branches.is_empty() {
                println!("No branches found.");
                return;
            }
            println!("{:-<60}", "");
            println!("{:<24} {:<20} {}", "Branch", "Head", "Updated");
            println!("{:-<60}", "");
            for branch in branches {
                let marker = if Some(branch.name.as_str()) == current_name {
                    "* "
                } else {
                    "  "
                };
                let short_head = &branch.head.to_hex()[..12];
                println!(
                    "{}{:<22} {:<20} {}",
                    marker, branch.name, short_head, branch.updated_at
                );
            }
            println!("{:-<60}", "");
        }
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = branches
                .iter()
                .map(|b| {
                    serde_json::json!({
                        "name": b.name,
                        "head": b.head.to_hex(),
                        "created_at": b.created_at,
                        "updated_at": b.updated_at,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
    }
}

/// Print delta diff display (similar to `git diff`)
pub fn print_diff(delta: &Delta, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            println!(
                "diff --git a/{} b/{}",
                delta.file.path_str(),
                delta.file.path_str()
            );
            let summary = delta.summary();
            println!(
                "--- a/{}\n+++ b/{}",
                delta.file.path_str(),
                delta.file.path_str()
            );
            println!(
                "@@ ... @@ ({} inserts, {} deletes, {} replaces)",
                summary.inserts, summary.deletes, summary.replaces
            );
            print_line_diff(&delta.diff);
        }
        OutputFormat::Json => {
            let summary = delta.summary();
            let json = serde_json::json!({
                "file": delta.file.path_str(),
                "source": format!("{:?}", delta.source),
                "timestamp": delta.timestamp,
                "summary": {
                    "inserts": summary.inserts,
                    "deletes": summary.deletes,
                    "replaces": summary.replaces,
                    "hunks": summary.total_hunks,
                },
                "diff": format!("{:?}", delta.diff),
            });
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
    }
}

fn print_line_diff(diff: &LineDiff) {
    for hunk in &diff.hunks {
        println!(
            "@@ -{},{} +{},{} @@",
            hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len
        );
        for op in &hunk.ops {
            match op {
                DiffOp::Equal { count } => {
                    for _ in 0..*count {
                        println!(" {}", " ");
                    }
                }
                DiffOp::Delete { count, .. } => {
                    for _ in 0..*count {
                        println!("-{}", " ");
                    }
                }
                DiffOp::Insert { lines, .. } => {
                    for line in lines {
                        println!("+{}", line);
                    }
                }
                DiffOp::Replace {
                    old_count,
                    lines,
                    ..
                } => {
                    for _ in 0..*old_count {
                        println!("-{}", " ");
                    }
                    for line in lines {
                        println!("+{}", line);
                    }
                }
            }
        }
    }
}

/// Print snapshot summary
pub fn print_snapshot(snapshot: &Snapshot, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            println!("Snapshot: {}", snapshot.id.to_hex());
            println!("  File:     {}", snapshot.file.path_str());
            println!("  Type:     {}", snapshot.partition_type);
            println!("  Deltas:   {}", snapshot.deltas.len());
            println!("  Parents:  {}", snapshot.parents.len());
            println!(
                "  Created:  {}",
                chrono::DateTime::from_timestamp_millis(snapshot.created_at)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| "unknown".to_string())
            );
            if !snapshot.parents.is_empty() {
                println!("  Parent IDs:");
                for (i, pid) in snapshot.parents.iter().enumerate() {
                    println!("    [{}] {}", i, pid.to_hex());
                }
            }
        }
        OutputFormat::Json => {
            let json = serde_json::json!({
                "id": snapshot.id.to_hex(),
                "file": snapshot.file.path_str(),
                "partition_type": snapshot.partition_type,
                "delta_count": snapshot.deltas.len(),
                "parent_count": snapshot.parents.len(),
                "parents": snapshot.parents.iter().map(|p| p.to_hex()).collect::<Vec<_>>(),
                "created_at": snapshot.created_at,
            });
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
    }
}

/// Print a progress message to stderr
pub fn print_progress(message: &str) {
    eprint!("  {} ... ", message);
    io::stderr().flush().ok();
}

/// Print a done message to stderr (completing a progress indicator)
pub fn print_done() {
    eprintln!("done");
}

/// Print a partition's details
pub fn print_partition(partition: &Partition, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            println!("Partition: {}", partition.name);
            println!("  ID:      {}", partition.id);
            println!("  Type:    {:?}", partition.partition_type);
            println!(
                "  Current: {}",
                &partition.current_snapshot.to_hex()[..12]
            );
            println!("  History: {} snapshots", partition.history.len());
        }
        OutputFormat::Json => {
            let json = serde_json::json!({
                "id": partition.id.to_string(),
                "name": partition.name,
                "partition_type": format!("{:?}", partition.partition_type),
                "current_snapshot": partition.current_snapshot.to_hex(),
                "history_len": partition.history.len(),
            });
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
    }
}

/// Print backup snapshot details
pub fn print_backup_snapshot(
    bs: &crate::backup::backup_snapshot::BackupSnapshot,
    format: OutputFormat,
) {
    match format {
        OutputFormat::Plain => {
            println!("Backup: {}", bs.id.to_hex());
            println!("  Source:  {}", bs.source_snapshot.to_hex());
            println!("  File:    {}", bs.file.path_str());
            println!("  Deltas:  {}", bs.deltas.len());
            if let Some(ref label) = bs.label {
                println!("  Label:   {}", label);
            }
            println!(
                "  Time:    {}",
                chrono::DateTime::from_timestamp_millis(bs.backed_at)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| "unknown".to_string())
            );
        }
        OutputFormat::Json => {
            let json = serde_json::json!({
                "id": bs.id.to_hex(),
                "source_snapshot": bs.source_snapshot.to_hex(),
                "file": bs.file.path_str(),
                "deltas_count": bs.deltas.len(),
                "label": bs.label,
                "backed_at": bs.backed_at,
                "agent_id": bs.agent_id,
                "source_type": bs.source_type,
            });
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_name_for_partition() {
        assert_eq!(
            layer_name_for_partition(&PartitionType::Manual),
            "manual_edit"
        );
        assert_eq!(
            layer_name_for_partition(&PartitionType::Staged),
            "staged"
        );
        assert_eq!(
            layer_name_for_partition(&PartitionType::Agent("agent-1".into())),
            "agent_edit"
        );
    }
}