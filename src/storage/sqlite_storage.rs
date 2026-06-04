use crate::checkpoint::branch::Branch;
use crate::checkpoint::checkpoint::Checkpoint;
use crate::checkpoint::dag::CheckpointDag;
use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::layer::Layer;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{CheckpointId, ContentId, DeltaId, LayerType, LineDiff, PartitionId, PartitionType, SnapshotId};
use crate::storage::migrations;
use crate::storage::repository::{
    AtomicOps, BranchStore, CheckpointStore, DagStore, DeltaStore, FileNodeStore, LayerStore, PartitionStore,
    SnapshotStore,
};
use crate::StorageResult;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// SQLite Storage Implementation
pub struct SqliteStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStorage {
    /// Creating a new SQLite store (in-memory database)
    pub fn new_in_memory() -> StorageResult<Self> {
        let conn = Connection::open_in_memory()?;
        migrations::initialize_database(&conn)?;
        Ok(SqliteStorage {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Creating a new SQLite store (file database)
    pub fn new(path: &Path) -> StorageResult<Self> {
        let conn = Connection::open(path)?;
        migrations::initialize_database(&conn)?;
        Ok(SqliteStorage {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create a new SQLite store (file database with checkpoint tables)
    pub fn new_full(path: &Path) -> StorageResult<Self> {
        let conn = Connection::open(path)?;
        migrations::initialize_full(&conn)?;
        Ok(SqliteStorage {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Creating storage instances using existing connections (for in-transaction operations)
    /// Share the same underlying Arc<Mutex<Connection>>
    pub fn new_with_connection_arc(conn: &Arc<Mutex<Connection>>) -> Self {
        SqliteStorage {
            conn: conn.clone(),
        }
    }

    /// Create a shared instance (clones the Arc, shares the same connection)
    pub fn share(&self) -> Self {
        SqliteStorage {
            conn: self.conn.clone(),
        }
    }

    /// Getting a reference to an internal connection (for transactions, etc.)
    pub fn with_conn<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&Connection) -> StorageResult<T>,
    {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }

    /// enforcement service
    pub fn with_transaction<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&Connection) -> StorageResult<T>,
    {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN TRANSACTION;")?;
        match f(&conn) {
            Ok(result) => {
                conn.execute_batch("COMMIT;")?;
                Ok(result)
            }
            Err(e) => {
                conn.execute_batch("ROLLBACK;")?;
                Err(e)
            }
        }
    }
}

// SnapshotStore implementation.

impl SnapshotStore for SqliteStorage {
    fn store_snapshot(&self, snapshot: &Snapshot, _content: &[u8]) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        let deltas_json = serde_json::to_vec(&snapshot.deltas)?;
        let parents_json = serde_json::to_vec(&snapshot.parents)?;

        conn.execute(
            "INSERT OR IGNORE INTO snapshots (id, file_path, file_hash, deltas, parents, partition_type, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &snapshot.id.0.to_vec(),
                snapshot.file.path_str(),
                &snapshot.file.base_hash.to_vec(),
                deltas_json,
                parents_json,
                snapshot.partition_type,
                snapshot.created_at,
            ],
        )?;
        Ok(())
    }

    fn get_snapshot(&self, id: &SnapshotId) -> StorageResult<Snapshot> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, file_path, file_hash, deltas, parents, partition_type, created_at FROM snapshots WHERE id = ?1"
        )?;

        let result = stmt.query_row(params![&id.0.to_vec()], |row| {
            let id_bytes: Vec<u8> = row.get(0)?;
            let mut id_arr = [0u8; 32];
            id_arr.copy_from_slice(&id_bytes);

            let file_path: String = row.get(1)?;
            let file_hash_bytes: Vec<u8> = row.get(2)?;
            let mut file_hash_arr = [0u8; 32];
            file_hash_arr.copy_from_slice(&file_hash_bytes);

            let deltas_json: Vec<u8> = row.get(3)?;
            let parents_json: Vec<u8> = row.get(4)?;
            let partition_type: String = row.get(5)?;
            let created_at: i64 = row.get(6)?;

            let deltas: Vec<DeltaId> = serde_json::from_slice(&deltas_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let parents: Vec<SnapshotId> = serde_json::from_slice(&parents_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            Ok(Snapshot {
                id: ContentId(id_arr),
                file: FileNode {
                    file_path: std::path::PathBuf::from(file_path),
                    base_hash: file_hash_arr,
                },
                deltas,
                parents,
                partition_type,
                created_at,
            })
        })?;
        Ok(result)
    }

    fn find_snapshots_by_file(&self, file_path: &str) -> StorageResult<Vec<Snapshot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, file_path, file_hash, deltas, parents, partition_type, created_at
             FROM snapshots WHERE file_path = ?1 ORDER BY created_at DESC"
        )?;

        let snapshots = stmt.query_map(params![file_path], |row| {
            let id_bytes: Vec<u8> = row.get(0)?;
            let mut id_arr = [0u8; 32];
            id_arr.copy_from_slice(&id_bytes);

            let fp: String = row.get(1)?;
            let fhb: Vec<u8> = row.get(2)?;
            let mut fh_arr = [0u8; 32];
            fh_arr.copy_from_slice(&fhb);

            let deltas_json: Vec<u8> = row.get(3)?;
            let parents_json: Vec<u8> = row.get(4)?;
            let pt: String = row.get(5)?;
            let ca: i64 = row.get(6)?;

            let deltas: Vec<DeltaId> = serde_json::from_slice(&deltas_json).unwrap_or_default();
            let parents: Vec<SnapshotId> = serde_json::from_slice(&parents_json).unwrap_or_default();

            Ok(Snapshot {
                id: ContentId(id_arr),
                file: FileNode {
                    file_path: std::path::PathBuf::from(fp),
                    base_hash: fh_arr,
                },
                deltas,
                parents,
                partition_type: pt,
                created_at: ca,
            })
        })?;

        let mut result = Vec::new();
        for s in snapshots {
            result.push(s?);
        }
        Ok(result)
    }

    fn find_snapshots_by_partition(&self, partition_type: &PartitionType) -> StorageResult<Vec<Snapshot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, file_path, file_hash, deltas, parents, partition_type, created_at
             FROM snapshots WHERE partition_type = ?1 ORDER BY created_at DESC"
        )?;

        let snapshots = stmt.query_map(params![partition_type.name()], |row| {
            let id_bytes: Vec<u8> = row.get(0)?;
            let mut id_arr = [0u8; 32];
            id_arr.copy_from_slice(&id_bytes);

            let fp: String = row.get(1)?;
            let fhb: Vec<u8> = row.get(2)?;
            let mut fh_arr = [0u8; 32];
            fh_arr.copy_from_slice(&fhb);

            let deltas_json: Vec<u8> = row.get(3)?;
            let parents_json: Vec<u8> = row.get(4)?;
            let pt: String = row.get(5)?;
            let ca: i64 = row.get(6)?;

            let deltas: Vec<DeltaId> = serde_json::from_slice(&deltas_json).unwrap_or_default();
            let parents: Vec<SnapshotId> = serde_json::from_slice(&parents_json).unwrap_or_default();

            Ok(Snapshot {
                id: ContentId(id_arr),
                file: FileNode {
                    file_path: std::path::PathBuf::from(fp),
                    base_hash: fh_arr,
                },
                deltas,
                parents,
                partition_type: pt,
                created_at: ca,
            })
        })?;

        let mut result = Vec::new();
        for s in snapshots {
            result.push(s?);
        }
        Ok(result)
    }

    fn snapshot_exists(&self, id: &SnapshotId) -> StorageResult<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT COUNT(*) FROM snapshots WHERE id = ?1")?;
        let count: i64 = stmt.query_row(params![&id.0.to_vec()], |row| row.get(0))?;
        Ok(count > 0)
    }
}

