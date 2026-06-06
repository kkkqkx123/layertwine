在一个需要长期运行且频繁进行更新、删除操作的项目中使用 SQLite 作为持久化存储，**文件持续膨胀**与**存储利用率下降**的核心原因在于：SQLite 默认不会将删除或更新释放的页面归还给操作系统，而是将其标记为空闲页，加入“空闲链表”（freelist）供后续重用。但由于碎片化或重用效率不高，空闲页可能大量堆积，导致物理文件只增不减。

要解决这一问题，需要结合**数据库内部机制**、**定期维护操作**以及**WAL 模式管理**。以下是系统性的应对策略及对应的 SQLite API（C 接口和 SQL 命令）。

---

### 一、根本对策：让数据库能主动回收空间

#### 1. 启用自动真空（auto_vacuum）

`auto_vacuum` 能让 SQLite 在提交事务后自动将空闲页移到文件末尾并截断文件。

- **模式选择**：
  - `FULL`：每次提交后立刻移动空闲页并截断，回收最积极，但写性能开销较大。
  - `INCREMENTAL`：不自动截断，只将空闲页信息存入“指针映射页”，需要手动调用 `incremental_vacuum` 逐步截断，适合不允许长时间锁定的场景。
- **限制**：只能在数据库**创建时**设置，已有数据的库需先执行 `VACUUM` 再通过 `PRAGMA` 修改。

**SQL 命令**

```sql
-- 创建时设定（或 VACUUM 后设置）
PRAGMA auto_vacuum = FULL;       -- 或 INCREMENTAL
```

**C API**  
通过 `sqlite3_exec()` 执行上述 PRAGMA 即可。

---

#### 2. 定期手动 VACUUM（完整重建文件）

执行 `VACUUM` 会重建整个数据库文件，清理所有碎片，回收全部空闲空间，同时重排记录使其更紧凑。代价是**需要短暂锁定数据库**，且会生成临时文件（与原库大小相当）。

**SQL 命令**

```sql
VACUUM;
-- 或将重建结果输出到新文件（SQLite 3.27+）
VACUUM INTO 'new_database.db';
```

**C API**

```c
sqlite3_exec(db, "VACUUM", NULL, NULL, NULL);
// 或者使用备份 API 实现在线“真空”效果（见后）
```

---

#### 3. 增量 VACUUM（INCREMENTAL 模式专用）

若已开启 `auto_vacuum = INCREMENTAL`，可在业务低峰期多次调用，每次回收指定数量的页面，避免长时间锁库。

**SQL 命令**

```sql
PRAGMA incremental_vacuum(100);   -- 一次回收 100 个空闲页
-- 不指定页数则回收全部可能的页
PRAGMA incremental_vacuum;
```

**C API**  
同样通过 `sqlite3_exec()` 执行 PRAGMA。

---

#### 4. 监控空闲页面，指导维护时机

定期检查空闲页数量，超过阈值时触发回收操作。

**SQL 命令**

```sql
PRAGMA freelist_count;   -- 当前空闲页数
PRAGMA page_count;       -- 总页数（含空闲页）
```

可据此计算碎片率：`freelist_count / page_count`。

---

### 二、WAL 模式下的额外管理（防止 WAL 文件无限增大）

若使用 WAL（Write-Ahead Logging）模式提升并发，`-wal` 文件会随着写入不断增长，必须定期 **checkpoint** 将其内容合并回主数据库，并截断 WAL 文件。

#### 1. 手动 WAL Checkpoint

```sql
PRAGMA wal_checkpoint(TRUNCATE);  -- 强制做一次 checkpoint 并截断 WAL 文件
-- 或 PASSIVE / FULL / RESTART 等模式
```

#### 2. 自动 Checkpoint 设置

```c
// 设置自动 checkpoint 阈值（WAL 达到 N 个页面时自动执行）
sqlite3_wal_autocheckpoint(db, 1000);  // C API
```

对应的 SQL：

```sql
PRAGMA wal_autocheckpoint = 1000;   -- 每 1000 页自动 checkpoint
```

#### 3. 限制 WAL 文件大小

```sql
PRAGMA journal_size_limit = 67108864;   -- 限制 WAL 最大为 64MB
```

达到限制后，SQLite 会尝试自动 checkpoint 并截断。

**WAL 相关 C API**

```c
int sqlite3_wal_checkpoint_v2(
  sqlite3 *db, const char *zDb,
  int eMode,            // SQLITE_CHECKPOINT_PASSIVE/FULL/RESTART/TRUNCATE
  int *pnLog, int *pnCkpt
);
```

### 三、辅助优化操作

- **重建索引**：若索引碎片严重，可 `REINDEX;` 但 `VACUUM` 已包含此操作。
- **合理页面大小**：创建库时根据典型记录长度设置 `PRAGMA page_size`（如 4096 或 8192），减少内部碎片，提高空间利用率。
- **避免频繁短事务**：合并更新操作，减少空闲页在 freelist 中的积累碎片。

---

### 五、长期运行项目的推荐维护策略组合

| 场景               | 推荐措施                                                                  |
| ------------------ | ------------------------------------------------------------------------- |
| 可接受定期维护窗口 | 低峰期执行 `VACUUM` / `VACUUM INTO` + 监控 `freelist_count`               |
| 几乎零停机时间     | 开启 `auto_vacuum=INCREMENTAL`，定时执行 `PRAGMA incremental_vacuum(N)`   |
| 读写并发高（WAL）  | 设置 `journal_size_limit` 和 `wal_autocheckpoint`，定期 `TRUNCATE` 检查点 |
| 大库、高可用       | 备份 API（`sqlite3_backup_*`）生成紧凑副本，在线替换                      |

所有操作均可通过 **SQL 命令** 或 **C API** 实现，选用时务必结合业务容忍度和文件增长速率动态调整。
