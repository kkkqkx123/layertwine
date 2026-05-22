# Stratum CLI 使用指南

> Stratum（地层）—— 轻量文件编辑历史存储层，专为多 Agent 协同 + 人工编辑混合场景设计。

## 概述

Stratum 提供一套完整的命令行接口（CLI），用于管理文件编辑历史、分层状态机流转、检查点提交、分支管理、快照备份以及 Git 双向同步。

当前 CLI 通过 Rust 库暴露，入口为 `stratum::cli::run()`。所有功能均通过 clap v4 子命令系统组织。

---

## 全局选项

| 选项                    | 说明                         | 默认值                |
| ----------------------- | ---------------------------- | --------------------- |
| `-d, --db <PATH>`       | Stratum 数据库文件路径       | `.stratum/stratum.db` |
| `-g, --git-repo <PATH>` | Git 仓库路径（用于同步操作） | 无                    |
| `--json`                | JSON 输出模式（全局标志）    | 否                    |
| `-h, --help`            | 打印帮助信息                 | —                     |
| `-V, --version`         | 打印版本号                   | —                     |

---

## 命令参考

### `stratum init` — 初始化仓库

```
stratum init [--git-ref <REF>]
```

在当前目录初始化一个新的 Stratum 仓库。若指定了 `--git-repo`，则同时从 Git 仓库导入基线。

**选项：**

| 选项              | 说明                                                         |
| ----------------- | ------------------------------------------------------------ |
| `--git-ref <REF>` | 从 Git 仓库初始化时指定的引用（如 `HEAD`、分支名、提交哈希） |

**示例：**

```bash
# 初始化空仓库
stratum init

# 从 Git 仓库导入基线初始化
stratum --git-repo /path/to/repo init --git-ref HEAD
```

**输出：**

```
  Initializing stratum repository ... done
Initialized empty stratum repository at '.stratum/stratum.db'
  Manual partition: <uuid>
  Staged partition: <uuid>
  Branch: main
```

---

### `stratum status` — 查看当前状态

```
stratum status
```

显示所有层的分区状态，包括当前快照 ID 和历史记录数量。

**输出示例（Plain 模式）：**

```
------------------------------------------------------------------------
Layer            Partition                Current Snapshot    History
------------------------------------------------------------------------
manual_edit      manual                   a1b2c3d4e5f6       3 snapshots
staged           staged                   f6e5d4c3b2a1       5 snapshots
agent_edit       agent:agent-01           b2c3d4e5f6a1       2 snapshots
approval         approved:agent-01        c3d4e5f6a1b2       1 snapshots
------------------------------------------------------------------------
```

**输出示例（JSON 模式，`--json`）：**

```json
{
  "status": "ok",
  "partitions": [
    {
      "layer": "manual_edit",
      "partition": "manual",
      "current_snapshot": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4",
      "history_len": 3
    }
  ]
}
```

---

### `stratum edit` — 手动编辑文件

```
stratum edit <FILE> [-c, --content <CONTENT>]
```

对指定文件进行手动编辑，记录到 `manual_edit` 层。编辑后自动合并到 `staged` 层。

**参数：**

| 参数                      | 说明                             |
| ------------------------- | -------------------------------- |
| `<FILE>`                  | 要编辑的文件路径（必需）         |
| `-c, --content <CONTENT>` | 新文件内容。若未提供，使用空内容 |

**示例：**

```bash
stratum edit src/main.rs -c "fn main() {}"
```

**输出：**

```
Edited 'src/main.rs' -> new snapshot a1b2c3d4e5f6
Merged to staged -> snapshot f6e5d4c3b2a1
```

---

### `stratum agent` — Agent 操作

```
stratum agent <AGENT_ID> edit <FILE> [-c, --content <CONTENT>]
stratum agent <AGENT_ID> submit
```

管理 Agent 的编辑和提交流程。

**子命令：**

| 子命令                       | 说明                                                 |
| ---------------------------- | ---------------------------------------------------- |
| `edit <FILE> [-c <CONTENT>]` | Agent 编辑指定文件，记录到 `agent_edit` 层           |
| `submit`                     | Agent 提交当前编辑内容到 `approval` 层，等待人工审核 |

**参数：**

| 参数                      | 说明                                |
| ------------------------- | ----------------------------------- |
| `<AGENT_ID>`              | Agent 实例 ID（必需）               |
| `<FILE>`                  | 要编辑的文件路径（edit 子命令必需） |
| `-c, --content <CONTENT>` | 新文件内容（edit 子命令可选）       |

**示例：**

```bash
# Agent 编辑文件
stratum agent agent-01 edit src/module.rs -c "pub fn new() -> Self { Self }"

# Agent 提交审核
stratum agent agent-01 submit
```

**输出：**

```
Agent 'agent-01' edited 'src/module.rs' -> snapshot b2c3d4e5f6a1
Agent 'agent-01' submitted for approval -> snapshot c3d4e5f6a1b2
```

---

### `stratum approve` — 审核通过 Agent 修改

