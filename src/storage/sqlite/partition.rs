use crate::core::partition::Partition;
use crate::core::types::{ContentId, PartitionId, PartitionType, SnapshotId};
use crate::storage::repository::PartitionStore;
use crate::storage::sqlite::connection::SqliteStorage;
use crate::StorageResult;
use rusqlite::{params, Connection};

impl PartitionStore for SqliteStorage {
    fn create_partition(&self, partition: &Partition) -> StorageResult<()> {
        let conn = self.conn.lock();
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

    fn update_pointer(
        &self,
        partition_id: &PartitionId,
        snapshot_id: &SnapshotId,
    ) -> StorageResult<()> {
        let conn = self.conn.lock();
        Self::update_pointer_internal(&conn, partition_id, snapshot_id)
    }

    fn get_partition(&self, id: &PartitionId) -> StorageResult<Partition> {
        let conn = self.conn.lock();
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

        let mut hist_stmt = conn.prepare(
            "SELECT snapshot_id, seq FROM partition_history WHERE partition_id = ?1 ORDER BY seq",
        )?;
        let history: Vec<SnapshotId> = hist_stmt
            .query_map(params![&id.as_bytes().to_vec()], |row| {
                let snap_bytes: Vec<u8> = row.get(0)?;
                let mut snap_arr = [0u8; 32];
                snap_arr.copy_from_slice(&snap_bytes);
                Ok(ContentId(snap_arr))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let (id_bytes, name, snap_arr, partition_type) = partition;
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
        let conn = self.conn.lock();

        let mut stmt = conn.prepare(
            "SELECT id, name, current_snapshot, partition_type, partition_data, created_at, updated_at
             FROM partitions WHERE name = ?1"
        )?;

        let (id_bytes, name_ret, snap_bytes, partition_data) =
            stmt.query_row(params![name], |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                let name: String = row.get(1)?;
                let snap_bytes: Vec<u8> = row.get(2)?;
                let partition_data: Option<String> = row.get(4)?;
                Ok((id_bytes, name, snap_bytes, partition_data))
            })?;

        let id = uuid::Uuid::from_slice(&id_bytes)
            .map_err(|e| crate::StorageError::Serialization(e.to_string()))?;

        let mut snap_arr = [0u8; 32];
        snap_arr.copy_from_slice(&snap_bytes);

        let partition_type: PartitionType = partition_data
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(PartitionType::Manual);

        let mut hist_stmt = conn.prepare(
            "SELECT snapshot_id, seq FROM partition_history WHERE partition_id = ?1 ORDER BY seq",
        )?;
        let history: Vec<SnapshotId> = hist_stmt
            .query_map(params![&id_bytes], |row| {
                let snap_bytes: Vec<u8> = row.get(0)?;
                let mut snap_arr = [0u8; 32];
                snap_arr.copy_from_slice(&snap_bytes);
                Ok(ContentId(snap_arr))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Partition {
            id,
            name: name_ret,
            current_snapshot: ContentId(snap_arr),
            history,
            partition_type,
        })
    }

    fn list_partitions(&self) -> StorageResult<Vec<Partition>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, current_snapshot, partition_type, partition_data, created_at, updated_at
             FROM partitions ORDER BY name"
        )?;

        let rows: Vec<(Vec<u8>, String, Vec<u8>, Option<String>)> = stmt
            .query_map([], |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                let name: String = row.get(1)?;
                let snap_bytes: Vec<u8> = row.get(2)?;
                let partition_data: Option<String> = row.get(4)?;
                Ok((id_bytes, name, snap_bytes, partition_data))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut result = Vec::new();
        for (id_bytes, name, snap_bytes, partition_data) in rows {
            let id = uuid::Uuid::from_slice(&id_bytes)
                .map_err(|e| crate::StorageError::Serialization(e.to_string()))?;

            let mut snap_arr = [0u8; 32];
            snap_arr.copy_from_slice(&snap_bytes);

            let partition_type: PartitionType = partition_data
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(PartitionType::Manual);

            let mut hist_stmt = conn.prepare(
                "SELECT snapshot_id, seq FROM partition_history WHERE partition_id = ?1 ORDER BY seq",
            )?;
            let history: Vec<SnapshotId> = hist_stmt
                .query_map(params![&id_bytes], |row| {
                    let snap_bytes: Vec<u8> = row.get(0)?;
                    let mut snap_arr = [0u8; 32];
                    snap_arr.copy_from_slice(&snap_bytes);
                    Ok(ContentId(snap_arr))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            result.push(Partition {
                id,
                name,
                current_snapshot: ContentId(snap_arr),
                history: history.clone(),
                partition_type,
            });
        }
        Ok(result)
    }
}

impl SqliteStorage {
    fn update_pointer_internal(
        conn: &Connection,
        partition_id: &PartitionId,
        snapshot_id: &SnapshotId,
    ) -> StorageResult<()> {
        let now = chrono::Utc::now().timestamp_millis();

        conn.execute(
            "UPDATE partitions SET current_snapshot = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                &snapshot_id.0.to_vec(),
                now,
                &partition_id.as_bytes().to_vec()
            ],
        )?;

        let max_seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), -1) FROM partition_history WHERE partition_id = ?1",
            params![&partition_id.as_bytes().to_vec()],
            |row| row.get(0),
        )?;

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
}
