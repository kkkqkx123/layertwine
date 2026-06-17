use crate::core::layer::Layer;
use crate::core::types::{LayerType, PartitionId};
use crate::storage::repository::{LayerStore, MetadataStore};
use crate::storage::sqlite::connection::SqliteStorage;
use crate::StorageResult;
use rusqlite::params;

impl MetadataStore for SqliteStorage {
    fn store_metadata(&self, key: &str, value: &str) -> StorageResult<()> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT OR REPLACE INTO dag_store (key, value, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![key, value.as_bytes(), now],
        )?;
        Ok(())
    }

    fn load_metadata(&self, key: &str) -> StorageResult<Option<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT value FROM dag_store WHERE key = ?1")?;
        let result = stmt.query_row(rusqlite::params![key], |row| {
            let value: Vec<u8> = row.get(0)?;
            String::from_utf8(value)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        });
        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(crate::StorageError::Database(e)),
        }
    }
}

impl LayerStore for SqliteStorage {
    fn store_layer(&self, layer: &Layer) -> StorageResult<()> {
        let conn = self.conn.lock();
        let partition_ids_json = serde_json::to_vec(&layer.partitions)
            .map_err(|e| crate::StorageError::Serialization(e.to_string()))?;
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT OR REPLACE INTO layers (layer_type, partition_ids, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![layer.layer_type.name(), partition_ids_json, now, now,],
        )?;
        Ok(())
    }

    fn get_layer(&self, layer_type: &LayerType) -> StorageResult<Layer> {
        let conn = self.conn.lock();
        let mut stmt =
            conn.prepare("SELECT layer_type, partition_ids FROM layers WHERE layer_type = ?1")?;

        let result = stmt.query_row(params![layer_type.name()], |row| {
            let _lt: String = row.get(0)?;
            let partition_ids_json: Vec<u8> = row.get(1)?;
            let partitions: Vec<PartitionId> = serde_json::from_slice(&partition_ids_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Ok(Layer {
                layer_type: layer_type.clone(),
                partitions,
            })
        })?;
        Ok(result)
    }

    fn list_layer_types(&self) -> StorageResult<Vec<LayerType>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT layer_type FROM layers ORDER BY layer_type")?;
        let types: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let result: Vec<LayerType> = types
            .iter()
            .filter_map(|s| match s.as_str() {
                "manual_edit" => Some(LayerType::ManualEdit),
                "agent_edit" => Some(LayerType::AgentEdit),
                "approval" => Some(LayerType::Approval),
                "staged" => Some(LayerType::Staged),
                _ => None,
            })
            .collect();
        Ok(result)
    }

    fn delete_layer(&self, layer_type: &LayerType) -> StorageResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM layers WHERE layer_type = ?1",
            params![layer_type.name()],
        )?;
        Ok(())
    }
}
