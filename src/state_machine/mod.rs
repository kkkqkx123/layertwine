//! Hierarchical State Machine Module (Phase 3)
//!
//! Management manual_edit → agent_edit → approval → staged Four levels of pure state machine complete
//! Forward flow and reverse fallback to ensure hierarchical segregation and the principle of immutability.

pub mod manual;
pub mod agent;
pub mod approval;
pub mod staged;
pub mod transition;

use crate::core::layer::Layer;
use crate::core::partition::Partition;
use crate::core::types::{LayerType, PartitionId, SnapshotId};
use crate::error::{Result, StratumError};
use crate::storage::repository::PartitionStore;
use crate::storage::sqlite_storage::SqliteStorage;
use std::sync::Arc;

/// Hierarchical State Machine - Unified Operations Portal
///
/// Holds storage tier references and provides partition access and state flow interfaces for each tier.
pub struct StateMachine {
    storage: Arc<SqliteStorage>,
}

impl StateMachine {
    /// Creating a new state machine instance
    pub fn new(storage: Arc<SqliteStorage>) -> Self {
        StateMachine { storage }
    }

    /// Getting Storage Layer References
    pub fn storage(&self) -> &SqliteStorage {
        &self.storage
    }

    // Partition access methods -

    /// Get the specified partition of the specified layer (read-only)
    pub fn get_partition(&self, _layer: &LayerType, partition_id: &PartitionId) -> Result<Partition> {
        self.storage
            .get_partition(partition_id)
            .map_err(|e| StratumError::Storage(e.into()))
    }

    /// Get the specified partition of the specified layer (variable)
    pub fn get_partition_mut(&self, _layer: &LayerType, partition_id: &PartitionId) -> Result<Partition> {
        self.storage
            .get_partition(partition_id)
            .map_err(|e| StratumError::Storage(e.into()))
    }

    /// Getting or creating partitions
    pub fn get_or_create_partition(
        &self,
        _layer: &LayerType,
        partition_id: &PartitionId,
        _name: &str,
        partition: &Partition,
    ) -> Result<Partition> {
        // First try to get
        match self.storage.get_partition(partition_id) {
            Ok(p) => Ok(p),
            Err(_) => {
                self.storage
                    .create_partition(partition)
                    .map_err(|e| StratumError::Storage(e.into()))?;
                Ok(partition.clone())
            }
        }
    }

    /// Updating the partition pointer
    pub fn update_partition_pointer(
        &self,
        partition_id: &PartitionId,
        snapshot_id: &SnapshotId,
    ) -> Result<()> {
        self.storage
            .update_pointer(partition_id, snapshot_id)
            .map_err(|e| StratumError::Storage(e.into()))
    }

    // Layer management -

    /// Creating a Default Layer
    pub fn create_layer(&self, layer_type: &LayerType) -> Layer {
        Layer::new(layer_type.clone())
    }

    // Transaction support -

    /// enforcement service
    pub fn with_transaction<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&SqliteStorage) -> Result<T>,
    {
        self.storage
            .with_conn(|conn| {
                conn.execute_batch("BEGIN TRANSACTION;")
                    .map_err(|e| crate::StorageError::Database(e))?;
                // 在事务内：f 会使用 storage 方法（锁自己），所以需要先释放锁
                // 事务由 SQLite 内部管理，不依赖 Rust Mutex 保持
                Ok(())
            })?;

        let result = f(&self.storage);

        // 提交或回滚
        self.storage
            .with_conn(|conn| {
                match &result {
                    Ok(_) => conn
                        .execute_batch("COMMIT;")
                        .map_err(crate::StorageError::Database),
                    Err(_) => conn
                        .execute_batch("ROLLBACK;")
                        .map_err(crate::StorageError::Database),
                }
            })?;

        result
    }
}
