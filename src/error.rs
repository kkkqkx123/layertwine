use thiserror::Error;

/// Stratum 全局错误类型
#[derive(Error, Debug)]
pub enum StratumError {
    // ── 存储层错误 ──
    #[error("存储层错误: {0}")]
    Storage(#[from] StorageError),

    // ── 引擎层错误 ──
    #[error("引擎错误: {0}")]
    Engine(String),

    // ── 状态机错误 ──
    #[error("状态机错误: {0}")]
    StateMachine(String),

    // ── 检查点错误 ──
    #[error("检查点错误: {0}")]
    Checkpoint(String),

    // ── Git 同步错误 ──
    #[error("Git 同步错误: {0}")]
    GitSync(String),

    // ── GC 错误 ──
    #[error("GC 错误: {0}")]
    Gc(String),

    // ── CLI 错误 ──
    #[error("CLI 错误: {0}")]
    Cli(String),

    // ── 通用错误 ──
    #[error("{0}")]
    General(String),

    // ── 序列化错误 ──
    #[error("序列化错误: {0}")]
    Serialization(String),

    // ── 未找到 ──
    #[error("未找到: {0}")]
    NotFound(String),
}

/// 存储层专用错误
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("记录未找到: {0}")]
    NotFound(String),

    #[error("约束违反: {0}")]
    ConstraintViolation(String),

    #[error("序列化错误: {0}")]
    Serialization(String),

    #[error("数据库错误: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("迁移错误: {0}")]
    Migration(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

/// 便捷 Result 别名
pub type Result<T> = std::result::Result<T, StratumError>;

/// 存储层 Result 别名
pub type StorageResult<T> = std::result::Result<T, StorageError>;

impl From<serde_json::Error> for StratumError {
    fn from(e: serde_json::Error) -> Self {
        StratumError::Serialization(e.to_string())
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::Serialization(e.to_string())
    }
}
