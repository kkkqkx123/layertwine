pub mod commands;
pub mod output;

use std::sync::Arc;

use crate::api::service::{ApiService, ApiServiceImpl, ServiceConfig};
use crate::api::types::*;
use crate::error::exit_codes;

use commands::*;
use output::*;

/// CLI entry point — parse args, call ApiService, format output, return exit code
pub fn run() -> i32 {
    run_with_cli(commands::parse_args())
}

/// Run CLI with a pre-built Cli struct (useful for testing)
pub fn run_with_cli(cli: Cli) -> i32 {
    let format = if cli.json {
        OutputFormat::Json
    } else {
        OutputFormat::Plain
    };

    let result = match &cli.command {
        Commands::Init { git_ref } => {
            let service = match ApiServiceImpl::open(ServiceConfig {
                db_path: cli.db_path.clone(),
                git_repo: cli.git_repo.clone(),
            }) {
                Ok(s) => Arc::new(s) as Arc<dyn ApiService>,
                Err(e) => {
                    eprintln!("error: {}", e);
                    return exit_codes::GENERAL_ERROR;
                }
            };
            service.init(InitRequest {
                db_path: Some(cli.db_path.clone()),
                git_repo: cli.git_repo.clone(),
                git_ref: git_ref.clone(),
            }).map(|_| ()).map_err(|e| e)
        }
        Commands::Status => {
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            let resp = service.status();
            print_status_from_response(&resp, format);
            return exit_codes::SUCCESS;
        }
        Commands::Edit { file, content } => {
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            let resp = service.edit(EditRequest {
                file: file.clone(),
                content: content.clone(),
            });
            if let Ok(ref r) = resp {
                if content.is_some() {
                    println!("Edited '{}' -> new snapshot {}", file, &r.snapshot_id[..12]);
                }
                if let Some(ref staged) = r.staged_snapshot_id {
                    println!("Merged to staged -> snapshot {}", &staged[..12]);
                }
            }
            resp.map(|_| ()).map_err(|e| e)
        }
        Commands::Agent { agent_id, action } => {
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            match action {
                AgentCommands::Edit { file, content } => {
                    let resp = service.agent_edit(AgentEditRequest {
                        agent_id: agent_id.clone(),
                        file: file.clone(),
                        content: content.clone(),
                    });
                    if let Ok(ref r) = resp {
                        if content.is_some() {
                            println!(
                                "Agent '{}' edited '{}' -> snapshot {}",
                                agent_id,
                                file,
                                &r.snapshot_id[..12]
                            );
                        }
                    }
                    resp.map(|_| ()).map_err(|e| e)
                }
                AgentCommands::Submit => {
                    let resp = service.agent_submit(AgentSubmitRequest {
                        agent_id: agent_id.clone(),
                    });
                    if let Ok(ref r) = resp {
                        println!(
                            "Agent '{}' submitted for approval -> snapshot {}",
                            agent_id,
                            &r.snapshot_id[..12]
                        );
                    }
                    resp.map(|_| ()).map_err(|e| e)
                }
            }
        }
        Commands::Approve { agent_id } => {
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            let resp = service.approve(ApproveRequest {
                agent_id: agent_id.clone(),
            });
            if let Ok(ref r) = resp {
                println!(
                    "Approved agent '{}' -> integrated snapshot {}",
                    agent_id,
                    &r.integrated_snapshot_id[..12]
                );
                println!(
                    "Merged to staged -> snapshot {}",
                    &r.staged_snapshot_id[..12]
                );
            }
            resp.map(|_| ()).map_err(|e| e)
        }
        Commands::Commit { message, author } => {
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            let resp = service.commit(CommitRequest {
                message: message.clone(),
                author: Some(author.clone()),
            });
            if let Ok(ref r) = resp {
                println!(
                    "Committed checkpoint {}: {}",
                    &r.checkpoint_id[..12],
                    r.message
                );
            }
            resp.map(|_| ()).map_err(|e| e)
        }
        Commands::Log { count } => {
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            let resp = service.log(LogRequest { count: Some(*count) });
            match resp {
                Ok(r) => {
                    let checkpoints: Vec<crate::checkpoint::checkpoint::Checkpoint> = vec![];
                    print_log_from_response(&r, format, checkpoints);
                    return exit_codes::SUCCESS;
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    return exit_codes::GENERAL_ERROR;
                }
            }
        }
        Commands::Branch { action } => {
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            match action {
                BranchCommands::List => {
                    let resp = service.branch_list();
                    match resp {
                        Ok(r) => {
                            print_branches_from_response(&r, format);
                            return exit_codes::SUCCESS;
                        }
                        Err(e) => {
                            eprintln!("error: {}", e);
                            return exit_codes::GENERAL_ERROR;
                        }
                    }
                }
                BranchCommands::Create { name } => {
                    let resp = service.branch_create(BranchCreateRequest { name: name.clone() });
                    if let Ok(ref r) = resp {
                        println!("Created branch '{}' at {}", r.name, &r.head[..12]);
                    }
                    resp.map(|_| ()).map_err(|e| e)
                }
                BranchCommands::Switch { name } => {
                    let resp = service.branch_switch(BranchSwitchRequest { name: name.clone() });
                    if let Ok(ref r) = resp {
                        println!(
                            "Switched to branch '{}' at checkpoint {}",
                            r.name,
                            &r.checkpoint_id[..12]
                        );
                    }
                    resp.map(|_| ()).map_err(|e| e)
                }
            }
        }
        Commands::Merge { branch, message } => {
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            let resp = service.merge(MergeRequest {
                branch: branch.clone(),
                message: Some(message.clone()),
            });
            if let Ok(ref r) = resp {
                println!(
                    "Merged '{}' into '{}' -> checkpoint {}",
                    r.source_branch,
                    r.target_branch,
                    &r.checkpoint_id[..12]
                );
            }
            resp.map(|_| ()).map_err(|e| e)
        }
        Commands::Backup { snapshot_id, label } => {
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            let resp = service.backup(BackupRequest {
                snapshot_id: snapshot_id.clone(),
                label: label.clone(),
            });
            if let Ok(ref r) = resp {
                if let Some(ref lbl) = r.label {
                    println!(
                        "Backup {} created for snapshot {} (label: {})",
                        &r.backup_id[..12],
                        &r.source_snapshot_id[..12],
                        lbl
                    );
                } else {
                    println!(
                        "Backup {} created for snapshot {}",
                        &r.backup_id[..12],
                        &r.source_snapshot_id[..12]
                    );
                }
            }
            resp.map(|_| ()).map_err(|e| e)
        }
        Commands::Restore { backup_id } => {
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            let resp = service.restore(RestoreRequest {
                backup_id: backup_id.clone(),
            });
            if let Ok(ref r) = resp {
                println!(
                    "Restored backup {} -> file: {}",
                    &r.backup_id[..12],
                    r.file
                );
            }
            resp.map(|_| ()).map_err(|e| e)
        }
        Commands::Gc => {
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            eprint!("  Running garbage collection ... ");
            let _ = std::io::Write::flush(&mut std::io::stderr());
            let resp = service.gc(GcRequest {});
            match resp {
                Ok(r) => {
                    eprintln!("done");
                    println!(
                        "GC complete: {} checkpoints removed, {} snapshots freed, {} bytes",
                        r.removed_checkpoints, r.removed_snapshots, r.freed_bytes
                    );
                    if r.delta_chain_depth_triggered {
                        println!("  Note: delta chain depth exceeded threshold");
                    }
                    return exit_codes::SUCCESS;
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    return exit_codes::GENERAL_ERROR;
                }
            }
        }
        Commands::Push { remote, message } => {
            let git_repo = match &cli.git_repo {
                Some(p) => p.clone(),
                None => {
                    eprintln!("error: --git-repo path is required for push");
                    return exit_codes::USAGE_ERROR;
                }
            };
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            eprint!("  Pushing to Git ... ");
            let _ = std::io::Write::flush(&mut std::io::stderr());
            let resp = service.push(PushRequest {
                remote: Some(remote.clone()),
                git_repo,
                message: Some(message.clone()),
            });
            match resp {
                Ok(r) => {
                    eprintln!("done");
                    println!("Pushed to remote '{}' (commit: {})", r.remote, r.git_commit_hash);
                    return exit_codes::SUCCESS;
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    return exit_codes::GENERAL_ERROR;
                }
            }
        }
        Commands::Pull { remote, git_ref } => {
            let git_repo = match &cli.git_repo {
                Some(p) => p.clone(),
                None => {
                    eprintln!("error: --git-repo path is required for pull");
                    return exit_codes::USAGE_ERROR;
                }
            };
            let service = match open_service(&cli) {
                Ok(s) => s,
                Err(code) => return code,
            };
            eprint!("  Pulling from Git remote ... ");
            let _ = std::io::Write::flush(&mut std::io::stderr());
            let resp = service.pull(PullRequest {
                remote: Some(remote.clone()),
                git_repo,
                git_ref: Some(git_ref.clone()),
            });
            match resp {
                Ok(r) => {
                    eprintln!("done");
                    println!("Pulled from remote '{}' ref '{}'", r.remote, r.git_ref);
                    return exit_codes::SUCCESS;
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    return exit_codes::GENERAL_ERROR;
                }
            }
        }
    };

    match result {
        Ok(()) => exit_codes::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", e);
            exit_codes::GENERAL_ERROR
        }
    }
}

fn open_service(cli: &Cli) -> std::result::Result<Arc<dyn ApiService>, i32> {
    match ApiServiceImpl::open(ServiceConfig {
        db_path: cli.db_path.clone(),
        git_repo: cli.git_repo.clone(),
    }) {
        Ok(s) => Ok(Arc::new(s)),
        Err(e) => {
            eprintln!("error: {}", e);
            Err(exit_codes::GENERAL_ERROR)
        }
    }
}