// DeltaStore implementation -

impl DeltaStore for SqliteStorage {
    fn store_delta(&self, delta: &Delta) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        let diff_json = serde_json::to_vec(&delta.diff)?;
        let source_data = match &delta.source {
            crate::core::types::SourceType::Agent(id) => Some(id.to_string()),
            _ => None,
        };
        let source_str = match &delta.source {
            crate::core::types::SourceType::Manual => "manual",
            crate::core::types::SourceType::Agent(_) => "agent",
            crate::core::types::SourceType::Backup => "backup",
        };

        conn.execute(
            "INSERT OR IGNORE INTO deltas (id, file_path, file_hash, diff, source, source_data, timestamp, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &delta.id.0.to_vec(),
                delta.file.path_str(),
                &delta.file.base_hash.to_vec(),
                diff_json,
                source_str,
                source_data,
                delta.timestamp,
                chrono::Utc::now().timestamp_millis(),
            ],
        )?;
        Ok(())
    }

    fn get_delta(&self, id: &DeltaId) -> StorageResult<Delta> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, file_path, file_hash, diff, source, source_data, timestamp FROM deltas WHERE id = ?1"
        )?;

        let result = stmt.query_row(params![&id.0.to_vec()], |row| {
            let id_bytes: Vec<u8> = row.get(0)?;
            let mut id_arr = [0u8; 32];
            id_arr.copy_from_slice(&id_bytes);

            let file_path: String = row.get(1)?;
            let file_hash_bytes: Vec<u8> = row.get(2)?;
            let mut fh_arr = [0u8; 32];
            fh_arr.copy_from_slice(&file_hash_bytes);

            let diff_json: Vec<u8> = row.get(3)?;
            let source: String = row.get(4)?;
            let source_data: Option<String> = row.get(5)?;
            let timestamp: i64 = row.get(6)?;

            let diff: LineDiff = serde_json::from_slice(&diff_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let source_type = match source.as_str() {
                "manual" => crate::core::types::SourceType::Manual,
                "agent" => crate::core::types::SourceType::Agent(
                    crate::core::types::AgentInstanceId(source_data.unwrap_or_default())
                ),
                "backup" => crate::core::types::SourceType::Backup,
                _ => crate::core::types::SourceType::Manual,
            };

            Ok(Delta {
                id: ContentId(id_arr),
                file: FileNode {
                    file_path: std::path::PathBuf::from(file_path),
                    base_hash: fh_arr,
                },
                diff,
                source: source_type,
                timestamp,
            })
        })?;
        Ok(result)
    }

    fn get_deltas(&self, ids: &[DeltaId]) -> StorageResult<Vec<Delta>> {
        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            result.push(self.get_delta(id)?);
        }
        Ok(result)
    }

    fn delta_exists(&self, id: &DeltaId) -> StorageResult<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT COUNT(*) FROM deltas WHERE id = ?1")?;
        let count: i64 = stmt.query_row(params![&id.0.to_vec()], |row| row.get(0))?;
        Ok(count > 0)
    }
}

