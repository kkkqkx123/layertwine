# P6 — Git 双向同步与冗余检查点 GC

> 参考文档：
> - [Git同步与GC](../architecture/06-Git同步与GC.md) — 完整设计
> - [设计方案 §七-八](../设计方案.md) — 同步与 GC 规则
> - [Git-delta存储 §3.5-3.6](../reference/03-git-delta存储.md) — Git 底层格式参考
> - [复用分析 §3](../reference/05-复用分析.md#3-git-delta-格式--不能依赖仅参考格式规范) — Delta 格式差异

## 目标

实现 Git ↔ 自研仓库的双向同步（基线对齐），以及自研仓库的冗余检查点 GC 机制。同步仅做基线对齐，不干涉编辑分层。

## 任务清单

### 6.1 Git 桥接抽象（`git_sync/git_bridge.rs`）[2h]

按 [Git同步 §6.2-6.3](../architecture/06-Git同步与GC.md) 实现：

- [ ] `GitBridge` 结构体 — 封装 `git2` crate 调用
- [ ] **Git → 自研仓库（初始化/拉取）**：
  - `fn init_from_git(git_repo, checkpoint_repo, git_ref) -> Result<()>`
  - 从 Git 检出文件 → 生成 FileNode + Delta + Snapshot → 构建初始 Checkpoint
  - 绑定 `git_anchor`
  - 参考 [Git同步 §6.2](../architecture/06-Git同步与GC.md) 的伪代码
- [ ] **自研仓库 → Git（推送/归档）**：
  - `fn push_to_git(checkpoint_repo, git_repo, branch_name, message) -> Result<()>`
  - 从 Checkpoint 恢复文件内容
  - 写入 Git 工作树 → add → commit → push
  - 参考 [Git同步 §6.3](../architecture/06-Git同步与GC.md) 的伪代码
- [ ] **比较状态**：`fn compare_status(checkpoint_repo, git_repo) -> Result<SyncStatus>`
  - 对比 Git HEAD 和自研仓库当前基线
  - 检测同步偏差

### 6.2 同步铁律实现 [0.5h]

按 [Git同步 §6.4 同步铁律](../architecture/06-Git同步与GC.md#64-同步铁律) 实现：

- [ ] 仅在 staged 提交（Checkpoint）时同步
- [ ] 同步不干涉编辑分层状态
- [ ] 同步失败不丢失编辑数据
- [ ] Git 锚点作为 Checkpoint metadata 存储

### 6.3 冗余检查点 GC（`git_sync/gc.rs`）[1.5h]

按 [Git同步 §6.5 GC](../architecture/06-Git同步与GC.md#65-冗余检查点-gc-机制) 和 [设计方案 §八](../设计方案.md#八冗余检查点-gc-机制) 实现：

- [ ] **保护规则**（不可删除的 Checkpoint）：
  - 所有分支 head 及其祖先
  - 绑定了 `git_anchor` 的 Checkpoint
  - 参考 [Git同步 §6.5](../architecture/06-Git同步与GC.md) 的 `collect_protected_checkpoints` 伪代码
- [ ] **清理规则**：
  - `fn collect_garbage(checkpoint_repo) -> Result<GCStats>`
  - 标记-清除：从未保护节点开始遍历，标记可达，清除不可达
  - 清理不可达的 Snapshot/Delta（引用计数归零的）
- [ ] **GC 统计**：`GCStats { removed_checkpoints, removed_snapshots, freed_bytes }`
- [ ] **Delta 链深度控制**：检测增量链深度 > 100 时触发重打包（参考 [Git-delta存储 §3.3.2](../reference/03-git-delta存储.md#332-增量链与-gc-参考)）

### 6.4 Cargo.toml 添加 git2 [0.1h]

- [ ] `cargo add git2`

### 6.5 单元测试 [1h]

- [ ] 从 Git 仓库初始化自研仓库
- [ ] 自研仓库提交后推送到 Git
- [ ] GC 保护规则：已保护节点不被删除
- [ ] GC 清理规则：不可达节点被删除
- [ ] 同步状态比对

## 验收标准

- [ ] 可从 Git 检出初始化自研仓库（含初始 Checkpoint）
- [ ] 自研仓库可推送到 Git
- [ ] Git 锚点绑定正确
- [ ] GC 保护规则正确（分支 head + git_anchor 不被清理）
- [ ] GC 清理后不可达实体被移除
