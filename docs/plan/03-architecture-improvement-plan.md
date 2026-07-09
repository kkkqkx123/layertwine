# 3. 架构改进计划

## 概述

基于对 Layertwine 当前代码库（v0.1.0）的全面架构审查，本文档记录需要改进的方面及具体实施方案。按优先级从高到低排列。

---

## 3.1 同步存储阻塞异步运行时

### 问题描述

存储层（`src/storage/sqlite/`）全部是同步 API，但 HTTP/gRPC 模式运行在 tokio 异步运行时上。同步的 SQLite 操作会阻塞 tokio 的工作线程，导致：

- 高并发下请求延迟增加
- 吞吐量受限
- 潜在的线程池饥饿

### 改进方案

**方案 A（推荐）：使用 `spawn_blocking` 包装**

在 `ApiService` 层对存储操作统一使用 `tokio::task::spawn_blocking` 包装，将同步操作委托给 tokio 的阻塞线程池。

```rust
// 在 ApiService 中新增辅助方法
async fn with_storage<T, F>(&self, f: F) -> ApiResult<T>
where
    F: FnOnce(&SqliteStorage) -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    let storage = self.storage.clone();
    tokio::task::spawn_blocking(move || f(&storage))
        .await
        .map_err(|e| ApiError::internal(format!("blocking task failed: {}", e)))?
}
```

**方案 B（长期）：迁移到 `tokio-rusqlite`**

将 SQLite 存储层替换为 `tokio-rusqlite`，该库内部使用 `spawn_blocking` 并提供了 async 接口。

### 涉及文件

- `src/storage/sqlite/connection.rs` — 数据库连接管理
- `src/api/service.rs` — ApiService 中所有存储操作

### 影响范围

- 中。接口签名不变，仅内部实现变更。
- 需要修改 `ApiService` 中所有直接调用 `self.storage.*` 的方法。

---

## 3.2 ApiService 锁机制不友好

### 问题描述

`ApiService` 使用 `std::sync::RwLock<CheckpointRepo>` 保护检查点仓库：

```rust
pub struct ApiService {
    checkpoint_repo: Arc<std::sync::RwLock<CheckpointRepo>>,
    // ...
}
```

在 async 上下文中：
- `.write()` / `.read()` 是同步方法，会阻塞当前线程
- 不能在持有锁时跨 `.await` 点，易引发死锁

### 改进方案

**替换为 `tokio::sync::RwLock`**

```rust
use tokio::sync::RwLock;

pub struct ApiService {
    checkpoint_repo: Arc<RwLock<CheckpointRepo>>,
    // ...
}
```

同时将 `ApiService` 方法改为 async：

```rust
impl ApiService {
    pub async fn commit(&self, req: CommitRequest) -> ApiResult<CommitResponse> {
        let mut repo = self.checkpoint_repo.write().await;
        // ...
    }
}
```

### 涉及文件

- `src/api/service.rs` — `ApiService` 结构体定义及所有方法

### 影响范围

- 中。需要将 `ApiService` 的所有方法改为 async，并同步修改 HTTP/gRPC 路由层。

---

## 3.3 文档与实现不一致

### 问题描述

| 文档位置 | 声称 | 实际实现 |
|---------|------|---------|
| `docs/architecture/07-Rust实现方案.md` | 使用 `rkyv` 零拷贝序列化 | 使用 `serde_json`，Cargo.toml 中无 `rkyv` |
| `docs/architecture/01-架构总览.md` | 4 层状态机 | 代码已演进到 6 层（integrated 层已分离） |
| `docs/architecture/07-Rust实现方案.md` | `async fn in trait` 存储接口 | 存储层全部是同步 trait |

### 改进方案

1. **同步文档与实际代码**：将所有 .md 文档更新为反映当前代码状态
2. **移除 `rkyv` 相关描述**：或增加 `rkyv` 作为后续优化项
3. **更新状态机文档**：包含 `integrated` 层，移除已废弃的 `unified` 层
4. **增加文档维护流程**：在 PR 中加入文档同步检查

### 涉及文件

- `docs/architecture/01-架构总览.md`
- `docs/architecture/02-核心数据模型.md`
- `docs/architecture/03-分层状态机.md`
- `docs/architecture/07-Rust实现方案.md`

### 影响范围

- 小。仅文档变更，无代码修改。

---

## 3.4 存储层 Trait 过于碎片化

