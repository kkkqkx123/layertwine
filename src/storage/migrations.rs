/// 数据库迁移 — 创建所有表
///
/// 参考 architecture/07-Rust实现方案.md §7.6 的建表 SQL
pub const MIGRATION_SQL: &str = "
-- 文件节点表（不可变，INSERT ONLY）
CREATE TABLE IF NOT EXISTS file_nodes (
    file_path    TEXT NOT NULL,
    base_hash    BLOB NOT NULL,
    content      BLOB NOT NULL,
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (file_path, base_hash)
);

-- Delta 表（不可变，INSERT ONLY）
CREATE TABLE IF NOT EXISTS deltas (
    id           BLOB PRIMARY KEY,
    file_path    TEXT NOT NULL,
    file_hash    BLOB NOT NULL,
    diff         BLOB NOT NULL,
    source       TEXT NOT NULL,
    source_data  TEXT,
    timestamp    INTEGER NOT NULL,
    created_at   INTEGER NOT NULL
);

-- Snapshot 表（不可变，INSERT ONLY）
CREATE TABLE IF NOT EXISTS snapshots (
    id              BLOB PRIMARY KEY,
    file_path       TEXT NOT NULL,
    file_hash       BLOB NOT NULL,
    deltas          BLOB NOT NULL,
    parents         BLOB NOT NULL,
    partition_type  TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);

-- 分区表
CREATE TABLE IF NOT EXISTS partitions (
    id              BLOB PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    current_snapshot BLOB NOT NULL,
    partition_type  TEXT NOT NULL,
    partition_data  TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- 分区历史快照关联表
CREATE TABLE IF NOT EXISTS partition_history (
    partition_id    BLOB NOT NULL,
    snapshot_id     BLOB NOT NULL,
    seq             INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (partition_id, seq),
    FOREIGN KEY (partition_id) REFERENCES partitions(id)
);

-- 分层表
CREATE TABLE IF NOT EXISTS layers (
    layer_type      TEXT PRIMARY KEY,
    partition_ids   BLOB NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- 索引
CREATE INDEX IF NOT EXISTS idx_snapshots_file ON snapshots(file_path);
CREATE INDEX IF NOT EXISTS idx_snapshots_partition ON snapshots(partition_type);
CREATE INDEX IF NOT EXISTS idx_deltas_file ON deltas(file_path);
CREATE INDEX IF NOT EXISTS idx_partition_history_snapshot ON partition_history(snapshot_id);
";

/// 检查点相关表（在 P4 中使用）
pub const MIGRATION_CHECKPOINT_SQL: &str = "
CREATE TABLE IF NOT EXISTS checkpoints (
    id              BLOB PRIMARY KEY,
    parents         BLOB NOT NULL,
    snapshot_id     BLOB NOT NULL,
    author          TEXT NOT NULL,
    message         TEXT NOT NULL,
    git_anchor      TEXT,
    created_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS branches (
    name            TEXT PRIMARY KEY,
    head            BLOB NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
";

/// 初始化数据库，应用所有迁移
pub fn initialize_database(conn: &rusqlite::Connection) -> Result<(), crate::StorageError> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    conn.execute_batch(MIGRATION_SQL)?;
    Ok(())
}

/// 应用完整迁移（含检查点相关表）
pub fn initialize_full(conn: &rusqlite::Connection) -> Result<(), crate::StorageError> {
    initialize_database(conn)?;
    conn.execute_batch(MIGRATION_CHECKPOINT_SQL)?;
    Ok(())
}
