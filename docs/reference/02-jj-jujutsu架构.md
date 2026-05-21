# 2. jj (Jujutsu) — 现代化版本控制系统架构参考

> 源码路径：`ref/jj-0.41.0/lib/src/`
> 相关文件：`store.rs`, `backend.rs`, `repo.rs`, `commit_builder.rs`, `working_copy.rs`, `local_working_copy.rs`, `git_backend.rs`

## 2.1 设计哲学 — 本项目可借鉴的核心思想

### 2.1.1 读写分离的 Repo 设计

jj 将仓库分为 **只读** 和 **可变** 两层，只在 transaction 中变更：

```
ReadonlyRepo (不可变快照)
    │
    ├── store() → &Store
    ├── index() → &dyn Index
    ├── view() → &View
    └── start_transaction() → Transaction
                │
                ▼
         Transaction
                │
                ▼
         MutableRepo (可变层)
                │
                └── commit() → ReadonlyRepo (新版本)
```

**本项目对应**：
- `ReadonlyRepo` → `Partition` + `Layer`（不可变快照）
- `MutableRepo` → `manual_edit` / `agent_edit` 层（可变操作区）
- `Transaction` → `approval` → `staged` 的状态流转

### 2.1.2 Store / Backend 分离

```
Store (高层封装 + 缓存)
  │
  └── Backend trait (底层存储抽象)
        ├── GitBackend (真正实现)
        ├── SimpleBackend (测试用)
        └── ...
```

**Store** 提供缓存和便利接口，**Backend trait** 定义存储操作。

```rust
// backend.rs - 核心 trait (简化)
pub trait Backend: Any + Send + Sync + Debug {
    fn name(&self) -> &str;
    fn commit_id_length(&self) -> usize;
    fn root_commit_id(&self) -> &CommitId;
    fn empty_tree_id(&self) -> &TreeId;

    async fn read_file(&self, path: &RepoPath, id: &FileId)
        -> BackendResult<Pin<Box<dyn AsyncRead + Send>>>;
    async fn write_file(&self, path: &RepoPath, contents: &mut (dyn AsyncRead + Send + Unpin))
        -> BackendResult<FileId>;
    async fn read_commit(&self, id: &CommitId) -> BackendResult<Commit>;
    async fn write_commit(&self, commit: Commit, signer: Option<&mut SigningFn>)
        -> BackendResult<(CommitId, Commit)>;
    fn gc(&self, index: &dyn Index, keep_newer: SystemTime) -> BackendResult<()>;
    // ...
}
```

**本项目对应**：
- `Backend` → `SnapshotStore` trait（定义 Snapshot/Delta 的读写）
- `Store` → 缓存层（HashMap 缓存已读取的 Snapshot 对象）
- `GitBackend` → `SqliteRepo`（具体实现）

### 2.1.3 CommitBuilder 模式

jj 使用 Builder 模式构建不可变 Commit：

```rust
// 使用链式调用构建，最终 write() 提交到存储
let commit = mut_repo
    .new_commit(vec![parent_id], merged_tree)
    .set_description("fix: bug")
    .set_author(signature)
    .write()
    .await?;

// 改写场景：从已有 commit 派生
let rewritten = mut_repo
    .rewrite_commit(&old_commit)
    .set_description("updated message")
    .write()
    .await?;
```

**本项目对应**：
```rust
// SnapshotBuilder — 构造不可变 Snapshot
let snapshot = SnapshotBuilder::new(partition_id)
    .add_delta(delta_id1)
    .add_delta(delta_id2)
    .with_parent(parent_snapshot_id)
    .build()?;  // 返回 Snapshot，不可变
```

### 2.1.4 GitBackend 的 "额外元数据" 策略

jj 的 GitBackend 在 Git 对象之上额外存储元数据（change_id、predecessors 等），使用 `TableStore`（LSM 风格的追加式表）：

```rust
pub struct GitBackend {
    base_repo: gix::ThreadSafeRepository,  // 底层 Git 仓库
    extra_metadata_store: TableStore,       // 追加式元数据表
    // ...
}
```