// FileNodeStore implementation.

impl FileNodeStore for SqliteStorage {
    fn store_file_node(&self, file_node: &FileNode, content: &[u8]) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO file_nodes (file_path, base_hash, content, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                file_node.path_str(),
                &file_node.base_hash.to_vec(),
                content,
                chrono::Utc::now().timestamp_millis(),
            ],
        )?;
        Ok(())
    }

    fn get_file_content(&self, file_path: &str, base_hash: &[u8; 32]) -> StorageResult<Vec<u8>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT content FROM file_nodes WHERE file_path = ?1 AND base_hash = ?2"
        )?;
        let content: Vec<u8> = stmt.query_row(
            params![file_path, &base_hash.to_vec()],
            |row| row.get(0),
        )?;
        Ok(content)
    }

    fn file_node_exists(&self, file_path: &str, base_hash: &[u8; 32]) -> StorageResult<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM file_nodes WHERE file_path = ?1 AND base_hash = ?2"
        )?;
        let count: i64 = stmt.query_row(
            params![file_path, &base_hash.to_vec()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

// PartitionStore implementation -

impl PartitionStore for SqliteStorage {
    fn create_partition(&self, partition: &Partition) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        let partition_type_str = format!("{:?}", partition.partition_type);
        let now = chrono::Utc::now().timestamp_millis();

        conn.execute(
            "INSERT INTO partitions (id, name, current_snapshot, partition_type, partition_data, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &partition.id.as_bytes().to_vec(),
                partition.name,
                &partition.current_snapshot.0.to_vec(),
                partition_type_str,
                serde_json::to_string(&partition.partition_type)?,
                now,
                now,
            ],
        )?;

        // put into history
        for (seq, snap_id) in partition.history.iter().enumerate() {
            conn.execute(
                "INSERT INTO partition_history (partition_id, snapshot_id, seq, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    &partition.id.as_bytes().to_vec(),
                    &snap_id.0.to_vec(),
                    seq as i64,
                    now,
                ],
            )?;
        }

        Ok(())
    }

    fn update_pointer(&self, partition_id: &PartitionId, snapshot_id: &SnapshotId) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp_millis();

        conn.execute(
            "UPDATE partitions SET current_snapshot = ?1, updated_at = ?2 WHERE id = ?3",
            params![&snapshot_id.0.to_vec(), now, &partition_id.as_bytes().to_vec()],
        )?;

        // Queries the current maximum seq
        let max_seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), -1) FROM partition_history WHERE partition_id = ?1",
                params![&partition_id.as_bytes().to_vec()],
                |row| row.get(0),
            )
            .unwrap_or(-1);

        // Insert new history
        conn.execute(
            "INSERT INTO partition_history (partition_id, snapshot_id, seq, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                &partition_id.as_bytes().to_vec(),
                &snapshot_id.0.to_vec(),
                max_seq + 1,
                now,
            ],
        )?;

        Ok(())
    }

    fn get_partition(&self, id: &PartitionId) -> StorageResult<Partition> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, current_snapshot, partition_type, partition_data, created_at, updated_at
             FROM partitions WHERE id = ?1"
        )?;

        let partition = stmt.query_row(params![&id.as_bytes().to_vec()], |row| {
            let id_bytes: Vec<u8> = row.get(0)?;
            let name: String = row.get(1)?;
            let snap_bytes: Vec<u8> = row.get(2)?;
            let mut snap_arr = [0u8; 32];
            snap_arr.copy_from_slice(&snap_bytes);
            let partition_data: Option<String> = row.get(4)?;

            let partition_type: PartitionType = partition_data
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(PartitionType::Manual);

            Ok((id_bytes, name, snap_arr, partition_type))
        })?;

        // Read history
        let mut hist_stmt = conn.prepare(
            "SELECT snapshot_id, seq FROM partition_history WHERE partition_id = ?1 ORDER BY seq"
        )?;
        let history: Vec<SnapshotId> = hist_stmt
            .query_map(params![&id.as_bytes().to_vec()], |row| {
                let snap_bytes: Vec<u8> = row.get(0)?;
                let mut snap_arr = [0u8; 32];
                snap_arr.copy_from_slice(&snap_bytes);
                Ok(ContentId(snap_arr))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Rebuild Partition
        // Overwrite the placeholder value with the actual value read from the database
        let (id_bytes, name, snap_arr, partition_type) = partition;
        // id_bytes is Vec<u8>, we need to rebuild PartitionId
        let actual_id = uuid::Uuid::from_slice(&id_bytes)
            .map_err(|e| crate::StorageError::Serialization(e.to_string()))?;

        Ok(Partition {
            id: actual_id,
            name,
            current_snapshot: ContentId(snap_arr),
            history: history.clone(),
            partition_type,
        })
    }

    fn get_partition_by_name(&self, name: &str) -> StorageResult<Partition> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id FROM partitions WHERE name = ?1"
        )?;

        let id_bytes: Vec<u8> = stmt.query_row(params![name], |row| row.get(0))?;
        let id = uuid::Uuid::from_slice(&id_bytes)
            .map_err(|e| crate::StorageError::Serialization(e.to_string()))?;
        drop(stmt);
        drop(conn);

        self.get_partition(&id)
    }

    fn list_partitions(&self) -> StorageResult<Vec<Partition>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id FROM partitions ORDER BY name"
        )?;

        let ids: Vec<PartitionId> = stmt
            .query_map([], |row| {
                let bytes: Vec<u8> = row.get(0)?;
                let id = uuid::Uuid::from_slice(&bytes)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                Ok(id)
            })?
            .collect::<Result<Vec<_>, _>>()?;

        drop(stmt);
        drop(conn);

        let mut result = Vec::new();
        for id in ids {
            result.push(self.get_partition(&id)?);
        }
        Ok(result)
    }
}

