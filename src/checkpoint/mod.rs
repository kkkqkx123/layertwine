//! Checkpoint Module (Phase 4)
//!
//! 自研检查点仓库：Checkpoint 提交流程、轻量分支创建/切换/合并、DAG 历史追踪。
//! 独立于 Git 的版本管理核心。

pub mod checkpoint;
pub mod branch;
pub mod dag;
pub mod repo;

pub use checkpoint::{Checkpoint, CheckpointBuilder, CheckpointMetadata};
pub use branch::Branch;
pub use dag::CheckpointDag;
pub use repo::CheckpointRepo;
