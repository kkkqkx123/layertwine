# Stratum 功能增强规范

> 目标：完整支持Agent/Graph执行状态的版本管理  
> 范围：Rust端 `crates/stratum` 功能增强  
> 版本：v1.0  
> 生效：作为final architecture的基础

---

## 一、核心功能需求

### 1.1 快照能力扩展

#### 当前状态
- Stratum仅支持**文件内容快照**（文件系统级）
- `baseline_snapshots: Vec<SnapshotId>` 存储文件快照

#### 需求
Stratum需支持**元数据快照**（执行状态级），用于存储：
- Agent执行状态（messages, iterations, variables）
- Graph执行状态（workflow state, node results）
- 任意JSON对象（作为通用快照机制）

#### 设计方案

```rust
// crates/stratum/src/core/types.rs - 扩展

/// 快照类型（支持多种内容）
pub enum SnapshotContent {
  /// 文件内容
  FileContent(Vec<u8>),
  
  /// JSON元数据（用于Agent/Graph状态）
  JsonMetadata(serde_json::Value),
  
  /// 结构化数据（未来可扩展为其他格式）
  Structured(Vec<u8>),
}

impl SnapshotContent {
  pub fn to_bytes(&self) -> Vec<u8> {
    match self {
      Self::FileContent(bytes) => bytes.clone(),
      Self::JsonMetadata(value) => serde_json::to_vec(value).unwrap(),
      Self::Structured(bytes) => bytes.clone(),
    }
  }
  
  pub fn from_bytes(source: &str, bytes: Vec<u8>) -> Result<Self> {
    // 根据source判断类型
    match source {
      _ if source.starts_with("file://") => Ok(Self::FileContent(bytes)),
      _ if source.starts_with("agent://") | source.starts_with("graph://") => {
        Ok(Self::JsonMetadata(serde_json::from_slice(&bytes)?))
      },
      _ => Ok(Self::Structured(bytes)),
    }
  }
}

/// 扩展Snapshot元数据
pub struct Snapshot {
  pub id: SnapshotId,
  pub content: SnapshotContent,          // 新增：支持多种内容
  pub source: String,                     // 新增：source identifier
  pub created_at: i64,
  pub size: usize,
}

/// 扩展Checkpoint以支持源追踪
pub struct Checkpoint {
  pub id: CheckpointId,
  pub parents: Vec<CheckpointId>,
  pub baseline_snapshots: Vec<SnapshotId>,
  
  // 新增：快照源信息（便于查询）
  pub snapshot_sources: HashMap<SnapshotId, String>,
  
  pub metadata: CheckpointMetadata,
  pub created_at: i64,
}
```

### 1.2 查询和恢复能力

#### 需求清单

| 功能 | 说明 | 优先级 |
|-----|------|--------|
| **restore_full** | 完整恢复：返回checkpoint及所有关联的快照内容 | P0 |
| **restore_selective** | 选择性恢复：按source/pattern筛选快照 | P0 |
| **restore_by_time** | 时间查询恢复：获取指定时间点的状态 | P1 |
| **diff_checkpoints** | 两个checkpoint之间的差异 | P1 |
| **list_snapshots** | 列出checkpoint的所有快照及其类型 | P0 |
| **validate_integrity** | 检查checkpoint数据完整性 | P1 |

#### API规范