### 问题描述

当前存储层 trait 设计：

```rust
pub trait Repository:
    SnapshotStore + DeltaStore + PartitionStore + FileNodeStore
    + CheckpointPersist + LayerStore + AtomicOps
{}
```

任何需要完整存储功能的组件都需要声明 7 个 trait bound。`SqliteStorage` 需要实现所有子 trait，每个 trait 只有 3-5 个方法。

### 改进方案

**合并为更少的更大的 trait**：

```rust
/// 存储读取接口
pub trait StorageRead: Send + Sync {
    fn get_snapshot(&self, id: &SnapshotId) -> StorageResult<Snapshot>;
    fn get_delta(&self, id: &DeltaId) -> StorageResult<Delta>;
    fn get_partition(&self, id: &PartitionId) -> StorageResult<Partition>;
    fn get_file_content(&self, path: &str, hash: &[u8; 32]) -> StorageResult<Vec<u8>>;
    fn get_checkpoint(&self, id: &CheckpointId) -> StorageResult<Checkpoint>;
    fn get_branch(&self, name: &str) -> StorageResult<Branch>;
    fn get_layer(&self, layer_type: &LayerType) -> StorageResult<Layer>;
    fn list_partitions(&self) -> StorageResult<Vec<Partition>>;
    fn list_branches(&self) -> StorageResult<Vec<Branch>>;
    // ... 其他读取方法
}

/// 存储写入接口
pub trait StorageWrite: Send + Sync {
    fn store_snapshot(&self, snapshot: &Snapshot, content: &[u8]) -> StorageResult<()>;
    fn store_delta(&self, delta: &Delta) -> StorageResult<()>;
    fn create_partition(&self, partition: &Partition) -> StorageResult<()>;
    fn update_pointer(&self, pid: &PartitionId, sid: &SnapshotId) -> StorageResult<()>;
    fn store_checkpoint(&self, checkpoint: &Checkpoint) -> StorageResult<()>;
    fn store_branch(&self, branch: &Branch) -> StorageResult<()>;
    fn store_layer(&self, layer: &Layer) -> StorageResult<()>;
    // ... 其他写入方法
}

/// 完整存储接口
pub trait Storage: StorageRead + StorageWrite + AtomicOps {}
```

### 涉及文件

- `src/storage/repository.rs` — 所有 trait 定义
- 所有引用子 trait 的模块（`src/layered/`, `src/checkpoint/`, `src/api/`）

### 影响范围

- 大。需要修改所有 `where S: SnapshotStore + DeltaStore + ...` 的约束。
- 建议作为 v0.2.0 的里程碑进行重构。

---

## 3.5 错误处理层次过多

### 问题描述

当前存在两个独立的错误枚举：

```rust
pub enum LayertwineError {
    Storage(StorageError),   // 15+ 变体
    Engine(String),
    StateMachine(String),
    Checkpoint(String),
    Restore(String),
    Transaction(String),
    Integrity(String),
    GitSync(String),
    Gc(String),
    Cli { context, suggestion },
    General(String),
    Serialization(String),
    NotFound(String),
}

pub enum StorageError {
    NotFound(String),
    ConstraintViolation(String),
    Serialization(String),
    Database(rusqlite::Error),
    Migration(String),
    Io(std::io::Error),
}
```

### 改进方案

1. **合并 `StorageError` 到 `LayertwineError`** — 减少一层包裹
2. **减少 `LayertwineError` 变体** — 将极少独立使用的变体合并为 `General`
3. **统一错误代码和格式化** — 所有错误都提供 `exit_code()` 和 `format_cli()`

```rust
pub enum LayertwineError {
    // 仅保留真正需要区分处理的错误
    Database(rusqlite::Error),
    NotFound(String),
    ConstraintViolation(String),
    StateMachine(String),
    Serialization(String),
    Cli { context: String, suggestion: Option<String> },
    Other(String), // 替代 Engine/Checkpoint/Restore/Transaction/Integrity/GitSync/Gc
}
```

### 涉及文件

- `src/error.rs` — 错误类型定义
- 所有 `impl From<...>` 转换

### 影响范围

- 中。需要修改所有错误匹配的代码，但可以逐步迁移。

---

## 3.6 遗留后向兼容代码

### 问题描述

`ApiService` 中存在多个标注为 `kept for backward compatibility` 的方法：