// CheckpointStore implementation -

impl CheckpointStore for SqliteStorage {
    fn store_checkpoint(&self, checkpoint: &Checkpoint) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        let parents_json = serde_json::to_vec(&checkpoint.parents)
            .map_err(|e| crate::StorageError::Serialization(e.to_string()))?;
        let snapshot_ids_json = serde_json::to_vec(&checkpoint.baseline_snapshots)
            .map_err(|e| crate::StorageError::Serialization(e.to_string()))?;

        conn.execute(
            "INSERT OR IGNORE INTO checkpoints (id, parents, snapshot_ids, author, message, git_anchor, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                &checkpoint.id.0.to_vec(),
                parents_json,
                snapshot_ids_json,
                checkpoint.metadata.author,
                checkpoint.metadata.message,
                checkpoint.metadata.git_anchor,
                checkpoint.created_at,
            ],
        )?;
        Ok(())
    }

    fn get_checkpoint(&self, id: &CheckpointId) -> StorageResult<Checkpoint> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, parents, snapshot_ids, author, message, git_anchor, created_at FROM checkpoints WHERE id = ?1"
        )?;

        let result = stmt.query_row(rusqlite::params![&id.0.to_vec()], |row| {
            let id_bytes: Vec<u8> = row.get(0)?;
            let mut id_arr = [0u8; 32];
            id_arr.copy_from_slice(&id_bytes);

            let parents_json: Vec<u8> = row.get(1)?;
            let parents: Vec<CheckpointId> = serde_json::from_slice(&parents_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let snap_ids_json: Vec<u8> = row.get(2)?;
            let baseline_snapshots: Vec<SnapshotId> = serde_json::from_slice(&snap_ids_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let author: String = row.get(3)?;
            let message: String = row.get(4)?;
            let git_anchor: Option<String> = row.get(5)?;
            let created_at: i64 = row.get(6)?;

            Ok(Checkpoint {
                id: ContentId(id_arr),
                parents,
                baseline_snapshots,
                metadata: crate::checkpoint::checkpoint::CheckpointMetadata {
                    author,
                    message,
                    git_anchor,
                },
                created_at,
            })
        })?;
        Ok(result)
    }

    fn checkpoint_exists(&self, id: &CheckpointId) -> StorageResult<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM checkpoints WHERE id = ?1")?;
        let count: i64 = stmt.query_row(rusqlite::params![&id.0.to_vec()], |row| row.get(0))?;
        Ok(count > 0)
    }

    fn list_checkpoints(&self) -> StorageResult<Vec<Checkpoint>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id FROM checkpoints ORDER BY created_at DESC")?;
        let ids: Vec<Vec<u8>> = stmt
            .query_map([], |row| row.get(0))
            .map_err(crate::StorageError::Database)?
            .filter_map(|r| r.ok())
            .collect();

        let mut result = Vec::new();
        drop(stmt);
        drop(conn);
        for id_bytes in ids {
            let mut id_arr = [0u8; 32];
            id_arr.copy_from_slice(&id_bytes);
            let id = ContentId(id_arr);
            result.push(self.get_checkpoint(&id)?);
        }
        Ok(result)
    }

    fn delete_checkpoint(&self, id: &CheckpointId) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "DELETE FROM checkpoints WHERE id = ?1",
            rusqlite::params![&id.0.to_vec()],
        )?;
        if affected == 0 {
            return Err(crate::StorageError::NotFound(format!(
                "checkpoint {} not found",
                id
            )));
        }
        Ok(())
    }
}