```rust
// crates/stratum/src/checkpoint/restore.rs - 新增模块

pub struct RestoreRequest {
  /// 目标checkpoint ID
  pub checkpoint_id: CheckpointId,
  
  /// 选择性恢复的source filter（支持glob）
  /// 例如：["agent://", "graph://state", "file://src/**"]
  pub source_filter: Option<Vec<String>>,
  
  /// 时间范围（可选）
  pub time_range: Option<(i64, i64)>,
}

pub struct RestoreResponse {
  /// Checkpoint信息
  pub checkpoint: Checkpoint,
  
  /// 快照列表及其内容
  pub snapshots: Vec<(SnapshotId, SnapshotContent, String)>,
  
  /// 祖先链（用于delta reconstruction）
  pub ancestry: Vec<CheckpointId>,
}

impl CheckpointRepo {
  /// 完整恢复：返回指定checkpoint的所有快照及内容
  pub fn restore_full(
    &self,
    cp_id: &CheckpointId,
  ) -> Result<RestoreResponse> {
    let cp = self.get_checkpoint(cp_id)?;
    let snapshots = self.load_all_snapshot_contents(&cp.baseline_snapshots)?;
    let ancestry = self.get_ancestry_chain(cp_id)?;
    
    Ok(RestoreResponse {
      checkpoint: cp,
      snapshots,
      ancestry,
    })
  }
  
  /// 选择性恢复：按source筛选快照
  /// 
  /// 示例：
  ///   restore_selective(cp_id, vec!["agent://"])  // 仅恢复Agent状态
  ///   restore_selective(cp_id, vec!["file://src/**"])  // 仅恢复source代码
  pub fn restore_selective(
    &self,
    cp_id: &CheckpointId,
    source_filters: Vec<&str>,
  ) -> Result<RestoreResponse> {
    let cp = self.get_checkpoint(cp_id)?;
    
    // 按source过滤snapshots
    let filtered_snapshots: Vec<_> = cp.baseline_snapshots
      .iter()
      .filter(|snap_id| {
        let source = cp.snapshot_sources.get(snap_id);
        source_filters.iter().any(|filter| {
          self.matches_glob(source.map(|s| s.as_str()).unwrap_or(""), filter)
        })
      })
      .cloned()
      .collect();
    
    let snapshots = self.load_snapshot_contents(&filtered_snapshots)?;
    let ancestry = self.get_ancestry_chain(cp_id)?;
    
    Ok(RestoreResponse {
      checkpoint: cp,
      snapshots,
      ancestry,
    })
  }
  
  /// 时间查询恢复：获取指定时间点最接近的checkpoint
  pub fn restore_by_time(
    &self,
    target_time: i64,
    source_filter: Option<&str>,
  ) -> Result<RestoreResponse> {
    // 在当前分支历史中查找时间最接近的checkpoint
    let mut closest: Option<&Checkpoint> = None;
    let mut min_diff = i64::MAX;
    
    for cp in self.checkpoints.values() {
      let diff = (cp.created_at - target_time).abs();
      if diff < min_diff {
        min_diff = diff;
        closest = Some(cp);
      }
    }
    
    let target_cp = closest.ok_or_else(||
      StratumError::NotFound("No checkpoint near target time".to_string())
    )?;
    
    // 如果指定了source filter则进行选择性恢复
    if let Some(filter) = source_filter {
      self.restore_selective(&target_cp.id, vec![filter])
    } else {
      self.restore_full(&target_cp.id)
    }
  }
  
  /// 列出checkpoint的所有快照信息
  pub fn list_snapshots(
    &self,
    cp_id: &CheckpointId,
  ) -> Result<Vec<(SnapshotId, String, usize)>> {
    let cp = self.get_checkpoint(cp_id)?;
    
    Ok(cp.baseline_snapshots.iter().map(|snap_id| {
      let source = cp.snapshot_sources
        .get(snap_id)
        .map(|s| s.clone())
        .unwrap_or_default();
      let snapshot = self.load_snapshot_content(snap_id);
      let size = snapshot.map(|s| s.size).unwrap_or(0);
      (*snap_id, source, size)
    }).collect())
  }
  
  /// 获取两个checkpoint之间的diff
  pub fn diff(
    &self,
    from_id: &CheckpointId,
    to_id: &CheckpointId,
  ) -> Result<CheckpointDiff> {
    let from_cp = self.get_checkpoint(from_id)?;
    let to_cp = self.get_checkpoint(to_id)?;
    
    // 计算快照差异
    let removed: Vec<_> = from_cp.baseline_snapshots.iter()
      .filter(|s| !to_cp.baseline_snapshots.contains(s))
      .cloned()
      .collect();
    
    let added: Vec<_> = to_cp.baseline_snapshots.iter()
      .filter(|s| !from_cp.baseline_snapshots.contains(s))
      .cloned()
      .collect();
    
    let modified: Vec<_> = from_cp.baseline_snapshots.iter()
      .filter(|s| to_cp.baseline_snapshots.contains(s))
      .filter_map(|snap_id| {
        let from_content = self.load_snapshot_content(snap_id).ok();
        let to_content = self.load_snapshot_content(snap_id).ok();
        if from_content != to_content {
          Some(*snap_id)
        } else {
          None
        }
      })
      .collect();
    
    Ok(CheckpointDiff {
      from_id: *from_id,
      to_id: *to_id,
      added,
      removed,
      modified,
    })
  }
}

pub struct CheckpointDiff {
  pub from_id: CheckpointId,
  pub to_id: CheckpointId,
  pub added: Vec<SnapshotId>,
  pub removed: Vec<SnapshotId>,
  pub modified: Vec<SnapshotId>,
}
```

