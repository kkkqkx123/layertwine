use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};

use crate::backup::backup_snapshot::{BackupFilter, BackupSnapshot};
use crate::core::delta::Delta;
use crate::core::snapshot::Snapshot;
use crate::core::types::{BackupId, ContentId, SnapshotId};
use crate::engine::diff::diff_to_line_diff;
use crate::engine::merge::apply_deltas;
use crate::error::{Result, StratumError};
use crate::storage::repository::{DeltaStore, FileNodeStore, PartitionStore, SnapshotStore};
use crate::{StorageError, StorageResult};

const BACKUP_MIGRATION_SQL: &str = "
CREATE TABLE IF NOT EXISTS backup_snapshots (
    id              BLOB PRIMARY KEY,
    source_snapshot BLOB NOT NULL,
    file_path       TEXT NOT NULL,
    file_hash       BLOB NOT NULL,
    deltas          BLOB NOT NULL,
    label           TEXT,
    backed_at       INTEGER NOT NULL,
    metadata        BLOB NOT NULL,
    agent_id        TEXT,
    source_type     TEXT
);

CREATE INDEX IF NOT EXISTS idx_backup_label ON backup_snapshots(label);
CREATE INDEX IF NOT EXISTS idx_backup_backed_at ON backup_snapshots(backed_at);
CREATE INDEX IF NOT EXISTS idx_backup_agent_id ON backup_snapshots(agent_id);
CREATE INDEX IF NOT EXISTS idx_backup_source_type ON backup_snapshots(source_type);
";

fn map_db_err(e: rusqlite::Error) -> StratumError {
    StratumError::Storage(StorageError::Database(e))
}

pub struct BackupRepo {
    conn: Arc<Mutex<Connection>>,
}