impl BranchStore for SqliteStorage {
    fn store_branch(&self, branch: &Branch) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO branches (name, head, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                branch.name,
                &branch.head.0.to_vec(),
                branch.created_at,
                branch.updated_at,
            ],
        )?;
        Ok(())
    }

    fn get_branch(&self, name: &str) -> StorageResult<Branch> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT name, head, created_at, updated_at FROM branches WHERE name = ?1"
        )?;

        let result = stmt.query_row(rusqlite::params![name], |row| {
            let name: String = row.get(0)?;
            let head_bytes: Vec<u8> = row.get(1)?;
            let mut head_arr = [0u8; 32];
            head_arr.copy_from_slice(&head_bytes);
            let created_at: i64 = row.get(2)?;
            let updated_at: i64 = row.get(3)?;

            Ok(Branch {
                name,
                head: ContentId(head_arr),
                created_at,
                updated_at,
            })
        })?;
        Ok(result)
    }

    fn update_branch_head(&self, name: &str, head: &CheckpointId) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "UPDATE branches SET head = ?1, updated_at = ?2 WHERE name = ?3",
            rusqlite::params![&head.0.to_vec(), now, name],
        )?;
        Ok(())
    }

    fn list_branches(&self) -> StorageResult<Vec<Branch>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT name, head, created_at, updated_at FROM branches ORDER BY name"
        )?;

        let branches = stmt
            .query_map([], |row| {
                let name: String = row.get(0)?;
                let head_bytes: Vec<u8> = row.get(1)?;
                let mut head_arr = [0u8; 32];
                head_arr.copy_from_slice(&head_bytes);
                let created_at: i64 = row.get(2)?;
                let updated_at: i64 = row.get(3)?;

                Ok(Branch {
                    name,
                    head: ContentId(head_arr),
                    created_at,
                    updated_at,
                })
            })
            .map_err(crate::StorageError::Database)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(branches)
    }

    fn delete_branch(&self, name: &str) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM branches WHERE name = ?1", rusqlite::params![name])?;
        Ok(())
    }
}

// DagStore implementation -

impl DagStore for SqliteStorage {
    fn store_dag(&self, dag: &CheckpointDag) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();