### 1.3 事务和原子性

#### 需求
多个操作需要原子保证（Checkpoint + 多个Snapshot的一致性）

#### 设计方案

```rust
// crates/stratum/src/checkpoint/transaction.rs - 新增模块

pub struct CheckpointTransaction {
  /// 待提交的快照
  snapshots_to_commit: Vec<(SnapshotId, SnapshotContent)>,
  
  /// Checkpoint元数据
  checkpoint_metadata: CheckpointMetadata,
  
  /// 父checkpoint
  parents: Vec<CheckpointId>,
}

impl CheckpointTransaction {
  pub fn new(metadata: CheckpointMetadata, parents: Vec<CheckpointId>) -> Self {
    Self {
      snapshots_to_commit: Vec::new(),
      checkpoint_metadata: metadata,
      parents,
    }
  }
  
  /// 添加快照
  pub fn add_snapshot(
    mut self,
    source: &str,
    content: SnapshotContent,
  ) -> Self {
    let snap_id = self.compute_snapshot_id(source, &content);
    self.snapshots_to_commit.push((snap_id, content));
    self
  }
  
  /// 原子提交：事务失败则回滚
  pub fn commit(self, repo: &mut CheckpointRepo) -> Result<CheckpointId> {
    // 事务开始
    repo.storage_mut().begin_transaction()?;
    
    let mut baseline_snapshots = Vec::new();
    let mut snapshot_sources = HashMap::new();
    
    // 提交所有快照
    for (snap_id, content) in self.snapshots_to_commit {
      repo.storage_mut().store_snapshot(&snap_id, content.clone())?;
      baseline_snapshots.push(snap_id);
      snapshot_sources.insert(snap_id, source.to_string());
    }
    
    // 创建Checkpoint
    let cp = Checkpoint::new(
      baseline_snapshots,
      self.parents,
      self.checkpoint_metadata,
    );
    cp.snapshot_sources = snapshot_sources;
    let cp_id = cp.id;
    
    // 提交Checkpoint
    repo.storage_mut().store_checkpoint(&cp)?;
    
    // 事务提交
    repo.storage_mut().commit_transaction()?;
    
    repo.checkpoints.insert(cp_id, cp);
    Ok(cp_id)
  }
}

impl CheckpointRepo {
  /// 启动事务
  pub fn transaction(&mut self) -> CheckpointTransaction {
    CheckpointTransaction::new(
      CheckpointMetadata::new("system", "transaction"),
      vec![self.current_branch_head()],
    )
  }
}
```

### 1.4 时间索引支持

#### 需求
支持按时间范围快速查询checkpoint

#### 设计方案

```rust
// crates/stratum/src/checkpoint/time_index.rs - 新增模块

pub struct TimeIndex {
  /// 时间戳到checkpoint ID的映射（有序）
  entries: BTreeMap<i64, CheckpointId>,
}

impl TimeIndex {
  /// 查询指定时间范围内的checkpoints
  pub fn query_range(
    &self,
    from: i64,
    to: i64,
  ) -> Vec<(i64, CheckpointId)> {
    self.entries
      .range(from..=to)
      .map(|(time, cp_id)| (*time, *cp_id))
      .collect()
  }
  
  /// 查找距离target_time最近的checkpoint
  pub fn find_nearest(&self, target_time: i64) -> Option<(i64, CheckpointId)> {
    // 使用BTreeMap的range查询找到前驱和后继
    // 选择距离较近的一个
    let after = self.entries.range(target_time..).next();
    let before = self.entries.range(..target_time).next_back();
    
    match (before, after) {
      (Some((t1, id1)), Some((t2, id2))) => {
        if (target_time - t1).abs() < (t2 - target_time).abs() {
          Some((*t1, *id1))
        } else {
          Some((*t2, *id2))
        }
      },
      (Some((t, id)), None) => Some((*t, *id)),
      (None, Some((t, id))) => Some((*t, *id)),
      (None, None) => None,
    }
  }
}

impl CheckpointRepo {
  /// 添加到时间索引
  fn index_checkpoint(&mut self, cp: &Checkpoint) {
    if let Some(index) = &mut self.time_index {
      index.entries.insert(cp.created_at, cp.id);
    }
  }
}
```