impl BackupRepo {
    pub fn new_in_memory() -> StorageResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(BACKUP_MIGRATION_SQL)?;
        Ok(BackupRepo {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn new(path: &Path) -> StorageResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(BACKUP_MIGRATION_SQL)?;
        Ok(BackupRepo {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn backup_snapshot<S>(
        &self,
        core_repo: &S,
        snapshot_id: SnapshotId,
        label: Option<String>,
    ) -> Result<BackupId>
    where
        S: SnapshotStore + DeltaStore,
    {
        let snapshot = core_repo
            .get_snapshot(&snapshot_id)
            .map_err(StratumError::Storage)?;

        let deltas: Vec<Delta> = core_repo
            .get_deltas(&snapshot.deltas)
            .map_err(StratumError::Storage)?;

        let backup = BackupSnapshot::new(snapshot_id, snapshot.file, deltas, label);
        self.store_backup(&backup)?;
        Ok(backup.id)
    }

    fn store_backup(&self, backup: &BackupSnapshot) -> StorageResult<()> {
        let conn = self.conn.lock().unwrap();
        let deltas_json = serde_json::to_vec(&backup.deltas)?;
        let metadata_json = serde_json::to_vec(&backup.metadata)?;

        conn.execute(
            "INSERT INTO backup_snapshots (id, source_snapshot, file_path, file_hash, deltas, label, backed_at, metadata, agent_id, source_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &backup.id.0.to_vec(),
                &backup.source_snapshot.0.to_vec(),
                backup.file.path_str(),
                &backup.file.base_hash.to_vec(),
                deltas_json,
                backup.label,
                backup.backed_at,
                metadata_json,
                backup.agent_id,
                backup.source_type,
            ],
        )?;
        Ok(())
    }

    pub fn get_backup(&self, backup_id: &BackupId) -> Result<BackupSnapshot> {
        let conn = self.conn.lock().map_err(|e| {
            StratumError::General(format!("mutex poisoned: {}", e))
        })?;
        let mut stmt = conn
            .prepare(
                "SELECT id, source_snapshot, file_path, file_hash, deltas, label, backed_at, metadata, agent_id, source_type
                 FROM backup_snapshots WHERE id = ?1",
            )
            .map_err(map_db_err)?;

        let result = stmt
            .query_row(params![&backup_id.0.to_vec()], |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                let mut id_arr = [0u8; 32];
                id_arr.copy_from_slice(&id_bytes);

                let src_bytes: Vec<u8> = row.get(1)?;
                let mut src_arr = [0u8; 32];
                src_arr.copy_from_slice(&src_bytes);

                let file_path: String = row.get(2)?;
                let file_hash_bytes: Vec<u8> = row.get(3)?;
                let mut fh_arr = [0u8; 32];
                fh_arr.copy_from_slice(&file_hash_bytes);

                let deltas_json: Vec<u8> = row.get(4)?;
                let label: Option<String> = row.get(5)?;
                let backed_at: i64 = row.get(6)?;
                let metadata_json: Vec<u8> = row.get(7)?;
                let agent_id: Option<String> = row.get(8)?;
                let source_type: Option<String> = row.get(9)?;

                let deltas: Vec<Delta> = serde_json::from_slice(&deltas_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let metadata: HashMap<String, String> = serde_json::from_slice(&metadata_json)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                Ok(BackupSnapshot {
                    id: ContentId(id_arr),
                    source_snapshot: ContentId(src_arr),
                    file: crate::core::file_node::FileNode {
                        file_path: std::path::PathBuf::from(file_path),
                        base_hash: fh_arr,
                    },
                    deltas,
                    label,
                    backed_at,
                    metadata,
                    agent_id,
                    source_type,
                })
            })
            .map_err(map_db_err)?;

        Ok(result)
    }

    pub fn query_backups(&self, filter: &BackupFilter) -> Result<Vec<BackupSnapshot>> {
        let conn = self.conn.lock().map_err(|e| {
            StratumError::General(format!("mutex poisoned: {}", e))
        })?;

        let mut sql = String::from(
            "SELECT id, source_snapshot, file_path, file_hash, deltas, label, backed_at, metadata, agent_id, source_type
             FROM backup_snapshots WHERE 1=1",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(src_id) = &filter.source_snapshot {
            sql.push_str(" AND source_snapshot = ?");
            param_values.push(Box::new(src_id.0.to_vec()));
        }
        if let Some((start, end)) = &filter.time_range {
            sql.push_str(" AND backed_at >= ? AND backed_at <= ?");
            param_values.push(Box::new(*start));
            param_values.push(Box::new(*end));
        }
        if let Some(label) = &filter.label {
            sql.push_str(" AND label = ?");
            param_values.push(Box::new(label.clone()));
        }
        if let Some(agent_id) = &filter.agent_id {
            sql.push_str(" AND agent_id = ?");
            param_values.push(Box::new(agent_id.clone()));
        }
        if let Some(source_type) = &filter.source_type {
            sql.push_str(" AND source_type = ?");
            param_values.push(Box::new(source_type.clone()));
        }

        sql.push_str(" ORDER BY backed_at DESC");

        let mut stmt = conn.prepare(&sql).map_err(map_db_err)?;

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                let mut id_arr = [0u8; 32];
                id_arr.copy_from_slice(&id_bytes);

                let src_bytes: Vec<u8> = row.get(1)?;
                let mut src_arr = [0u8; 32];
                src_arr.copy_from_slice(&src_bytes);

                let file_path: String = row.get(2)?;
                let file_hash_bytes: Vec<u8> = row.get(3)?;
                let mut fh_arr = [0u8; 32];
                fh_arr.copy_from_slice(&file_hash_bytes);

                let deltas_json: Vec<u8> = row.get(4)?;
                let label: Option<String> = row.get(5)?;
                let backed_at: i64 = row.get(6)?;
                let metadata_json: Vec<u8> = row.get(7)?;
                let agent_id: Option<String> = row.get(8)?;
                let source_type: Option<String> = row.get(9)?;

                let deltas: Vec<Delta> = serde_json::from_slice(&deltas_json).unwrap_or_default();
                let metadata: HashMap<String, String> =
                    serde_json::from_slice(&metadata_json).unwrap_or_default();

                Ok(BackupSnapshot {
                    id: ContentId(id_arr),
                    source_snapshot: ContentId(src_arr),
                    file: crate::core::file_node::FileNode {
                        file_path: std::path::PathBuf::from(file_path),
                        base_hash: fh_arr,
                    },
                    deltas,
                    label,
                    backed_at,
                    metadata,
                    agent_id,
                    source_type,
                })
            })
            .map_err(map_db_err)?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(map_db_err)?);
        }

        if let Some(meta_key) = &filter.metadata_key {
            result.retain(|b| {
                if let Some(meta_val) = &filter.metadata_value {
                    b.metadata.get(meta_key.as_str()).map(|s| s.as_str()) == Some(meta_val.as_str())
                } else {
                    b.metadata.contains_key(meta_key.as_str())
                }
            });
        }

