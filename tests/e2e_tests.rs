// E2E tests entry point

// Common modules for testing
mod common;

// E2E test modules
#[path = "shared_tests/approval_flow.rs"]
pub mod approval_flow;
#[path = "shared_tests/backup_restore.rs"]
pub mod backup_restore;
#[path = "shared_tests/basic_workflow.rs"]
pub mod basic_workflow;
#[path = "shared_tests/branch_operations.rs"]
pub mod branch_operations;
#[path = "shared_tests/edge_cases_additional.rs"]
pub mod edge_cases_additional;
#[path = "shared_tests/multi_agent.rs"]
pub mod multi_agent;