---

## 二、API接口规范

### 2.1 HTTP API 扩展

当前Stratum已有HTTP API，需扩展以下端点：

```http
# 查询操作（GET）

GET /api/v1/checkpoint/{checkpointId}
  → CheckpointDetail (包含metadata和snapshot列表)

GET /api/v1/checkpoint/{checkpointId}/snapshots
  → SnapshotList (所有快照的类型和大小)

GET /api/v1/checkpoint/{checkpointId}/snapshots/{snapshotId}
  → SnapshotContent (快照的实际内容)

GET /api/v1/checkpoint/{checkpointId}/restore?sources=agent://,file://src/**
  → RestoreResponse (选择性恢复)

GET /api/v1/checkpoint/time/{timestamp}?source=agent://
  → RestoreResponse (时间查询)

GET /api/v1/checkpoint/diff?from={id1}&to={id2}
  → CheckpointDiff (差异对比)

# 写操作（POST）

POST /api/v1/checkpoint/transaction
{
  "snapshots": [
    {
      "source": "agent://loop-1",
      "content": {...}
    },
    {
      "source": "file://src/main.ts",
      "content": "..."
    }
  ],
  "message": "Agent iteration 5",
  "author": "agent-1"
}
→ CommitResponse { checkpoint_id }
```

### 2.2 gRPC API 扩展

```proto
// crates/stratum/src/api/proto/stratum.proto

service Stratum {
  // 现有RPC
  rpc Init(InitRequest) returns (InitResponse);
  rpc Edit(EditRequest) returns (EditResponse);
  rpc Commit(CommitRequest) returns (CommitResponse);
  rpc Log(LogRequest) returns (LogResponse);
  
  // 新增RPC
  rpc RestoreFull(RestoreFullRequest) returns (RestoreFullResponse);
  rpc RestoreSelective(RestoreSelectiveRequest) returns (RestoreSelectiveResponse);
  rpc RestoreByTime(RestoreByTimeRequest) returns (RestoreByTimeResponse);
  rpc ListSnapshots(ListSnapshotsRequest) returns (ListSnapshotsResponse);
  rpc DiffCheckpoints(DiffCheckpointsRequest) returns (DiffCheckpointsResponse);
  rpc CreateTransaction(CreateTransactionRequest) returns (TransactionHandle);
  rpc CommitTransaction(CommitTransactionRequest) returns (CommitResponse);
}

message RestoreFullRequest {
  string checkpoint_id = 1;
}

message RestoreFullResponse {
  CheckpointInfo checkpoint = 1;
  repeated SnapshotInfo snapshots = 2;
  repeated string ancestry = 3;
}

message RestoreSelectiveRequest {
  string checkpoint_id = 1;
  repeated string source_filters = 2;  // e.g., ["agent://", "file://src/**"]
}

message RestoreSelectiveResponse {
  CheckpointInfo checkpoint = 1;
  repeated SnapshotInfo snapshots = 2;
  repeated string ancestry = 3;
}

message ListSnapshotsRequest {
  string checkpoint_id = 1;
}

message ListSnapshotsResponse {
  repeated SnapshotInfo snapshots = 1;
}

message SnapshotInfo {
  string id = 1;
  string source = 2;
  uint64 size = 3;
  int64 created_at = 4;
}

message DiffCheckpointsRequest {
  string from_id = 1;
  string to_id = 2;
}

message DiffCheckpointsResponse {
  repeated string added = 1;
  repeated string removed = 2;
  repeated string modified = 3;
}
```

---

## 三、数据模型变更

### 3.1 SQLite Schema 更新