        // Convert to JSON-friendly format: hex string keys
        let nodes_serializable: std::collections::HashMap<String, Vec<String>> = dag
            .all_nodes()
            .iter()
            .map(|id| {
                let key = id.to_hex();
                let children: Vec<String> = dag.get_children(id).iter().map(|c| c.to_hex()).collect();
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
        let mut stmt = conn.prepare(
            "SELECT value FROM dag_store WHERE key = ?1"
        )?;
        let result = stmt.query_row(rusqlite::params!["checkpoint_dag"], |row| {
            let json: Vec<u8> = row.get(0)?;

            // Parse as generic Value first
            let parsed: serde_json::Value = serde_json::from_slice(&json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let nodes_map = parsed["nodes"]
                .as_object()
                .ok_or_else(|| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(std::fmt::Error))
                })?;

            let _gen_map = parsed["generation"]
                .as_object()
                .ok_or_else(|| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(std::fmt::Error))
                })?;

            let mut dag = CheckpointDag::new();

            // First pass: add all nodes
            for (hex_key, _) in nodes_map {
                if let Some(id) = crate::core::types::ContentId::from_hex(hex_key) {
                    dag.add_node(id);
                }
            }

            // Second pass: add edges
            for (hex_key, children_value) in nodes_map {
                if let Some(parent) = crate::core::types::ContentId::from_hex(hex_key) {
                    if let Some(children) = children_value.as_array() {
                        for child_val in children {
                            if let Some(child_hex) = child_val.as_str() {
                                if let Some(child) = crate::core::types::ContentId::from_hex(child_hex) {
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
        let mut stmt = conn
            .prepare("SELECT value FROM dag_store WHERE key = ?1")?;
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

// LayerStore implementation -

impl LayerStore for SqliteStorage {
    fn store_layer(&self, layer: &Layer) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        let partition_ids_json = serde_json::to_vec(&layer.partitions)
            .map_err(|e| crate::StorageError::Serialization(e.to_string()))?;
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT OR REPLACE INTO layers (layer_type, partition_ids, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                layer.layer_type.name(),
                partition_ids_json,
                now,
                now,
            ],
        )?;
        Ok(())
    }

    fn get_layer(&self, layer_type: &LayerType) -> StorageResult<Layer> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT layer_type, partition_ids FROM layers WHERE layer_type = ?1"
        )?;

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
        let mut stmt = conn.prepare(
            "SELECT layer_type FROM layers ORDER BY layer_type"
        )?;
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

impl<T: SnapshotStore + DeltaStore + PartitionStore + FileNodeStore + CheckpointStore + BranchStore + DagStore + LayerStore + AtomicOps>
    crate::storage::repository::Repository for T
{}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::Delta;
    use crate::core::file_node::FileNode;
    use crate::core::partition::Partition;
    use crate::core::snapshot::Snapshot;
    use crate::core::types::{
        Hunk, PartitionType, SourceType, DiffOp,
    };
    use std::path::PathBuf;

    fn create_test_storage() -> SqliteStorage {
        SqliteStorage::new_in_memory().unwrap()
    }

    fn create_test_file_node(path: &str, content: &[u8]) -> FileNode {
        FileNode::new(PathBuf::from(path), content)
    }

    fn create_test_delta(file: &FileNode) -> Delta {
        let hunk = Hunk {
            old_start: 1,
            old_len: 0,
            new_start: 1,
            new_len: 1,
            ops: vec![DiffOp::Insert {
                new_start: 1,
                lines: vec!["new line".to_string()],
            }],
        };
        let diff = LineDiff::new(vec![hunk]);
        Delta::new(file.clone(), diff, SourceType::Manual)
    }

    #[test]
    fn test_store_and_get_snapshot() {
        let storage = create_test_storage();
        let file = create_test_file_node("test.txt", b"original content");
        storage.store_file_node(&file, b"original content").unwrap();

        let delta = create_test_delta(&file);
        storage.store_delta(&delta).unwrap();

        let snapshot = Snapshot::new_initial(file, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();

        let retrieved = storage.get_snapshot(&snapshot.id).unwrap();
        assert_eq!(retrieved.id, snapshot.id);
        assert_eq!(retrieved.deltas.len(), 1);
        assert_eq!(retrieved.deltas[0], delta.id);
    }

    #[test]
    fn test_store_and_get_delta() {
        let storage = create_test_storage();
        let file = create_test_file_node("test.txt", b"content");
        let delta = create_test_delta(&file);

        storage.store_delta(&delta).unwrap();
        let retrieved = storage.get_delta(&delta.id).unwrap();

        assert_eq!(retrieved.id, delta.id);
        assert_eq!(retrieved.file.path_str(), "test.txt");
    }

    #[test]
    fn test_file_node_roundtrip() {
        let storage = create_test_storage();
        let file = create_test_file_node("hello.txt", b"hello world");

        storage.store_file_node(&file, b"hello world").unwrap();
        assert!(storage.file_node_exists(file.path_str(), &file.base_hash).unwrap());

        let content = storage.get_file_content(file.path_str(), &file.base_hash).unwrap();
        assert_eq!(content, b"hello world");
    }

    #[test]
    fn test_partition_crud() {
        let storage = create_test_storage();
        let file = create_test_file_node("test.txt", b"initial");
        storage.store_file_node(&file, b"initial").unwrap();

        let delta = create_test_delta(&file);
        storage.store_delta(&delta).unwrap();

        let snapshot = Snapshot::new_initial(file, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();

        // Creating Partitions
        let partition = Partition::new(
            "test_partition".to_string(),
            PartitionType::Manual,
            snapshot.id,
        );
        storage.create_partition(&partition).unwrap();

        // Get Partition
        let retrieved = storage.get_partition(&partition.id).unwrap();
        assert_eq!(retrieved.name, "test_partition");
        assert_eq!(retrieved.current_snapshot, snapshot.id);
        assert_eq!(retrieved.history.len(), 1);

        // Get by Name
        let by_name = storage.get_partition_by_name("test_partition").unwrap();
        assert_eq!(by_name.id, partition.id);

        // Updating the pointer
        let snapshot2 = Snapshot::from_parent(&snapshot, delta.id, "manual".to_string());
        storage.store_snapshot(&snapshot2, b"").unwrap();
        storage.update_pointer(&partition.id, &snapshot2.id).unwrap();

        let updated = storage.get_partition(&partition.id).unwrap();
        assert_eq!(updated.current_snapshot, snapshot2.id);
        assert_eq!(updated.history.len(), 2);
    }

    #[test]
    fn test_list_partitions() {
        let storage = create_test_storage();
        let file = create_test_file_node("test.txt", b"content");
        storage.store_file_node(&file, b"content").unwrap();
        let delta = create_test_delta(&file);
        let snapshot = Snapshot::new_initial(file, delta.id);
        storage.store_delta(&delta).unwrap();
        storage.store_snapshot(&snapshot, b"").unwrap();

        let p1 = Partition::new("p1".to_string(), PartitionType::Manual, snapshot.id);
        let p2 = Partition::new(
            "p2".to_string(),
            PartitionType::Agent("agent1".into()),
            snapshot.id,
        );

        storage.create_partition(&p1).unwrap();
        storage.create_partition(&p2).unwrap();

        let list = storage.list_partitions().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_delta_exists() {
        let storage = create_test_storage();
        let file = create_test_file_node("test.txt", b"content");
        let delta = create_test_delta(&file);

        assert!(!storage.delta_exists(&delta.id).unwrap());
        storage.store_delta(&delta).unwrap();
        assert!(storage.delta_exists(&delta.id).unwrap());
    }

    #[test]
    fn test_snapshot_exists() {
        let storage = create_test_storage();
        let file = create_test_file_node("test.txt", b"content");
        storage.store_file_node(&file, b"content").unwrap();

        let delta = create_test_delta(&file);
        storage.store_delta(&delta).unwrap();

        let snapshot = Snapshot::new_initial(file, delta.id);
        assert!(!storage.snapshot_exists(&snapshot.id).unwrap());
        storage.store_snapshot(&snapshot, b"").unwrap();
        assert!(storage.snapshot_exists(&snapshot.id).unwrap());
    }

    #[test]
    fn test_find_snapshots_by_file() {
        let storage = create_test_storage();
        let file = create_test_file_node("test.txt", b"content");
        storage.store_file_node(&file, b"content").unwrap();

        let delta = create_test_delta(&file);
        storage.store_delta(&delta).unwrap();

        let s1 = Snapshot::new_initial(file.clone(), delta.id);
        storage.store_snapshot(&s1, b"").unwrap();

        let s2 = Snapshot::from_parent(&s1, delta.id, "manual".to_string());
        storage.store_snapshot(&s2, b"").unwrap();

        let found = storage.find_snapshots_by_file("test.txt").unwrap();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn test_transaction_rollback() {
        let storage = create_test_storage();

        // Deliberate failure in a transaction to validate rollback
        let result: StorageResult<()> = storage.with_transaction(|conn| {
            conn.execute("INSERT INTO layers (layer_type, partition_ids, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params!["test_layer", b"[]", 1000, 1000])?;

            // Create an error - transaction should be rolled back
            Err(crate::StorageError::Database(
                rusqlite::Error::InvalidParameterName("rollback test".to_string()),
            ))
        });

        assert!(result.is_err());

        // Verify that the transaction has rolled back and the table should be empty
        let conn = storage.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM layers WHERE layer_type = ?1", params!["test_layer"], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_partition_advance_and_rollback() {
        let storage = create_test_storage();
        let file = create_test_file_node("test.txt", b"content");
        let delta = create_test_delta(&file);

        storage.store_file_node(&file, b"content").unwrap();
        storage.store_delta(&delta).unwrap();

        let s1 = Snapshot::new_initial(file.clone(), delta.id);
        let s2 = Snapshot::from_parent(&s1, delta.id, "manual".to_string());
        let s3 = Snapshot::from_parent(&s2, delta.id, "manual".to_string());

        storage.store_snapshot(&s1, b"").unwrap();
        storage.store_snapshot(&s2, b"").unwrap();
        storage.store_snapshot(&s3, b"").unwrap();

        let mut partition = Partition::new("rollback_test".to_string(), PartitionType::Manual, s1.id);
        assert_eq!(partition.history.len(), 1);

        partition.advance(s2.id);
        assert_eq!(partition.history.len(), 2);
        assert_eq!(partition.current_snapshot, s2.id);

        partition.advance(s3.id);
        assert_eq!(partition.history.len(), 3);

        let prev = partition.rollback_one();
        assert_eq!(prev, Some(s2.id));
        assert_eq!(partition.current_snapshot, s2.id);
        assert_eq!(partition.history.len(), 2);

        assert!(partition.rollback_to(&s1.id));
        assert_eq!(partition.current_snapshot, s1.id);
        assert_eq!(partition.history.len(), 1);
    }

    #[test]
    fn test_find_snapshots_by_partition() {
        let storage = create_test_storage();
        let file = create_test_file_node("multi.txt", b"multi");
        storage.store_file_node(&file, b"multi").unwrap();
        let delta = create_test_delta(&file);
        storage.store_delta(&delta).unwrap();

        // Create a base snapshot
        let base = Snapshot::new_initial(file.clone(), delta.id);
        storage.store_snapshot(&base, b"").unwrap();

        // Create snapshots with explicit partition types (from_parent sets the partition_type)
        let s1 = Snapshot::from_parent(&base, delta.id, PartitionType::Manual.name());
        storage.store_snapshot(&s1, b"").unwrap();

        let s2 = Snapshot::from_parent(&base, delta.id, PartitionType::Agent("agent_test".into()).name());
        storage.store_snapshot(&s2, b"").unwrap();

        let manual_snapshots = storage.find_snapshots_by_partition(&PartitionType::Manual).unwrap();
        assert_eq!(manual_snapshots.len(), 1, "should find 1 manual snapshot");

        let agent_snapshots = storage.find_snapshots_by_partition(&PartitionType::Agent("agent_test".into())).unwrap();
        assert_eq!(agent_snapshots.len(), 1, "should find 1 agent_test snapshot");
    }

    #[test]
    fn test_store_and_get_deltas_batch() {
        let storage = create_test_storage();

        let file1 = create_test_file_node("f1.txt", b"content1");
        let delta1 = create_test_delta(&file1);
        storage.store_delta(&delta1).unwrap();

        let file2 = create_test_file_node("f2.txt", b"content2");
        let delta2 = create_test_delta(&file2);
        storage.store_delta(&delta2).unwrap();

        let deltas = storage.get_deltas(&[delta1.id, delta2.id]).unwrap();
        assert_eq!(deltas.len(), 2, "should retrieve both deltas");
        let ids: Vec<_> = deltas.iter().map(|d| d.id).collect();
        assert!(ids.contains(&delta1.id));
        assert!(ids.contains(&delta2.id));
    }

    // --- CheckpointStore tests ---

    fn create_full_storage() -> SqliteStorage {
        let conn = Connection::open_in_memory().unwrap();
        crate::storage::migrations::initialize_full(&conn).unwrap();
        let conn = std::sync::Arc::new(std::sync::Mutex::new(conn));
        SqliteStorage::new_with_connection_arc(&conn)
    }

    fn make_checkpoint_id(data: &[u8]) -> CheckpointId {
        crate::core::types::ContentId::from_content(data)
    }

    fn make_snapshot_id(data: &[u8]) -> SnapshotId {
        crate::core::types::ContentId::from_content(data)
    }

    #[test]
    fn test_checkpoint_store_roundtrip() {
        let storage = create_full_storage();
        let snap_id = make_snapshot_id(b"snap1");
        let cp = Checkpoint::new(
            vec![snap_id],
            vec![],
            crate::checkpoint::checkpoint::CheckpointMetadata::new("author1", "msg1"),
        );

        storage.store_checkpoint(&cp).unwrap();
        assert!(storage.checkpoint_exists(&cp.id).unwrap());

        let retrieved = storage.get_checkpoint(&cp.id).unwrap();
        assert_eq!(retrieved.metadata.author, "author1");
        assert_eq!(retrieved.metadata.message, "msg1");
        assert_eq!(retrieved.baseline_snapshots, vec![snap_id]);
    }

    #[test]
    fn test_checkpoint_list_and_delete() {
        let storage = create_full_storage();

        let cp1 = Checkpoint::new(
            vec![make_snapshot_id(b"a")],
            vec![],
            crate::checkpoint::checkpoint::CheckpointMetadata::new("author1", "first"),
        );
        let cp2 = Checkpoint::new(
            vec![make_snapshot_id(b"b")],
            vec![cp1.id],
            crate::checkpoint::checkpoint::CheckpointMetadata::new("author2", "second"),
        );

        storage.store_checkpoint(&cp1).unwrap();
        storage.store_checkpoint(&cp2).unwrap();

        let list = storage.list_checkpoints().unwrap();
        assert_eq!(list.len(), 2, "should list 2 checkpoints");

        storage.delete_checkpoint(&cp1.id).unwrap();
        assert!(!storage.checkpoint_exists(&cp1.id).unwrap());
        // Second checkpoint should still exist
        assert!(storage.checkpoint_exists(&cp2.id).unwrap());
    }

    #[test]
    fn test_delete_checkpoint_not_found() {
        let storage = create_full_storage();
        let fake_id = make_checkpoint_id(b"nonexistent");
        let result = storage.delete_checkpoint(&fake_id);
        assert!(result.is_err(), "deleting non-existent checkpoint should fail");
    }

    #[test]
    fn test_checkpoint_exists_false() {
        let storage = create_full_storage();
        let fake_id = make_checkpoint_id(b"nope");
        assert!(!storage.checkpoint_exists(&fake_id).unwrap());
    }

    // --- BranchStore tests ---

    #[test]
    fn test_branch_store_roundtrip() {
        let storage = create_full_storage();
        let cp_id = make_checkpoint_id(b"branch-root");
        let branch = Branch::new("main", cp_id);

        storage.store_branch(&branch).unwrap();

        let retrieved = storage.get_branch("main").unwrap();
        assert_eq!(retrieved.name, "main");
        assert_eq!(retrieved.head, cp_id);
    }

    #[test]
    fn test_branch_update_head() {
        let storage = create_full_storage();
        let cp1 = make_checkpoint_id(b"head1");
        let branch = Branch::new("feature", cp1);
        storage.store_branch(&branch).unwrap();

        let cp2 = make_checkpoint_id(b"head2");
        storage.update_branch_head("feature", &cp2).unwrap();

        let updated = storage.get_branch("feature").unwrap();
        assert_eq!(updated.head, cp2);
    }

    #[test]
    fn test_branch_list_and_delete() {
        let storage = create_full_storage();
        let cp_id = make_checkpoint_id(b"root");

        let b1 = Branch::new("main", cp_id);
        let b2 = Branch::new("develop", cp_id);
        storage.store_branch(&b1).unwrap();
        storage.store_branch(&b2).unwrap();

        let list = storage.list_branches().unwrap();
        assert_eq!(list.len(), 2);

        storage.delete_branch("develop").unwrap();
        let list = storage.list_branches().unwrap();
        assert_eq!(list.len(), 1);
    }

    // --- DagStore tests ---

    #[test]
    fn test_dag_store_roundtrip() {
        let storage = create_full_storage();
        let mut dag = CheckpointDag::new();

        let a = make_checkpoint_id(b"node-a");
        let b = make_checkpoint_id(b"node-b");
        dag.add_node(a);
        dag.add_node(b);
        dag.add_edge(a, b);

        storage.store_dag(&dag).unwrap();

        let loaded = storage.load_dag().unwrap();
        assert!(loaded.has_node(&a));
        assert!(loaded.has_node(&b));
        assert!(loaded.is_ancestor(&a, &b));
    }

    #[test]
    fn test_dag_load_empty() {
        let storage = create_full_storage();
        let dag = storage.load_dag().unwrap();
        assert!(dag.is_empty(), "should return empty DAG when no DAG stored");
    }

    // --- Repetitive ops tests ---

    #[test]
    fn test_sqlite_storage_repeated_snapshot_ops() {
        let storage = create_test_storage();
        let file = create_test_file_node("stratum.txt", b"stratum content");
        storage.store_file_node(&file, b"stratum content").unwrap();

        assert!(storage.file_node_exists(file.path_str(), &file.base_hash).unwrap());

        let delta = create_test_delta(&file);
        storage.store_delta(&delta).unwrap();

        let snapshot = Snapshot::new_initial(file, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();

        let retrieved = storage.get_snapshot(&snapshot.id).unwrap();
        assert_eq!(retrieved.id, snapshot.id);
    }

    #[test]
    fn test_sqlite_storage_repeated_partition_ops() {
        let storage = create_test_storage();
        let file = create_test_file_node("sp.txt", b"sp");
        storage.store_file_node(&file, b"sp").unwrap();
        let delta = create_test_delta(&file);
        storage.store_delta(&delta).unwrap();
        let snapshot = Snapshot::new_initial(file, delta.id);
        storage.store_snapshot(&snapshot, b"").unwrap();

        let partition = Partition::new("stratum-partition".to_string(), PartitionType::Manual, snapshot.id);
        storage.create_partition(&partition).unwrap();

        let retrieved = storage.get_partition(&partition.id).unwrap();
        assert_eq!(retrieved.name, "stratum-partition");
    }
}
