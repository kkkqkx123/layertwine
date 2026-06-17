//! Checkpoint Module (Phase 4)
//!
//! Self-developed checkpoint repository: Checkpoint commit process, lightweight branch creation/switching/merging, DAG history tracking.
//! A versioning core independent of Git.

pub mod branch;
pub mod checkpoint;
pub mod dag;
pub mod repo;

pub use branch::Branch;
pub use checkpoint::{Checkpoint, CheckpointBuilder, CheckpointMetadata};
pub use dag::CheckpointDag;
pub use repo::CheckpointRepo;