```sql
-- 扩展现有snapshot表
ALTER TABLE snapshots ADD COLUMN source TEXT DEFAULT '';
ALTER TABLE snapshots ADD COLUMN content_type TEXT DEFAULT 'file';  -- 'file', 'json', 'structured'

-- 扩展现有checkpoint表
ALTER TABLE checkpoints ADD COLUMN snapshot_sources TEXT;  -- JSON格式: {"snap_id": "source"}

-- 新增时间索引表
CREATE TABLE IF NOT EXISTS time_index (
  checkpoint_id TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL UNIQUE,
  FOREIGN KEY (checkpoint_id) REFERENCES checkpoints(id)
);

CREATE INDEX idx_time_index_created_at ON time_index(created_at);

-- 事务日志表（用于WAL）
CREATE TABLE IF NOT EXISTS transaction_log (
  id TEXT PRIMARY KEY,
  state TEXT NOT NULL,  -- 'pending', 'committed', 'rolled_back'
  checkpoints TEXT,  -- JSON array of checkpoint IDs
  created_at INTEGER NOT NULL
);
```

### 3.2 向后兼容性

- 现有文件快照继续正常工作
- 新的JSON元数据快照使用不同的source prefix
- 查询API自动适配两种快照类型

---

## 四、性能考虑

### 4.1 优化策略

| 操作 | 优化方式 |
|-----|---------|
| **快照加载** | 按需加载（支持延迟加载） |
| **查询** | 时间索引、source索引 |
| **事务提交** | 批量写入、日志预写 |
| **checkpoint查询** | DAG缓存、LRU缓存 |

### 4.2 快照压缩

```rust
// 支持快照压缩以节省存储
pub enum SnapshotCompression {
  None,
  Gzip,
  Zstd,
}

impl Snapshot {
  pub fn compress(&mut self, compression: SnapshotCompression) -> Result<()> {
    // 压缩content字段
    self.content = self.content.compress(compression)?;
    self.compression = compression;
    Ok(())
  }
}
```

---

## 五、测试要求

### 5.1 单元测试
- 快照操作（创建、读取、压缩）
- 恢复操作（完整、选择性、按时间）
- 事务操作（提交、回滚）
- Diff操作

### 5.2 集成测试
- 多快照transaction
- 并发commit
- 分支合并（涉及多个快照）
- 大型checkpoint（多个大快照）

### 5.3 性能基准
- 10MB快照：恢复时间 < 100ms
- 100个快照：列表查询 < 50ms
- 1000个checkpoint：时间查询 < 10ms

---

## 六、迁移路径

### 6.1 阶段
1. **快照扩展** → 支持JSON元数据快照
2. **恢复API** → 完整和选择性恢复
3. **事务支持** → 原子多快照提交
4. **时间索引** → 按时间查询
5. **HTTP/gRPC** → 接口扩展
6. **性能优化** → 缓存、索引、压缩

### 6.2 验收标准
- ✅ 所有新API可用且通过集成测试
- ✅ 向后兼容：旧快照正常查询
- ✅ 性能基准达标
- ✅ 文档完整

---

## 七、关键实现细节

### 7.1 Source标识符规范

```
agent://     → Agent执行状态
graph://     → Graph执行状态
file://      → 文件内容（现有）
system://    → 系统内部数据

示例：
  agent://loop-1/iteration-5
  graph://execution-abc/workflow-state
  file://src/main.ts
  system://metadata
```

### 7.2 快照ID生成

```rust
// 基于source + content hash生成
fn compute_snapshot_id(source: &str, content: &SnapshotContent) -> SnapshotId {
  let mut hasher = blake3::Hasher::new();
  hasher.update(source.as_bytes());
  hasher.update(&content.to_bytes());
  ContentId(hasher.finalize().into())
}
```

### 7.3 错误处理

定义清晰的错误类型：
```rust
pub enum StratumError {
  CheckpointNotFound(String),
  SnapshotNotFound(String),
  CorruptedData(String),
  TransactionConflict(String),
  InvalidRestoreFilter(String),
  IntegrityCheckFailed(String),
}
```

---

## 总结

| 核心改动 | 内容 |
|---------|------|
| **快照系统** | 支持多种内容类型（文件、JSON元数据、结构化数据） |
| **恢复机制** | 完整恢复、选择性恢复、时间查询 |
| **事务支持** | 原子多快照提交 |
| **查询能力** | 时间索引、diff对比、快照列表 |
| **API扩展** | HTTP + gRPC接口 |
| **数据存储** | SQLite schema扩展，保持向后兼容 |

这些改动使Stratum能够**完整地管理Agent/Graph执行状态的版本历史**，而不仅仅是文件版本。
