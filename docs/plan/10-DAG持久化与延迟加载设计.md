# DAG 持久化与延迟加载设计

## 1. 背景

当前 `CheckpointDag` 完全驻留内存，每次 `CheckpointRepo::load()` 时通过
`build_dag_from_checkpoints()` 从所有 checkpoint 的 `parents` 关系中重新构建。

在 **10K 级 checkpoint** 场景下，全量重建的性能开销极低（<1ms），参见
`docs/plan/09-存储层重构与接口加固.md` 中的性能分析。

但在以下场景中，需要 **DAG 持久化 + 延迟加载**：

| 场景 | checkpoint 规模 | 全量加载的瓶颈 |
|------|----------------|---------------|
| 长期运行的 CI/CD 流水线 | 100K+ | SQLite 读取所有行 + JSON 反序列化 |
| 多项目聚合仓库 | 1M+ | 内存占用（每个 checkpoint ~1KB） |
| 资源受限设备（边缘/嵌入式） | 50K+ | 内存压力 |

## 2. 设计目标

1. **DAG 持久化**：将 DAG 的 `nodes`（父子关系）和 `generation` 编号持久化到 SQLite
2. **延迟加载**：不读取全量 checkpoint，只从 DAG 表恢复图结构
3. **渐进式 checkpoint 加载**：按需从 SQLite 加载 checkpoint 实体（访问时才反序列化）
4. **向后兼容**：不破坏现有 API，`CheckpointRepo` 的 `storage` 字段依然可选
5. **一致性保证**：DAG 写操作与 checkpoint 写入在同一事务中，避免数据不一致

## 3. Schema 设计

### 3.1 表结构

```sql
-- DAG 边表：存储 parent→child 关系
CREATE TABLE IF NOT EXISTS dag_edges (
    parent_id   BLOB NOT NULL,
    child_id    BLOB NOT NULL,
    PRIMARY KEY (parent_id, child_id)
) WITHOUT ROWID;

-- Generation 编号表：存储每个节点的最大距离（根节点距离）
CREATE TABLE IF NOT EXISTS dag_generations (
    node_id     BLOB PRIMARY KEY,
    generation  INTEGER NOT NULL
) WITHOUT ROWID;

-- 索引：child_id 方向查询（反向遍历）
CREATE INDEX IF NOT EXISTS idx_dag_edges_child ON dag_edges(child_id);
```

### 3.2 说明

- `dag_edges` 使用 `WITHOUT ROWID` 优化主键查询性能
- `dag_generations` 的 generation 值与 `CheckpointDag` 中的定义一致：根节点为 0，每增加一个父级 +1
- 反向索引用于 `ancestors()` 和 `merge_base()` 中的 BFS 逆向遍历

## 4. 存储接口

在现有 `CheckpointPersist` trait 中增加 DAG 相关方法，或拆分为独立的 `DagStore` trait：

### 方案推荐：独立 `DagStore` trait

```rust
/// DAG 持久化存储接口
pub trait DagStore: Send + Sync {
    /// 批量存储 DAG 节点和边
    fn store_dag_batch(
        &self,
        nodes: &[(CheckpointId, u64)],                    // (node_id, generation)
        edges: &[(CheckpointId, CheckpointId)],            // (parent_id, child_id)
    ) -> StorageResult<()>;

    /// 添加单条边（用于 commit/merge）
    fn store_dag_edge(
        &self,
        parent_id: &CheckpointId,
        child_id: &CheckpointId,
        child_generation: u64,
    ) -> StorageResult<()>;

    /// 删除节点及关联边（用于 remove_checkpoint）
    fn delete_dag_node(&self, node_id: &CheckpointId) -> StorageResult<()>;

    /// 加载全量 DAG
    fn load_dag(&self) -> StorageResult<(HashMap<CheckpointId, HashSet<CheckpointId>>,
                                          HashMap<CheckpointId, u64>)>;

    /// 判断节点是否存在
    fn dag_has_node(&self, node_id: &CheckpointId) -> StorageResult<bool>;
}
```