- `merge_to_unified()` — 已废弃的 unified 层合并
- `merge_to_staged()` — 与 `integrated → staged` 流程重复

此外 `PartitionType` 和 `LayerType` 中仍保留 `Unified` 变体，但实际流程已不再使用。

### 改进方案

1. **移除 `merge_to_unified()` 和 `merge_to_staged()`** — 统一使用 `approve()` 流程
2. **移除 `PartitionType::Unified` 和 `LayerType::Unified`** — 清理枚举
3. **更新 CLI 和 API 文档** — 移除对应的命令说明

### 涉及文件

- `src/api/service.rs` — 废弃方法
- `src/core/types.rs` — `PartitionType` / `LayerType` 枚举
- `src/cli/commands.rs` — CLI 命令定义
- `src/api/types.rs` — API 请求/响应类型

### 影响范围

- 中。需要确认是否有外部调用方依赖这些接口。

---

## 3.7 快照类型混合关注点

### 问题描述

`Snapshot` 结构体包含了多个不同关注点的字段：

```rust
pub struct Snapshot {
    pub content: Option<SnapshotContent>,  // 数据内容（文件/JSON/结构化）
    pub compression: SnapshotCompression,  // 压缩方式
    pub has_conflicts: bool,               // 冲突标记
    // ... 核心字段
}
```

压缩逻辑在 `Snapshot::compress_content()` 和 `Snapshot::decompress_content()` 中，耦合了数据模型与存储细节。

### 改进方案

**将压缩移到存储层**：

```rust
// 存储层处理压缩，Snapshot 保持纯净
impl SqliteStorage {
    fn store_snapshot_with_compression(&self, snapshot: &Snapshot) -> StorageResult<()> {
        let compressed = match &snapshot.content {
            Some(content) => {
                let bytes = content.to_bytes();
                if bytes.len() > COMPRESSION_THRESHOLD {
                    Some(zstd::encode_all(bytes.as_slice(), 3)?)
                } else {
                    None
                }
            }
            None => None,
        };
        // 存储 compressed 或原始数据
    }
}
```

移除 `SnapshotCompression` 枚举和 `Snapshot` 上的 `compress_content`/`decompress_content` 方法。

### 涉及文件

- `src/core/snapshot.rs` — 移除压缩相关字段和方法
- `src/storage/sqlite/snapshot.rs` — 在存储层实现压缩

### 影响范围

- 小。接口兼容性好，对外部无影响。

---

## 3.8 数据流耦合：transition.rs 为上帝调度模块

### 问题描述

`src/layered/transition.rs` 同时引用了几乎所有其他模块：

```rust
use crate::layered::manual::*;
use crate::layered::agent::*;
use crate::layered::approval::*;
use crate::layered::staged::*;
use crate::layered::integrated::*;
use crate::backup::backup_repo::BackupRepo;
```

该模块充当了"中央调度器"，随着层数增加，职责会越来越膨胀。

### 改进方案

**将流转逻辑分散到各层模块中**：

- 每个层模块（`manual.rs`, `agent.rs` 等）各自定义自己的输入/输出流转
- `transition.rs` 仅保留铁律检查（`check_forward_valid` / `check_rollback_valid`）和路由分发

```rust
// manual.rs 中定义
impl ManualLayer {
    pub fn merge_to_staged(&self, storage: &S) -> Result<SnapshotId> { ... }
}

// agent.rs 中定义
impl AgentLayer {
    pub fn submit_to_approval(&self, storage: &S) -> Result<SnapshotId> { ... }
}

// transition.rs 仅保留
pub fn execute_forward(storage, transition, params) -> Result<SnapshotId> {
    match transition {
        ManualToStaged => ManualLayer::merge_to_staged(storage),
        AgentToApproval => AgentLayer::submit_to_approval(storage, ...),
        // ...
    }
}
```

### 涉及文件

- `src/layered/transition.rs` — 拆分调度逻辑
- `src/layered/manual.rs` — 新增 `merge_to_staged` 方法
- `src/layered/agent.rs` — 新增 `submit_to_approval` 方法
- 其他层模块

### 影响范围

- 中。需要重构方法调用路径，但业务逻辑不变。

---

## 3.9 测试工具方法区分不明确

### 问题描述

`src/test_utils.rs` 中：

```rust
pub fn setup_storage() -> SqliteStorage { ... }
pub fn setup_storage_full() -> SqliteStorage { ... }
```