```
stratum approve <AGENT_ID>
```

审核通过指定 Agent 的修改。流程：

1. 将 Agent 的修改从 `approval` 层迁移到 `integrated` 分区
2. 将所有 `integrated` 分区合并到 `unified` 分区
3. 将 `unified` 分区合并到 `staged` 层

**参数：**

| 参数         | 说明                               |
| ------------ | ---------------------------------- |
| `<AGENT_ID>` | 要审核通过的 Agent 实例 ID（必需） |

**示例：**

```bash
stratum approve agent-01
```

**输出：**

```
Approved agent 'agent-01' -> integrated snapshot d4e5f6a1b2c3
Merged to staged -> snapshot e5f6a1b2c3d4
```

---

### `stratum commit` — 提交检查点

```
stratum commit -m <MESSAGE> [-a <AUTHOR>]
```

将 `staged` 层的当前状态提交为一个检查点（Checkpoint），记录到自研检查点仓库。

**参数：**

| 参数                      | 说明                          |
| ------------------------- | ----------------------------- |
| `-m, --message <MESSAGE>` | 提交信息（必需）              |
| `-a, --author <AUTHOR>`   | 作者名称（可选，默认 `user`） |

**示例：**

```bash
stratum commit -m "实现用户登录模块" -a "developer-1"
```

**输出：**

```
Committed checkpoint a1b2c3d4e5f6: 实现用户登录模块
```

---

### `stratum log` — 查看提交历史

```
stratum log [--count <N>]
```

查看检查点提交历史，支持表格和 JSON 输出。

**参数：**

| 参数          | 说明                          |
| ------------- | ----------------------------- |
| `--count <N>` | 最大显示数量（可选，默认 20） |

**输出示例（Plain 模式）：**

```
----------------------------------------------------------------------------------------------------
Checkpoint ID         Author           Parents      Snapshots     Message
----------------------------------------------------------------------------------------------------
a1b2c3d4e5f6         developer-1      1            3             实现用户登录模块
b2c3d4e5f6a1         developer-1      1            2             修复边界条件
c3d4e5f6a1b2 [git]   developer-1      1            2             从 Git 同步 [git]
----------------------------------------------------------------------------------------------------
Total: 3 checkpoint(s)
```

---

### `stratum branch` — 分支管理

```
stratum branch create <NAME>
stratum branch switch <NAME>
stratum branch list
```

**子命令：**

| 子命令          | 说明                             |
| --------------- | -------------------------------- |
| `create <NAME>` | 创建新分支（基于当前最新检查点） |
| `switch <NAME>` | 切换到指定分支                   |
| `list`          | 列出所有分支                     |

**示例：**

```bash
# 创建新分支
stratum branch create feature/login

# 切换分支
stratum branch switch feature/login

# 列出所有分支
stratum branch list
```

**输出（list）：**

```
------------------------------------------------------------
Branch                   Head                 Updated
------------------------------------------------------------
* main                   a1b2c3d4e5f6         2026-05-22 10:00:00
  feature/login          b2c3d4e5f6a1         2026-05-22 10:30:00
------------------------------------------------------------
```

---

### `stratum merge` — 合并分支

```
stratum merge <BRANCH> [-m, --message <MESSAGE>]
```

将指定分支合并到当前分支。

**参数：**

| 参数                      | 说明                               |
| ------------------------- | ---------------------------------- |
| `<BRANCH>`                | 源分支名称（必需）                 |
| `-m, --message <MESSAGE>` | 合并提交信息（可选，默认 `merge`） |

**示例：**

```bash
stratum merge feature/login -m "合并登录功能到主分支"
```

**输出：**

```
Merged 'feature/login' into 'main' -> checkpoint d4e5f6a1b2c3
```

---

### `stratum backup` — 备份快照

```
stratum backup <SNAPSHOT_ID> [--label <LABEL>]
```

将指定快照备份到独立的备份仓库（默认写入 `stratum-backup.db`）。

**参数：**

| 参数              | 说明                              |
| ----------------- | --------------------------------- |
| `<SNAPSHOT_ID>`   | 要备份的快照 ID（十六进制，必需） |
| `--label <LABEL>` | 备份标签（可选）                  |

**示例：**

```bash
stratum backup a1b2c3d4e5f6 --label "发布前基线"
```

**输出：**

```
Backup f6e5d4c3b2a1 created for snapshot a1b2c3d4e5f6 (label: 发布前基线)
```

---

### `stratum restore` — 从备份恢复

```
stratum restore <BACKUP_ID>
```

从备份仓库恢复指定备份的快照数据到主存储。

**参数：**

| 参数          | 说明                      |
| ------------- | ------------------------- |
| `<BACKUP_ID>` | 备份 ID（十六进制，必需） |

**示例：**

```bash
stratum restore f6e5d4c3b2a1
```

**输出：**

```
Restored backup f6e5d4c3b2a1 -> file: src/main.rs
```

---

### `stratum gc` — 执行垃圾回收

```
stratum gc
```