`SqliteStorage` 的实现：

```rust
impl DagStore for SqliteStorage {
    fn store_dag_batch(...) {
        // 使用事务批量写入
        // INSERT OR IGNORE INTO dag_edges
        // INSERT OR REPLACE INTO dag_generations
    }

    fn load_dag(...) {
        // SELECT parent_id, child_id FROM dag_edges
        // SELECT node_id, generation FROM dag_generations
        // 重建 HashMap
    }
}
```

## 5. `CheckpointRepo` 延迟加载改造

### 5.1 延迟加载后的结构

```rust
pub struct CheckpointRepo {
    pub branches: Vec<Branch>,
    pub current_branch: usize,
    pub checkpoint_dag: CheckpointDag,     // 从持久化 DAG 恢复
    checkpoints: LazyCheckpointMap,         // 按需加载的 checkpoint 映射
    snapshots: HashMap<SnapshotId, Snapshot>,
    storage: Option<Box<dyn CheckpointPersist>>,
    pub time_index: TimeIndex,
    deleted_checkpoints: HashSet<CheckpointId>,
}

/// 延迟加载的 checkpoint 映射
struct LazyCheckpointMap {
    loaded: RwLock<HashMap<CheckpointId, Checkpoint>>,
    storage: Option<Box<dyn CheckpointPersist>>,
}

impl LazyCheckpointMap {
    fn get(&self, id: &CheckpointId) -> Result<Checkpoint> {
        if let Some(cp) = self.loaded.read().get(id) {
            return Ok(cp.clone());
        }
        // 从 storage 按需加载
        if let Some(storage) = &self.storage {
            let cp = storage.get_checkpoint(id)?;
            self.loaded.write().insert(*id, cp.clone());
            return Ok(cp);
        }
        Err(...)
    }
}
```

### 5.2 `load()` 方法改造

```rust
pub fn load(storage: Box<dyn CheckpointPersist>) -> Result<Self> {
    // 1. 加载分支（轻量）
    let branches = storage.list_branches()?;

    // 2. 从 DAG 表加载图结构（而非遍历所有 checkpoint）
    let checkpoint_dag = if let Ok(dag_data) = storage.load_dag() {
        dag_data  // 直接从持久化 DAG 恢复
    } else {
        // 兼容旧的未持久化 DAG：从 checkpoint 重建（暂不支持延迟加载）
        let checkpoints = load_all_checkpoints(&storage);
        build_dag_from_checkpoints(&checkpoints)
    };

    // 3. 加载 time_index（可以从 time_index 表，或从 checkpoint）
    let time_index = storage.load_time_index()?;

    // 4. 加载 current_branch 元数据
    let current_branch_name = storage.load_metadata("current_branch")?
        .unwrap_or_else(|| "main".to_string());

    // 5. 分支选择和 DAG 就绪，但不加载全量 checkpoint 内容
    //    后续通过 LazyCheckpointMap 按需加载

    Ok(CheckpointRepo {
        branches,
        current_branch,
        checkpoint_dag,
        checkpoints: LazyCheckpointMap::new(Some(storage)),
        snapshots: HashMap::new(),       // 延迟加载
        storage: Some(storage),
        time_index,
        deleted_checkpoints: HashSet::new(),
    })
}
```

### 5.3 关键方法适配

| 方法 | 当前行为 | 延迟加载适配 |
|------|---------|-------------|
| `get_checkpoint(id)` | `self.checkpoints.get(id)` | `self.checkpoints.get(id)` → 按需加载 |
| `commit(...)` | 插入 `self.checkpoints` | 插入 + 写 DAG 边 + 更新 generation |
| `log(count)` | 从 `self.checkpoints` BFS | 需从 storage 按需加载沿途 checkpoint |
| `get_ancestry_chain(id)` | BFS traverse + HashMap get | 按需加载沿途 checkpoint |
| `sync_all()` | 遍历 `self.checkpoints.values()` | 只需 sync 已加载的 + 确保 DAG edges 已持久化 |

