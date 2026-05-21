use crate::core::delta::{Delta, LineDiff};
use crate::core::file_node::FileNode;
use crate::core::partition::Partition;
use crate::core::snapshot::Snapshot;
use crate::core::types::{ContentId, DeltaId, PartitionId, PartitionType, SnapshotId};
use crate::storage::migrations;
use crate::storage::repository::{DeltaStore, FileNodeStore, PartitionStore, SnapshotStore};
use crate::StorageResult;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

/// SQLite 存储实现
pub struct SqliteStorage {
    conn: Mutex<Connection>,
}

impl SqliteStorage {
    /// 创建新的 SQLite 存储（内存数据库）
    pub fn new_in_memory() -> StorageResult<Self> {
        let conn = Connection::open_in_memory()?;
        migrations::initialize_database(&conn)?;
        Ok(SqliteStorage {
            conn: Mutex::new(conn),
        })
    }

    /// 创建新的 SQLite 存储（文件数据库）
    pub fn new(path: &Path) -> StorageResult<Self> {
        let conn = Connection::open(path)?;
        migrations::initialize_database(&conn)?;
        Ok(SqliteStorage {
            conn: Mutex::new(conn),
        })
    }

    /// 创建新的 SQLite 存储（文件数据库，含检查点表）
    pub fn new_full(path: &Path) -> StorageResult<Self> {
        let conn = Connection::open(path)?;
        migrations::initialize_full(&conn)?;
        Ok(SqliteStorage {
            conn: Mutex::new(conn),
        })
    }

    /// 获取内部连接的引用（用于事务等）
    pub fn with_conn<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&Connection) -> StorageResult<T>,
    {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }

    /// 执行事务
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

// ── SnapshotStore 实现 ──

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

    fn find_snapshots_by_partition(&self, partition_type: &str) -> StorageResult<Vec<Snapshot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, file_path, file_hash, deltas, parents, partition_type, created_at
             FROM snapshots WHERE partition_type = ?1 ORDER BY created_at DESC"
        )?;

        let snapshots = stmt.query_map(params![partition_type], |row| {
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

// ── DeltaStore 实现 ──

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

// ── FileNodeStore 实现 ──

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

    fn get_file_content(&self, file_node: &FileNode) -> StorageResult<Vec<u8>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT content FROM file_nodes WHERE file_path = ?1 AND base_hash = ?2"
        )?;
        let content: Vec<u8> = stmt.query_row(
            params![file_node.path_str(), &file_node.base_hash.to_vec()],
            |row| row.get(0),
        )?;
        Ok(content)
    }

    fn file_node_exists(&self, file_node: &FileNode) -> StorageResult<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM file_nodes WHERE file_path = ?1 AND base_hash = ?2"
        )?;
        let count: i64 = stmt.query_row(
            params![file_node.path_str(), &file_node.base_hash.to_vec()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

// ── PartitionStore 实现 ──

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

        // 写入历史
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

        // 查询当前最大 seq
        let max_seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), -1) FROM partition_history WHERE partition_id = ?1",
                params![&partition_id.as_bytes().to_vec()],
                |row| row.get(0),
            )
            .unwrap_or(-1);

        // 插入新的历史记录
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

        // 读取历史
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

        // 重建 Partition
        // 用从数据库中读取的实际值覆盖占位值
        let (id_bytes, name, snap_arr, partition_type) = partition;
        // id_bytes 是 Vec<u8>，我们需要重建 PartitionId
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

// ── 组合存储 ──

/// 组合存储结构体，同时实现了所有存储 trait
pub struct StratumStorage {
    pub inner: SqliteStorage,
}

impl StratumStorage {
    pub fn new_in_memory() -> StorageResult<Self> {
        Ok(StratumStorage {
            inner: SqliteStorage::new_in_memory()?,
        })
    }

    pub fn new(path: &Path) -> StorageResult<Self> {
        Ok(StratumStorage {
            inner: SqliteStorage::new(path)?,
        })
    }

    pub fn new_full(path: &Path) -> StorageResult<Self> {
        Ok(StratumStorage {
            inner: SqliteStorage::new_full(path)?,
        })
    }
}

