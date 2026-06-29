# Git Sync 重构设计方案

## 背景

当前 Git Sync 模块存在以下设计问题：

1. **`push_to_remote` 直接干涉远程仓库**：调用 `git push` 改写远程分支，用户无法控制推送时机和策略
2. **`push_to_git` 侵入式写入 HEAD + working tree**：修改 git 检出状态，可能覆盖用户未提交的工作
3. **职责耦合**：内容重建、磁盘写入、git commit、remote push 四件事绑定在一个方法里
4. **缺乏配置**：无法自定义目标分支、ref 命名空间、clean tree 检查等

## 设计原则

1. **Layertwine 管融合，Git 管版本，用户管推送**
   - layertwine 负责从 checkpoint 重建文件内容
   - git 负责版本记录（layertwine 写入后 git 自动检测变更）
   - 用户决定何时 `git push`、推送到哪个远程

2. **非侵入式**：layertwine 的 git 操作不应影响用户的 git 工作流（HEAD、index、working tree）

3. **渐进式配置**：默认行为兼容当前用法，通过配置逐步迁移到更安全的模式

## 架构变更

### 移除 `push_to_remote`

**原因**：
- 用户应自行控制推送时机和认证
- 远程推送涉及网络、认证、冲突策略，不应由 layertwine 代劳
- `git push` 后的远程结果超出了 layertwine 的责任范围

**替代方式**：用户执行 `layertwine sync` 后自行 `git push origin <branch>`

### 保留 `fetch_from_remote`

`fetch_from_remote` 是只读操作（获取远程 refs 到本地），不修改远程仓库，保留用于 `pull` 流程。

### 重构 `push_to_git`

拆分为两个方法：

```
push_to_git (保留，修改)
  → sync_to_working_tree: 内容重建 → 写入磁盘 (可选 step)
  → commit_to_git: stage + commit → git_anchor 更新
```

通过 `GitSyncConfig` 控制行为差异。

## GitSyncConfig 配置

```rust
pub struct GitSyncConfig {
    /// Ref 命名空间
    /// CurrentBranch: 写入 HEAD + working tree（当前行为）
    /// Isolated: 写入 refs/layertwine/<branch>，不碰 working tree
    pub ref_namespace: RefNamespace,

    /// 是否要求 git working tree 干净后才执行 commit
    pub require_clean_tree: bool,

    /// 是否写入 working tree（仅 CurrentBranch 模式有效）
    /// false = 只写 git blob/tree，不写磁盘
    pub write_working_tree: bool,
}

pub enum RefNamespace {
    CurrentBranch,
    Isolated,
}
```

### CurrentBranch 模式（默认，向后兼容）

- 写入 working tree → stage → commit(Some("HEAD"))
- 等同于当前 `push_to_git` 行为
- 适用于"layertwine 直接管理当前分支"的场景

### Isolated 模式（推荐）

- 用 `TreeBuilder` 直接创建 tree 对象，不碰 working tree
- commit 到 `refs/layertwine/<branch_name>`
- 不影响 HEAD、index、working tree
- 适用于"layertwine 独立管理版本，用户手动合并"的场景
- 需要额外 merge 命令来将 `refs/layertwine/*` 的变更合并到工作分支

## 命令变更

### CLI

```bash
# 旧
layertwine push --remote origin -m "msg"    # 推送本地 + 远程

# 新
layertwine sync [--branch <name>] [-m <msg>]  # 仅本地 git commit
```

- `push` 命令改为 `sync`（或保留 `push` 但移除 `--remote`）
- 不再支持 `--remote` 参数
- `sync` 完成后输出 git commit hash，用户自行 `git push`

### HTTP API

```json
// POST /api/v1/sync   (替代 /api/v1/push)
{
  "git_repo": "/path/to/repo",
  "message": "sync from layertwine",
  "branch": "main"          
}
// 响应
{
  "success": true,
  "data": {
    "git_commit_hash": "abc123..."
  }
}
```

- 移除 `remote` 字段
- 移除 `/api/v1/push` 端点，改为 `/api/v1/sync`
- 或保留 `/api/v1/push` 但语义改为本地 commit

## 回滚设计

回滚 = 从 checkpoint 重建文件 + 写入磁盘（不改 git 历史）

```
layertwine checkpoint rollback <checkpoint_id>
  → 从 checkpoint 获取 baseline_snapshots
  → reconstruct_text 重建每个文件
  → 写入 git working tree（覆盖磁盘文件）
  → git status 显示变更
```

用户确认后自行 `git commit` 完成回滚。

回滚不需要 push、不需要新建分支，直接改写工作区文件。

## Merge 命令（下一阶段考虑）

当使用 `Isolated` 模式时，需要一种方式将 `refs/layertwine/<branch>` 的变更合并到工作分支。

```
layertwine merge <source> <target>
  前置条件：
    - working tree 干净
    - source 存在（layertwine checkpoint branch）
    - target 存在（git branch）
  流程：
    - 从 source checkpoint 重建文件
    - checkout target 分支
    - 写入文件 → git add → git commit
```

如果直接使用 `CurrentBranch` 模式（HEAD + 主分支），则不需要 merge 命令。

## 实施计划

### 第一阶段（本次实施）

1. 移除 `push_to_remote` 方法
2. 添加 `GitSyncConfig` + `RefNamespace`
3. `push_to_git` 支持 `Isolated` 模式（TreeBuilder）
4. 更新 CLI：`push` 命令移除 `--remote`，改为纯本地
5. 更新 HTTP API：移除 `remote` 字段
6. 更新测试

### 第二阶段（后续）

1. 实现 `sync_to_working_tree` / `commit_to_git` 拆分
2. `require_clean_tree` 检查
3. Merge 命令（Isolated 模式的必要补充）
4. 文档更新

## 配置示例

```toml
# layertwine.toml
[git_sync]
# 可选值: "current_branch"（默认）, "isolated"
ref_namespace = "isolated"

# 是否要求 working tree 干净
require_clean_tree = true

# 是否写入 working tree
write_working_tree = false
```