清理冗余检查点和快照数据。当 Delta 链深度超过阈值时触发深度压缩。

**示例：**

```bash
stratum gc
```

**输出：**

```
  Running garbage collection ... done
GC complete: 5 checkpoints removed, 12 snapshots freed, 102400 bytes
  Note: delta chain depth exceeded threshold (15)
```

---

### `stratum push` — 推送到 Git

```
stratum push [--remote <REMOTE>] [-m, --message <MESSAGE>]
```

将当前分支的检查点历史推送到关联的 Git 仓库。需要预先通过 `--git-repo` 指定 Git 仓库路径。

**参数：**

| 参数                      | 说明                                           |
| ------------------------- | ---------------------------------------------- |
| `--remote <REMOTE>`       | Git 远程名称（可选，默认 `origin`）            |
| `-m, --message <MESSAGE>` | Git 提交信息（可选，默认 `sync from stratum`） |

**示例：**

```bash
stratum --git-repo /path/to/repo push --remote origin -m "同步检查点到 Git"
```

**输出：**

```
  Pushing to Git ... done
Pushed to remote 'origin' (commit: 9f8e7d6c5b4a)
```

---

### `stratum pull` — 从 Git 拉取

```
stratum pull [--remote <REMOTE>] [--git-ref <REF>]
```

从关联的 Git 仓库拉取并导入最新提交作为 Stratum 检查点。需要预先通过 `--git-repo` 指定 Git 仓库路径。

**参数：**

| 参数                | 说明                                |
| ------------------- | ----------------------------------- |
| `--remote <REMOTE>` | Git 远程名称（可选，默认 `origin`） |
| `--git-ref <REF>`   | Git 引用（可选，默认 `HEAD`）       |

**示例：**

```bash
stratum --git-repo /path/to/repo pull --remote origin --git-ref main
```

**输出：**

```
  Fetching from Git remote ... done
  Importing from Git ... done
Pulled from remote 'origin' ref 'main'
```

---

## 典型工作流程

### 单人编辑流程

```bash
# 1. 初始化仓库
stratum init

# 2. 编辑文件
stratum edit src/main.rs -c "fn main() { println!(\"Hello\"); }"

# 3. 提交检查点
stratum commit -m "初始提交"

# 4. 查看状态和历史
stratum status
stratum log
```

### 多 Agent 协同流程

```bash
# 1. 初始化
stratum init

# 2. Agent A 编辑并提交审核
stratum agent agent-a edit src/auth.rs -c "pub fn login() {}"
stratum agent agent-a submit

# 3. Agent B 编辑并提交审核
stratum agent agent-b edit src/db.rs -c "pub fn connect() {}"
stratum agent agent-b submit

# 4. 人工审核
stratum approve agent-a
stratum approve agent-b

# 5. 提交最终检查点
stratum commit -m "合并 auth 和 db 模块"
```

### Git 集成流程

```bash
# 1. 从已有 Git 仓库初始化
stratum --git-repo /path/to/repo init --git-ref main

# 2. 在 Stratum 中进行编辑
stratum edit src/lib.rs -c "// new code"
stratum commit -m "Stratum 编辑"

# 3. 将检查点推回 Git
stratum --git-repo /path/to/repo push

# 4. 拉取 Git 更新到 Stratum
stratum --git-repo /path/to/repo pull
```

---

## 架构对应关系

每个 CLI 命令对应的内部模块：

| CLI 命令            | 所属模块                                 | 对应架构层        |
| ------------------- | ---------------------------------------- | ----------------- |
| `init`              | `state_machine` + `storage`              | P1 存储层         |
| `status`            | `cli::output`                            | P3 状态机查询     |
| `edit`              | `state_machine::manual`                  | P3 manual_edit 层 |
| `agent edit/submit` | `state_machine::agent`                   | P3 agent_edit 层  |
| `approve`           | `state_machine::approval`                | P3 approval 层    |
| `commit`            | `state_machine::staged` + `checkpoint`   | P4 检查点仓库     |
| `log`               | `checkpoint` + `storage`                 | P4 历史查询       |
| `branch`            | `checkpoint::branch` + `checkpoint::dag` | P4 分支管理       |
| `merge`             | `checkpoint::repo` + `engine::merge`     | P4/P2 合并引擎    |
| `backup`            | `backup`                                 | P5 备份模块       |
| `restore`           | `backup`                                 | P5 恢复模块       |
| `gc`                | `git_sync::gc`                           | P6 垃圾回收       |
| `push`              | `git_sync::git_bridge`                   | P6 Git 同步       |
| `pull`              | `git_sync::git_bridge`                   | P6 Git 同步       |

---

## 错误码说明

| 退出码 | 含义     | 常见原因                               |
| ------ | -------- | -------------------------------------- |
| 0      | 成功     | —                                      |
| 1      | 一般错误 | 数据库损坏、模块内部错误、实体未找到   |
| 2      | 参数错误 | 缺少必选参数、参数格式错误、分支已存在 |

所有错误信息均包含上下文描述和修复建议（hint）。
