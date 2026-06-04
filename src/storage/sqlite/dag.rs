use crate::checkpoint::dag::CheckpointDag;
use crate::core::layer::Layer;
use crate::core::types::{LayerType, PartitionId};
use crate::storage::repository::{DagStore, LayerStore};
use crate::storage::sqlite::connection::SqliteStorage;
use crate::StorageResult;
use rusqlite::params;

impl DagStore for SqliteStorage {
    fn store_dag(&self, dag: &CheckpointDag) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();

        let nodes_serializable: std::collections::HashMap<String, Vec<String>> = dag
            .all_nodes()
            .iter()
            .map(|id| {
                let key = id.to_hex();
                let children: Vec<String> =
                    dag.get_children(id).iter().map(|c| c.to_hex()).collect();
                (key, children)
            })
            .collect();

        let gen_serializable: std::collections::HashMap<String, u64> = dag
            .all_nodes()
            .iter()
            .map(|id| {
                let key = id.to_hex();
                let gen = dag.generation(id).unwrap_or(0);
                (key, gen)
            })
            .collect();

        let payload = serde_json::json!({
            "nodes": nodes_serializable,
            "generation": gen_serializable,
        });

        let json = serde_json::to_vec(&payload)
            .map_err(|e| crate::StorageError::Serialization(e.to_string()))?;
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT OR REPLACE INTO dag_store (key, value, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params!["checkpoint_dag", json, now],
        )?;
        Ok(())
    }

    fn load_dag(&self) -> StorageResult<CheckpointDag> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM dag_store WHERE key = ?1")?;
        let result = stmt.query_row(rusqlite::params!["checkpoint_dag"], |row| {
            let json: Vec<u8> = row.get(0)?;

            let parsed: serde_json::Value = serde_json::from_slice(&json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let nodes_map = parsed["nodes"].as_object().ok_or_else(|| {
                rusqlite::Error::ToSqlConversionFailure(Box::new(std::fmt::Error))
            })?;

            let _gen_map = parsed["generation"].as_object().ok_or_else(|| {
                rusqlite::Error::ToSqlConversionFailure(Box::new(std::fmt::Error))
            })?;

            let mut dag = CheckpointDag::new();

            for (hex_key, _) in nodes_map {
                if let Some(id) = crate::core::types::ContentId::from_hex(hex_key) {
                    dag.add_node(id);
                }
            }

            for (hex_key, children_value) in nodes_map {
                if let Some(parent) = crate::core::types::ContentId::from_hex(hex_key) {
                    if let Some(children) = children_value.as_array() {
                        for child_val in children {
                            if let Some(child_hex) = child_val.as_str() {
                                if let Some(child) =
                                    crate::core::types::ContentId::from_hex(child_hex)
                                {
                                    dag.add_edge(parent, child);
                                }
                            }
                        }
                    }
                }
            }

            Ok(dag)
        });
        match result {
            Ok(dag) => Ok(dag),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(CheckpointDag::new()),
            Err(e) => Err(crate::StorageError::Database(e)),
        }
    }

    fn store_metadata(&self, key: &str, value: &str) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT OR REPLACE INTO dag_store (key, value, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![key, value.as_bytes(), now],
        )?;
        Ok(())
    }

    fn load_metadata(&self, key: &str) -> StorageResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM layers WHERE layer_type = ?1",
            params![layer_type.name()],
        )?;
        Ok(())
    }
}
