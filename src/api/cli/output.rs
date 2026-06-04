use crate::api::types::*;
use crate::core::delta::Delta;
use crate::core::types::LineDiff;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::DiffOp;

/// Output format mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Plain,
    Json,
}

/// Print status from ApiService response
pub fn print_status_from_response(resp: &Result<StatusResponse, crate::api::types::ApiError>, format: OutputFormat) {
    match resp {
        Ok(r) => match format {
            OutputFormat::Plain => {
                if r.partitions.is_empty() {
                    println!("No partitions found. Run 'stratum init' to initialize.");
                    return;
                }
                println!("{:-<72}", "");
                println!("{:<16} {:<24} {:<20} History", "Layer", "Partition", "Current Snapshot");
                println!("{:-<72}", "");
                for entry in &r.partitions {
                    let short_hash = if entry.current_snapshot.len() > 12 {
                        &entry.current_snapshot[..12]
                    } else {
                        &entry.current_snapshot
                    };
                    println!(
                        "{:<16} {:<24} {:<20} {} snapshots",
                        entry.layer, entry.name, short_hash, entry.history_len
                    );
                }
                println!("{:-<72}", "");
            }
            OutputFormat::Json => {
                println!("{}", serde_json::to_string_pretty(r).unwrap());
            }
        },
        Err(e) => {
            eprintln!("error reading status: {}", e);
        }
    }
}

/// Print checkpoint history from ApiService response
pub fn print_log_from_response(
    resp: &LogResponse,
    format: OutputFormat,
) {
    match format {
        OutputFormat::Plain => {
            if resp.checkpoints.is_empty() {
                println!("No checkpoints found.");
                return;
            }
            println!("{:-<100}", "");
            println!(
                "{:<20} {:<16} {:<12} {:<12} {:<30}",
                "Checkpoint ID", "Author", "Parents", "Snapshots", "Message"
            );
            println!("{:-<100}", "");
            for cp in resp.checkpoints.iter().rev() {
                let short_id = &cp.id[..12];
                let short_msg = if cp.message.len() > 27 {
                    format!("{}...", &cp.message[..27])
                } else {
                    cp.message.clone()
                };
                let git_tag = if cp.git_anchor.is_some() { " [git]" } else { "" };
                println!(
                    "{:<20} {:<16} {:<12} {:<12} {:<30}",
                    format!("{}{}", short_id, git_tag),
                    cp.author,
                    cp.parents.len(),
                    cp.snapshots.len(),
                    short_msg,
                );
            }
            println!("{:-<100}", "");
            println!("Total: {} checkpoint(s)", resp.checkpoints.len());
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(resp).unwrap());
        }
    }
}

/// Print branch list from ApiService response
pub fn print_branches_from_response(resp: &BranchListResponse, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            if resp.branches.is_empty() {
                println!("No branches found.");
                return;
            }
            println!("{:-<60}", "");
            println!("{:<24} {:<20} Updated", "Branch", "Head");
            println!("{:-<60}", "");
            for b in &resp.branches {
                let marker = if b.is_current { "* " } else { "  " };
                let short_head = &b.head[..12];
                println!(
                    "{}{:<22} {:<20} {}",
                    marker, b.name, short_head, b.updated_at
                );
            }
            println!("{:-<60}", "");
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(resp).unwrap());
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

/// Print show response (unified diff display)
pub fn print_show_from_response(resp: &ShowResponse, format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            for diff in &resp.diffs {
                println!("--- a/{}", diff.file_path);
                println!("+++ b/{}", diff.file_path);
                println!("{}", diff.unified_diff);
            }
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(resp).unwrap());
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
                        println!("  ");
                    }
                }
                DiffOp::Delete { count, .. } => {
                    for _ in 0..*count {
                        println!("- ");
                    }
                }
                DiffOp::Insert { lines, .. } => {
                    for line in lines {
                        println!("+{}", line);
                    }
                }
                DiffOp::Replace { old_count, lines, .. } => {
                    for _ in 0..*old_count {
                        println!("- ");
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

/// Print a progress message to stderr
pub fn print_progress(message: &str) {
    eprint!("  {} ... ", message);
    std::io::Write::flush(&mut std::io::stderr()).ok();
}

/// Print a done message to stderr (completing a progress indicator)
pub fn print_done() {
    eprintln!("done");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_format_enum() {
        assert_ne!(OutputFormat::Plain as u8, OutputFormat::Json as u8);
    }
}