**本项目参考**：Snapshot 的 `partition_id` 和文件路径元数据，与本项目的 Delta 无关内容可用类似策略分层存储。

## 2.2 关键数据结构

### Commit（核心不可变实体）

```rust
pub struct Commit {
    pub parents: Vec<CommitId>,
    pub predecessors: Vec<CommitId>,
    pub root_tree: Merge<TreeId>,
    pub change_id: ChangeId,
    pub description: String,
    pub author: Signature,
    pub committer: Signature,
}
```

### Tree（目录结构）

```rust
pub struct Tree {
    entries: Vec<(RepoPathComponentBuf, TreeValue)>,
    // 按 RepoPathComponent 排序，二分查找
}

pub enum TreeValue {
    File { id: FileId, executable: bool },
    Symlink(SymlinkId),
    Tree(TreeId),
    GitSubmodule(CommitId),
    Conflict(Merge<TreeValue>),
}
```

### View（引用/分支视图）

View 存储所有分支、标签、工作副本 commit 的引用状态：
```rust
pub struct View {
    // local_bookmarks: HashMap<RefName, RefTarget>,
    // remote_bookmarks: HashMap<(RemoteName, RefName), RemoteRef>,
    // tags: HashMap<RefName, RefTarget>,
    // git_refs: HashMap<GitRefName, RefTarget>,
    // wc_commits: HashMap<WorkspaceName, CommitId>,
}
```

### Repo trait

```rust
pub trait Repo {
    fn store(&self) -> &Arc<Store>;
    fn op_store(&self) -> &Arc<dyn OpStore>;
    fn index(&self) -> &dyn Index;
    fn view(&self) -> &View;
    fn resolve_change_id_prefix(&self, prefix: &str) -> ...;
    fn shortest_unique_change_id_prefix_len(&self, id: &ChangeId) -> usize;
}
```

**本项目对应**：
- `Repo` → `Partition`（状态查询接口）
- `Store` → `SqliteRepo`（存储接口）
- `View` → `Layer`（管理分支指针）

## 2.3 WorkingCopy — 工作副本分层设计

```rust
pub trait WorkingCopy: Any + Send {
    fn operation_id(&self) -> &OperationId;
    fn tree(&self) -> Option<&MergedTree>;
}

pub trait LockedWorkingCopy: Any + Send {
    async fn snapshot(&mut self, options: &SnapshotOptions)
        -> Result<(MergedTree, SnapshotStats), SnapshotError>;
    // 检出操作
}
```

local_working_copy 实现了 `FileState` 跟踪（mtime + size），类似于 Git 的索引缓存：

```rust
pub struct FileState {
    pub file_type: FileType,
    pub mtime: MillisSinceEpoch,
    pub size: u64,
}
```

**本项目对应**：
- `WorkingCopy` → `Layer`（工作区状态）
- `LockedWorkingCopy` → 编辑操作锁定机制
- `FileState` → 行级变更跟踪（类似但粒度更细）

## 2.4 分层缓存架构

```rust
pub struct Store {
    backend: Box<dyn Backend>,
    commit_cache: Mutex<CLruCache<CommitId, Arc<backend::Commit>>>,
    tree_cache: Mutex<CLruCache<(RepoPathBuf, TreeId), Arc<backend::Tree>>>,
}
```

**本项目参考**：SqliteRepo 也可添加 LRU 缓存层，缓存最近读取的 Snapshot/Delta 对象。

## 2.5 与项目架构的映射

| jj 概念 | 本项目概念 | 借鉴点 |
|---------|-----------|--------|
| `Backend` trait | `SnapshotStore` trait | 存储抽象 |
| `Store` | `SqliteRepo` + 缓存 | 读缓存 |
| `ReadonlyRepo` | `Partition` | 不可变快照 |
| `MutableRepo` | `manual_edit` / `agent_edit` | 可变层 |
| `CommitBuilder` | `SnapshotBuilder` | Builder 模式 |
| `Transaction` | `Layer::transition()` | 状态流转 |
| `View` | `Branch` 指针 | 引用管理 |
| `WorkingCopy` + `FileState` | `Layer` 文件状态 | 层级追踪 |