impl SnapshotStore for StratumStorage {
    fn store_snapshot(&self, snapshot: &Snapshot, content: &[u8]) -> StorageResult<()> {
        self.inner.store_snapshot(snapshot, content)
    }
    fn get_snapshot(&self, id: &SnapshotId) -> StorageResult<Snapshot> {
        self.inner.get_snapshot(id)
    }
    fn find_snapshots_by_file(&self, file_path: &str) -> StorageResult<Vec<Snapshot>> {
        self.inner.find_snapshots_by_file(file_path)
    }
    fn find_snapshots_by_partition(&self, partition_type: &str) -> StorageResult<Vec<Snapshot>> {
        self.inner.find_snapshots_by_partition(partition_type)
    }
    fn snapshot_exists(&self, id: &SnapshotId) -> StorageResult<bool> {
        self.inner.snapshot_exists(id)
    }
}

impl DeltaStore for StratumStorage {
    fn store_delta(&self, delta: &Delta) -> StorageResult<()> {
        self.inner.store_delta(delta)
    }
    fn get_delta(&self, id: &DeltaId) -> StorageResult<Delta> {
        self.inner.get_delta(id)
    }
    fn get_deltas(&self, ids: &[DeltaId]) -> StorageResult<Vec<Delta>> {
        self.inner.get_deltas(ids)
    }
    fn delta_exists(&self, id: &DeltaId) -> StorageResult<bool> {
        self.inner.delta_exists(id)
    }
}

impl FileNodeStore for StratumStorage {
    fn store_file_node(&self, file_node: &FileNode, content: &[u8]) -> StorageResult<()> {
        self.inner.store_file_node(file_node, content)
    }
    fn get_file_content(&self, file_node: &FileNode) -> StorageResult<Vec<u8>> {
        self.inner.get_file_content(file_node)
    }
    fn file_node_exists(&self, file_node: &FileNode) -> StorageResult<bool> {
        self.inner.file_node_exists(file_node)
    }
}

impl PartitionStore for StratumStorage {
    fn create_partition(&self, partition: &Partition) -> StorageResult<()> {
        self.inner.create_partition(partition)
    }
    fn update_pointer(&self, partition_id: &PartitionId, snapshot_id: &SnapshotId) -> StorageResult<()> {
        self.inner.update_pointer(partition_id, snapshot_id)
    }
    fn get_partition(&self, id: &PartitionId) -> StorageResult<Partition> {
        self.inner.get_partition(id)
    }
    fn get_partition_by_name(&self, name: &str) -> StorageResult<Partition> {
        self.inner.get_partition_by_name(name)
    }
    fn list_partitions(&self) -> StorageResult<Vec<Partition>> {
        self.inner.list_partitions()
    }
}

impl<T: SnapshotStore + DeltaStore + PartitionStore + FileNodeStore> crate::storage::repository::Repository for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::{Delta, LineDiff};
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
        assert!(storage.file_node_exists(&file).unwrap());

        let content = storage.get_file_content(&file).unwrap();
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

        // 创建分区
        let partition = Partition::new(
            "test_partition".to_string(),
            PartitionType::Manual,
            snapshot.id,
        );
        storage.create_partition(&partition).unwrap();

        // 获取分区
        let retrieved = storage.get_partition(&partition.id).unwrap();
        assert_eq!(retrieved.name, "test_partition");
        assert_eq!(retrieved.current_snapshot, snapshot.id);
        assert_eq!(retrieved.history.len(), 1);

        // 按名称获取
        let by_name = storage.get_partition_by_name("test_partition").unwrap();
        assert_eq!(by_name.id, partition.id);

        // 更新指针
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

        // 故意在事务中失败，验证回滚
        let result: StorageResult<()> = storage.with_transaction(|conn| {
            conn.execute("INSERT INTO layers (layer_type, partition_ids, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params!["test_layer", b"[]", 1000, 1000])?;

            // 制造一个错误 — 事务应回滚
            Err(crate::StorageError::Database(
                rusqlite::Error::InvalidParameterName("rollback test".to_string()),
            ))
        });

        assert!(result.is_err());

        // 验证事务已回滚，表应该为空
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

        // 回退一步
        let prev = partition.rollback_one();
        assert_eq!(prev, Some(s2.id));
        assert_eq!(partition.current_snapshot, s2.id);
        assert_eq!(partition.history.len(), 2);

        // 回退到指定位置
        assert!(partition.rollback_to(&s1.id));
        assert_eq!(partition.current_snapshot, s1.id);
        assert_eq!(partition.history.len(), 1);
    }
}
