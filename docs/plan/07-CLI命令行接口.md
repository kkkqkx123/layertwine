# P7 — CLI 命令行接口

> 参考文档：
> - [Rust实现方案 §7.2](../architecture/07-Rust实现方案.md) — 项目结构中的 cli/ 目录
> - [设计方案 §十二](../设计方案.md#十二配套工具) — 配套工具概述
> - [jj架构 §2.1.1](../reference/02-jj-jujutsu架构.md) — jj CLI 设计参考

## 目标

实现完整的命令行接口，基于 `clap` v4 框架，提供所有核心功能的 CLI 入口、格式化输出、友好错误提示。

## 任务清单

### 7.1 clap 子命令定义（`cli/commands.rs`）[1.5h]

- [ ] **整体 CLI 框架**：
  - `stratum init [--git-ref]` — 初始化仓库
  - `stratum status` — 查看当前状态
  - `stratum edit <file>` — 手动编辑（manual_edit 层）
  - `stratum agent <id> edit <file>` — Agent 编辑
  - `stratum agent <id> submit` — Agent 提交审核
  - `stratum approve <agent-id>` — 审核通过
  - `stratum commit -m <message>` — 提交 Checkpoint
  - `stratum log [--count]` — 查看提交历史
  - `stratum branch <name>` — 分支操作（create/switch/list）
  - `stratum merge <branch>` — 合并分支
  - `stratum backup <snapshot-id> [--label]` — 备份快照
  - `stratum restore <backup-id>` — 从备份恢复
  - `stratum gc` — 执行 GC
  - `stratum push [--remote]` — 推送到 Git
  - `stratum pull [--remote]` — 从 Git 拉取
- [ ] 参数验证：路径存在性检查、必选参数缺失检查

### 7.2 命令执行器（`cli/commands.rs` 或 `main.rs`）[1.5h]

- [ ] 每个子命令映射到对应模块的 API 调用
- [ ] 错误处理：CLI 错误 → 用户友好的错误信息
- [ ] 进度指示（长时间操作如 GC/Push 时）

### 7.3 格式化输出（`cli/output.rs`）[1h]

- [ ] `fn print_status(state_machine)` — 显示各层分区的当前状态
- [ ] `fn print_log(checkpoints)` — 提交历史表格输出
- [ ] `fn print_branches(branches)` — 分支列表
- [ ] `fn print_diff(delta)` — Delta 差异展示（类似 `git diff`）
- [ ] `fn print_snapshot(snapshot)` — 快照摘要
- [ ] JSON 输出模式（`--json` 标志）

### 7.4 错误处理（`error.rs` 扩展）[0.5h]

- [ ] CLI 友好的错误格式化（带上下文和建议）
- [ ] 退出码定义（0=成功, 1=一般错误, 2=参数错误）

### 7.5 集成测试 [1h]

- [ ] 初始化仓库
- [ ] 编辑文件 → 查看状态
- [ ] 提交 Checkpoint → 查看日志
- [ ] 创建/切换分支
- [ ] 备份/恢复

## 验收标准

- [ ] 所有核心功能可通过 CLI 调用
- [ ] 格式化输出清晰可读
- [ ] 错误信息包含上下文和建议
- [ ] `cargo run -- --help` 完整展示
- [ ] 不需要手动编辑 SQLite 数据库即可完成全部操作