两者区别无文档说明，且测试代码中存在大量重复模式（创建初始 snapshot → 创建 partition → 编辑 → 验证）。

### 改进方案

1. **增加文档注释**说明两个函数的区别
2. **提取公共测试模式**为辅助函数

```rust
/// 创建内存中的 SQLite 存储，只运行必需的表迁移。
/// 适用于大部分单元测试。
pub fn setup_storage() -> SqliteStorage { ... }

/// 创建内存中的 SQLite 存储，运行所有表迁移。
/// 适用于需要完整 schema 的集成测试。
pub fn setup_storage_full() -> SqliteStorage { ... }

/// 测试辅助：创建初始 snapshot 并设置 manual 和 staged 分区
pub fn setup_initial_workflow(storage: &SqliteStorage) -> (SnapshotId, Partition, Partition) {
    let initial_id = create_initial_snapshot(storage, "base\n", SourceType::Manual);
    let manual = ensure_manual_partition(storage, initial_id).unwrap();
    let staged = ensure_staged_partition(storage, initial_id).unwrap();
    (initial_id, manual, staged)
}
```

### 涉及文件

- `src/test_utils.rs` — 添加文档和辅助函数
- 各测试文件 — 使用新的辅助函数减少重复

### 影响范围

- 小。仅测试代码变更。

---

## 3.10 配置加载方式可简化

### 问题描述

当前配置加载方式：

```rust
// 自定义递归合并
fn merge_values(base: &mut toml::Value, overlay: toml::Value) { ... }

pub fn load_with_priority(db_dir: &Path) -> Result<Self> {
    let defaults_str = toml::to_string(&LayertwineConfig::default())?;
    let mut base: toml::Value = toml::from_str(&defaults_str)?;
    // 遍历路径，加载并 merge
    for path in Self::config_paths(db_dir) {
        // ...
        merge_values(&mut base, overlay);
    }
    // 再序列化回去
    let out = toml::to_string_pretty(&base)?;
    let config: LayertwineConfig = toml::from_str(&out)?;
}
```

### 改进方案

**使用 `figment` crate 简化配置加载**：

```rust
use figment::{Figment, providers::{Toml, Serialized, Env}};

pub fn load_with_priority(db_dir: &Path) -> Result<Self> {
    let mut figment = Figment::from(Serialized::defaults(LayertwineConfig::default()));

    for path in Self::config_paths(db_dir) {
        if path.exists() {
            figment = figment.merge(Toml::file(&path));
        }
    }

    figment = figment.merge(Env::prefixed("LAYERTWINE_"));

    figment.extract()
        .map_err(|e| LayertwineError::General(format!("config error: {}", e)))
}
```

这样也自然地支持了环境变量覆盖（如 `LAYERTWINE_DB_PATH`、`LAYERTWINE_MAINTENANCE_FREELIST_THRESHOLD`）。

### 涉及文件

- `Cargo.toml` — 新增 `figment` 依赖
- `src/config.rs` — 替换 `merge_values` 和 `load_with_priority`

### 影响范围

- 小。配置接口不变，仅内部实现变更。

---

## 实施路线图

| 优先级 | 改进项 | 预估工作量 | 建议版本 |
|-------|--------|-----------|---------|
| P0 | 3.1 异步阻塞 | 2-3 天 | v0.1.1 |
| P0 | 3.2 锁机制 | 1-2 天 | v0.1.1 |
| P1 | 3.3 文档同步 | 0.5 天 | v0.1.2 |
| P1 | 3.6 遗留代码清理 | 1 天 | v0.1.2 |
| P1 | 3.9 测试工具改进 | 0.5 天 | v0.1.2 |
| P2 | 3.5 错误处理简化 | 2 天 | v0.2.0 |
| P2 | 3.7 快照关注点分离 | 1 天 | v0.2.0 |
| P2 | 3.8 拆分 transition.rs | 1-2 天 | v0.2.0 |
| P3 | 3.4 存储 trait 合并 | 3-4 天 | v0.3.0 |
| P3 | 3.10 配置加载简化 | 0.5 天 | v0.3.0 |

### 说明

- **P0**：影响系统正确性和稳定性，建议立即修复
- **P1**：影响开发效率和维护成本，建议短期修复
- **P2**：架构优化，建议中期规划
- **P3**：代码质量提升，建议长期规划