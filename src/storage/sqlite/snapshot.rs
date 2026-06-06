use crate::core::file_node::FileNode;
use crate::core::snapshot::Snapshot;
use crate::core::types::{ContentId, SnapshotId};
use crate::storage::repository::SnapshotStore;
use crate::storage::sqlite::connection::SqliteStorage;
use crate::StorageResult;
use rusqlite::{params, Row};

fn bytes_to_array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut arr = [0u8; N];
    arr.copy_from_slice(bytes);
    arr
}

fn row_to_snapshot(row: &Row) -> Result<Snapshot, rusqlite::Error> {
    let id_bytes: Vec<u8> = row.get(0)?;
    let id = ContentId(bytes_to_array(&id_bytes));

    let file_path: String = row.get(1)?;
    let file_hash_bytes: Vec<u8> = row.get(2)?;
    let file_hash = bytes_to_array(&file_hash_bytes);

    let deltas_json: Vec<u8> = row.get(3)?;
    let parents_json: Vec<u8> = row.get(4)?;
    let partition_type: String = row.get(5)?;
    let created_at: i64 = row.get(6)?;
    let has_conflicts: bool = row.get::<_, i32>(7)? != 0;

    let deltas: Vec<crate::core::types::DeltaId> = serde_json::from_slice(&deltas_json)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let parents: Vec<SnapshotId> = serde_json::from_slice(&parents_json)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

    Ok(Snapshot {
        id,
        file: FileNode {
            file_path: std::path::PathBuf::from(file_path),
            base_hash: file_hash,
        },
        deltas,
        parents,
        partition_type,
        created_at,
        has_conflicts,
    })
}

fn row_to_snapshot_lenient(row: &Row) -> Snapshot {
    let id_bytes: Vec<u8> = row.get(0).unwrap_or_default();
    let id = ContentId(bytes_to_array(&id_bytes));

    let file_path: String = row.get(1).unwrap_or_default();
    let file_hash_bytes: Vec<u8> = row.get(2).unwrap_or_default();
    let file_hash = bytes_to_array(&file_hash_bytes);

    let deltas_json: Vec<u8> = row.get(3).unwrap_or_default();
    let parents_json: Vec<u8> = row.get(4).unwrap_or_default();
    let partition_type: String = row.get(5).unwrap_or_default();
    let created_at: i64 = row.get(6).unwrap_or(0);
    let has_conflicts: bool = row.get::<_, i32>(7).unwrap_or(0) != 0;

    let deltas: Vec<crate::core::types::DeltaId> =
        serde_json::from_slice(&deltas_json).unwrap_or_default();
    let parents: Vec<SnapshotId> = serde_json::from_slice(&parents_json).unwrap_or_default();

    Snapshot {
        id,
        file: FileNode {
            file_path: std::path::PathBuf::from(file_path),
            base_hash: file_hash,
        },
        deltas,
        parents,
        partition_type,
        created_at,
        has_conflicts,
    }
}

impl SnapshotStore for SqliteStorage {
    fn store_snapshot(&self, snapshot: &Snapshot, _content: &[u8]) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        let deltas_json = serde_json::to_vec(&snapshot.deltas)?;
        let parents_json = serde_json::to_vec(&snapshot.parents)?;

        conn.execute(
            "INSERT OR IGNORE INTO snapshots (id, file_path, file_hash, deltas, parents, partition_type, created_at, has_conflicts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &snapshot.id.0.to_vec(),
                snapshot.file.path_str(),
                &snapshot.file.base_hash.to_vec(),
                deltas_json,
                parents_json,
                snapshot.partition_type,
                snapshot.created_at,
                snapshot.has_conflicts as i32,
            ],
        )?;
        Ok(())
    }

    fn get_snapshot(&self, id: &SnapshotId) -> StorageResult<Snapshot> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, file_path, file_hash, deltas, parents, partition_type, created_at, has_conflicts FROM snapshots WHERE id = ?1"
        )?;

        let result = stmt.query_row(params![&id.0.to_vec()], row_to_snapshot)?;
        Ok(result)
    }

    fn find_snapshots_by_file(&self, file_path: &str) -> StorageResult<Vec<Snapshot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, file_path, file_hash, deltas, parents, partition_type, created_at, has_conflicts
             FROM snapshots WHERE file_path = ?1 ORDER BY created_at DESC",
        )?;

        let snapshots =
            stmt.query_map(params![file_path], |row| Ok(row_to_snapshot_lenient(row)))?;

        let mut result = Vec::new();
        for s in snapshots {
            result.push(s?);
        }
        Ok(result)
    }

    fn find_snapshots_by_partition(
        &self,
        partition_type: &crate::core::types::PartitionType,
    ) -> StorageResult<Vec<Snapshot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, file_path, file_hash, deltas, parents, partition_type, created_at, has_conflicts
             FROM snapshots WHERE partition_type = ?1 ORDER BY created_at DESC",
        )?;

        let snapshots = stmt.query_map(params![partition_type.name()], |row| {
            Ok(row_to_snapshot_lenient(row))
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