## 6. 事务一致性

DAG 持久化必须与 checkpoint 写入处于同一 SQLite 事务，以避免以下不一致：

```
Time 故障点                    SQLite 状态
──────────────────────────────────────────────
checkpoint INSERT OK           checkpoint 存在，但 dag_edges 缺失
     ↓ CRASH ✗
dag_edges INSERT NOT executed
```

### 解决方案

所有写操作使用 `with_transaction` 包装：

```rust
pub fn commit(&mut self, ...) -> Result<CheckpointId> {
    // ...
    if let Some(storage) = &self.storage {
        storage.with_transaction(|tx| {
            tx.store_checkpoint(&cp)?;
            tx.store_dag_edge(current_head, cp_id, gen)?;
            tx.update_branch_head(...)?;
            Ok(())
        })?;
    }
    // ...
}
```

现有 `AtomicOps` trait 已提供 `with_atomic` 方法，但当前实现在 `StateMachine` 层面有
Mutex 死锁问题（已知问题，见 `09-存储层重构与接口加固.md`）。DAG 持久化应在该问题
修复后接入。

## 7. 实施路径

### 阶段一：接口定义（1-2 天）

1. 在 `storage/repository.rs` 中定义 `DagStore` trait
2. 实现 `impl DagStore for SqliteStorage`
3. 添加迁移 SQL（`CREATE TABLE IF NOT EXISTS dag_edges/generations`）

### 阶段二：写路径集成（2-3 天）

1. `commit()` 中在 `storage.store_checkpoint()` 后调用 `storage.store_dag_edge()`
2. `merge_branches()` 同理
3. `remove_checkpoint()` 中调用 `storage.delete_dag_node()`
4. 所有操作使用事务包装

### 阶段三：读路径集成（2-3 天）

1. `load()` 中优先从 `load_dag()` 恢复 DAG
2. 实现 `LazyCheckpointMap` 结构
3. 迁移现有 `self.checkpoints` 的所有直接访问到 `LazyCheckpointMap`

### 阶段四：回退兼容与测试（1-2 天）

1. 如果 `dag_edges` 表为空（旧数据库），回退到全量重建
2. 集成测试覆盖：纯内存模式、DAG 持久化模式、旧库迁移
3. 基准测试：10K/100K/1M checkpoint 下的 load 时间

## 8. 风险与注意事项

1. **事务死锁**：`AtomicOps` 的 Mutex 问题（`StateMachine::with_transaction`）
   必须先修复，否则 DAG 写和 checkpoint 写不在同一事务中
2. **内存占用**：即使延迟加载，`branches`、`time_index`、`checkpoint_dag` 仍需全量
   在内存中。`time_index` 约 (checkpoint_count × 40 bytes)，`checkpoint_dag` 约
   (checkpoint_count × 100 bytes)
3. **增量读取**：`log()` 和 `get_ancestry_chain()` 需要 BFS 遍历时按需读取沿途
   checkpoint，可能多次访问 SQLite。可通过预读（prefetch）批量优化
4. **`sync_all()` 语义变化**：当前 `sync_all()` 遍历所有 checkpoint 写入 storage，
   延迟加载后改为只写 `dirty` 的 checkpoint + 持久化 DAG 边

## 9. 不采用方案

### 方案 X：Checkpoint 表增加 `generation` 列

直接在 `checkpoints` 表增加 `generation INTEGER` 列，替代独立的
`dag_generations` 表。优点是没有额外表，缺点是：
- 每个 checkpoint 多一个列（nullable）
- 批量查询 generation 需要全表扫描
- 与原有 `parents BLOB` 配合不够直观

### 方案 Y：Redis/外部缓存

DAG 数据量小（每条边 ~64 bytes），SQLite 完全胜任。引入外部缓存
（Redis/Memcached）增加系统复杂度，不必要。

---

*文档版本：v1.0*
*关联文档：[09-存储层重构与接口加固.md](./09-存储层重构与接口加固.md)*