        Ok(result)
    }

    pub fn delete_backup(&self, backup_id: &BackupId) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StratumError::General(format!("mutex poisoned: {}", e)))?;
        let affected = conn
            .execute(
                "DELETE FROM backup_snapshots WHERE id = ?1",
                params![&backup_id.0.to_vec()],
            )
            .map_err(map_db_err)?;

        if affected == 0 {
            return Err(StratumError::NotFound(format!(
                "backup {} not found",
                backup_id
            )));
        }
        Ok(())
    }

    pub fn count(&self) -> Result<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StratumError::General(format!("mutex poisoned: {}", e)))?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM backup_snapshots", [], |row| {
                row.get(0)
            })
            .map_err(map_db_err)?;
        Ok(count as u64)
    }

    pub fn merge_to_staged<S>(&self, backup_id: &BackupId, core_repo: &S) -> Result<SnapshotId>
    where
        S: SnapshotStore + DeltaStore + PartitionStore + FileNodeStore,
    {
        let backup = self.get_backup(backup_id)?;

        let integrity_ok = {
            let mut recomputed = BackupSnapshot::new(
                backup.source_snapshot,
                backup.file.clone(),
                backup.deltas.clone(),
                backup.label.clone(),
            );
            recomputed.backed_at = backup.backed_at;
            if let Some(agent_id) = &backup.agent_id {
                recomputed = recomputed.with_agent_id(agent_id);
            }
            if let Some(source_type) = &backup.source_type {
                recomputed = recomputed.with_source_type(source_type);
            }
            recomputed.id == backup.id
        };
        if !integrity_ok {
            return Err(StratumError::General(
                "backup data integrity check failed".to_string(),
            ));
        }

        fn strip_trailing_newline(s: &str) -> &str {
            s.strip_suffix('\n').unwrap_or(s)
        }

        // Step 1: Reconstruct the backed-up file content
        let base_content = core_repo.get_file_content(backup.file.path_str(), &backup.file.base_hash)?;
        let base_str = String::from_utf8(base_content)
            .map_err(|e| StratumError::General(format!("non-utf8 file content: {}", e)))?;
        // Normalise trailing newlines - apply_deltas uses .lines() which strips them
        let base_str = strip_trailing_newline(&base_str).to_string();
        let backup_text = apply_deltas(&base_str, &backup.deltas)?;

        // Step 2: Get current staged content
        let staged_partition = core_repo.get_partition_by_name("staged")?;
        let staged_snapshot = core_repo.get_snapshot(&staged_partition.current_snapshot)?;
        let staged_base = core_repo.get_file_content(staged_snapshot.file.path_str(), &staged_snapshot.file.base_hash)?;
        let staged_base_str = String::from_utf8(staged_base)
            .map_err(|e| StratumError::General(format!("non-utf8 file content: {}", e)))?;
        let staged_deltas = core_repo.get_deltas(&staged_snapshot.deltas)?;
        let staged_text = apply_deltas(strip_trailing_newline(&staged_base_str), &staged_deltas)?;

        // Step 3: Three-way merge
        //   base = original file content (common ancestor)
        //   ours = current staged content
        //   theirs = backed-up content
        let (merged_text, _conflicts) =
            crate::engine::merge::merge_texts(&base_str, &staged_text, &backup_text);

        // Step 4: Compute diff from base to merged result, create delta
        let diff = diff_to_line_diff(&base_str, &merged_text);
        let merge_delta = Delta::new(backup.file.clone(), diff, crate::core::types::SourceType::Backup);
        core_repo.store_delta(&merge_delta)?;

        // Step 5: Create merge snapshot with both parents
        let source_snapshot = core_repo.get_snapshot(&backup.source_snapshot)?;
        let merged = Snapshot::merge(
            vec![&staged_snapshot, &source_snapshot],
            merge_delta.id,
            "staged".to_string(),
        );
        core_repo.store_snapshot(&merged, &[])?;
        core_repo.update_pointer(&staged_partition.id, &merged.id)?;

        Ok(merged.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::LineDiff;
    use crate::core::file_node::FileNode;
    use crate::core::snapshot::Snapshot;
    use crate::core::types::{DeltaId, SnapshotId, SourceType};
    use crate::storage::repository::{DeltaStore, FileNodeStore, SnapshotStore};
    use crate::storage::sqlite_storage::SqliteStorage;
    use std::path::PathBuf;

    fn setup_core_repo() -> SqliteStorage {
        SqliteStorage::new_in_memory().unwrap()
    }

    fn create_test_snapshot(
        store: &SqliteStorage,
        path: &str,
        content: &[u8],
        source_type: SourceType,
    ) -> SnapshotId {
        let file_node = FileNode::new(PathBuf::from(path), content);
        store.store_file_node(&file_node, content).unwrap();

        let diff = LineDiff::new(vec![]);
        let delta = Delta::new(file_node.clone(), diff, source_type);
        store.store_delta(&delta).unwrap();

        let snapshot = Snapshot::new_initial(file_node, delta.id);
        store.store_snapshot(&snapshot, content).unwrap();

        snapshot.id
    }

    fn create_staged_partition(store: &SqliteStorage, snapshot_id: SnapshotId) {
        let partition = crate::core::partition::Partition {
            id: uuid::Uuid::new_v4(),
            name: "staged".to_string(),
            current_snapshot: snapshot_id,
            history: vec![snapshot_id],
            partition_type: crate::core::types::PartitionType::Staged,
        };
        store.create_partition(&partition).unwrap();
    }

    #[test]
    fn test_backup_snapshot() {
        let core = setup_core_repo();
        let backup_repo = BackupRepo::new_in_memory().unwrap();

        let snap_id = create_test_snapshot(&core, "test.txt", b"hello world", SourceType::Manual);

        let backup_id = backup_repo
            .backup_snapshot(&core, snap_id, Some("test-backup".to_string()))
            .unwrap();

        let loaded = backup_repo.get_backup(&backup_id).unwrap();
        assert_eq!(loaded.source_snapshot, snap_id);
        assert_eq!(loaded.label, Some("test-backup".to_string()));
        assert_eq!(loaded.deltas.len(), 1);
    }

    #[test]
    fn test_query_backups() {
        let core = setup_core_repo();
        let backup_repo = BackupRepo::new_in_memory().unwrap();

        let snap_id1 = create_test_snapshot(&core, "a.txt", b"content a", SourceType::Manual);
        let snap_id2 = create_test_snapshot(
            &core,
            "b.txt",
            b"content b",
            SourceType::Agent("agent-1".into()),
        );

        backup_repo
            .backup_snapshot(&core, snap_id1, Some("label-a".to_string()))
            .unwrap();
        backup_repo
            .backup_snapshot(&core, snap_id2, Some("label-b".to_string()))
            .unwrap();

        let all = backup_repo.query_backups(&BackupFilter::new()).unwrap();
        assert_eq!(all.len(), 2);

        let filtered = BackupFilter::new().with_label("label-a");
        let result = backup_repo.query_backups(&filtered).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, Some("label-a".to_string()));
    }

    #[test]
    fn test_delete_backup() {
        let core = setup_core_repo();
        let backup_repo = BackupRepo::new_in_memory().unwrap();

        let snap_id = create_test_snapshot(&core, "del.txt", b"delete me", SourceType::Manual);
        let backup_id = backup_repo.backup_snapshot(&core, snap_id, None).unwrap();

        assert_eq!(backup_repo.count().unwrap(), 1);
        backup_repo.delete_backup(&backup_id).unwrap();
        assert_eq!(backup_repo.count().unwrap(), 0);
    }

    /// Create a file node + delta + snapshot in a realistic chain.
    /// Returns (file_node, delta_id, snapshot_id).
    fn create_initial_snapshot(
        store: &SqliteStorage,
        path: &str,
        content: &[u8],
        source_type: SourceType,
    ) -> (FileNode, DeltaId, SnapshotId) {
        let file_node = FileNode::new(PathBuf::from(path), content);
        store.store_file_node(&file_node, content).unwrap();

        let diff = LineDiff::new(vec![]);
        let delta = Delta::new(file_node.clone(), diff, source_type);
        store.store_delta(&delta).unwrap();

        let snapshot = Snapshot::new_initial(file_node.clone(), delta.id);
        store.store_snapshot(&snapshot, content).unwrap();

        (file_node, delta.id, snapshot.id)
    }

    /// Create a child snapshot from a parent with a real text edit diff.
    fn create_edited_snapshot(
        store: &SqliteStorage,
        parent_id: SnapshotId,
        file_node: &FileNode,
        old_text: &str,
        new_text: &str,
        partition_type: &str,
    ) -> (DeltaId, SnapshotId) {
        let diff = diff_to_line_diff(old_text, new_text);
        let delta = Delta::new(file_node.clone(), diff, SourceType::Manual);
        store.store_delta(&delta).unwrap();

        let parent = store.get_snapshot(&parent_id).unwrap();
        let snapshot = Snapshot::from_parent(&parent, delta.id, partition_type.to_string());
        store.store_snapshot(&snapshot, &[]).unwrap();

        (delta.id, snapshot.id)
    }

    #[test]
    fn test_merge_to_staged() {
        let core = setup_core_repo();
        let backup_repo = BackupRepo::new_in_memory().unwrap();

        // Setup: initial file content "a\nb\nc\n"
        let (file_node, _delta_id, initial_id) =
            create_initial_snapshot(&core, "file.txt", b"a\nb\nc\n", SourceType::Manual);
        create_staged_partition(&core, initial_id);

        // Staged advances: edit "b" → "B" (diverges from backup branch)
        let (_staged_delta_id, staged_id) = create_edited_snapshot(
            &core,
            initial_id,
            &file_node,
            "a\nb\nc\n",
            "a\nB\nc\n",
            "staged",
        );
        let staged_partition = core.get_partition_by_name("staged").unwrap();
        core.update_pointer(&staged_partition.id, &staged_id).unwrap();

        // Backup branch: edit "c" → "C"
        let (_backup_delta_id, backup_snap_id) = create_edited_snapshot(
            &core,
            initial_id,
            &file_node,
            "a\nb\nc\n",
            "a\nb\nC\n",
            "manual",
        );

        let backup_id = backup_repo
            .backup_snapshot(&core, backup_snap_id, Some("merge-test".to_string()))
            .unwrap();

        // Restore: merge backup into staged
        let merged_id = backup_repo.merge_to_staged(&backup_id, &core).unwrap();

        let staged = core.get_partition_by_name("staged").unwrap();
        assert_eq!(staged.current_snapshot, merged_id);

        let merged_snapshot = core.get_snapshot(&merged_id).unwrap();
        assert!(merged_snapshot.parents.contains(&staged_id));
        assert!(merged_snapshot.parents.contains(&backup_snap_id));
        assert!(!merged_snapshot.parents.contains(&initial_id));

        // Verify content combines both edits
        let merged_base = core.get_file_content(merged_snapshot.file.path_str(), &merged_snapshot.file.base_hash).unwrap();
        let merged_deltas = core.get_deltas(&merged_snapshot.deltas).unwrap();
        let merged_content =
            apply_deltas(&String::from_utf8(merged_base).unwrap(), &merged_deltas).unwrap();
        assert_eq!(merged_content, "a\nB\nC");
    }

    #[test]
    fn test_restore_from_backup_reconstructs_content() {
        let core = setup_core_repo();
        let backup_repo = BackupRepo::new_in_memory().unwrap();

        // Setup: initial file content "line1\nline2\nline3\n"
        let (file_node, _delta_id, initial_id) =
            create_initial_snapshot(&core, "restore.txt", b"line1\nline2\nline3\n", SourceType::Manual);
        create_staged_partition(&core, initial_id);

        // Create a backup snapshot: edit "line2" → "modified"
        let (_delta_id, backup_id) = create_edited_snapshot(
            &core,
            initial_id,
            &file_node,
            "line1\nline2\nline3\n",
            "line1\nmodified\nline3\n",
            "manual",
        );

        let backup_id = backup_repo
            .backup_snapshot(&core, backup_id, Some("restore-test".to_string()))
            .unwrap();

        // Staged remains at initial, no divergence
        let merged_id = backup_repo.merge_to_staged(&backup_id, &core).unwrap();

        // Verify content matches the backed-up state
        let staged = core.get_partition_by_name("staged").unwrap();
        assert_eq!(staged.current_snapshot, merged_id);

        let merged_snapshot = core.get_snapshot(&merged_id).unwrap();
        let merged_base = core.get_file_content(merged_snapshot.file.path_str(), &merged_snapshot.file.base_hash).unwrap();
        let merged_deltas = core.get_deltas(&merged_snapshot.deltas).unwrap();
        let merged_content =
            apply_deltas(&String::from_utf8(merged_base).unwrap(), &merged_deltas).unwrap();
        assert_eq!(merged_content, "line1\nmodified\nline3");
    }

    #[test]
    fn test_physical_isolation() {
        let core = setup_core_repo();
        let backup_repo = BackupRepo::new_in_memory().unwrap();

        let snap_id =
            create_test_snapshot(&core, "isolated.txt", b"isolated", SourceType::Manual);
        backup_repo
            .backup_snapshot(&core, snap_id, None)
            .unwrap();

        assert!(core.snapshot_exists(&snap_id).unwrap());
        backup_repo.delete_backup(&snap_id).unwrap_err();
        assert!(core.snapshot_exists(&snap_id).unwrap());
    }

    #[test]
    fn test_backup_integrity_check() {
        let core = setup_core_repo();
        let backup_repo = BackupRepo::new_in_memory().unwrap();

        let snap_id = create_test_snapshot(&core, "check.txt", b"integrity", SourceType::Manual);
        let backup_id = backup_repo.backup_snapshot(&core, snap_id, None).unwrap();

        let backup = backup_repo.get_backup(&backup_id).unwrap();
        let recomputed = BackupSnapshot::new(
            backup.source_snapshot,
            backup.file.clone(),
            backup.deltas.clone(),
            backup.label.clone(),
        );
        assert_eq!(recomputed.id, backup.id);
    }
}