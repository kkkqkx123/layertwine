# Stratum gRPC API 参考

> Stratum 的 gRPC API 以 Protobuf 定义为核心，通过 Tonic 框架提供。当前 proto 定义已完备，tonic-build 代码生成和处理器实现处于开发中。

## 当前状态

gRPC 服务端 **尚未完整实现**。调用 `rpc::serve()` 会返回 `"gRPC server requires tonic-build codegen"` 错误。

- **Proto 文件**: `src/api/rpc/proto/stratum.proto` — 已完备
- **Rust 类型映射**: `src/api/types.rs` 中定义了与 proto 消息 1:1 对应的请求/响应类型
- **服务逻辑**: `ApiService` trait（`src/api/service.rs`）已实现所有业务逻辑
- **待完成**: `tonic-build` 编译 proto 并实现 `StratumGrpc` 的 gRPC handler

启用需在 `build.rs` 中加入 tonic-build 编译：

```rust
fn main() {
    tonic_build::compile_protos("src/api/rpc/proto/stratum.proto")
        .expect("failed to compile protos");
}
```

并在 `Cargo.toml` 中启用 `grpc` feature 和 `tonic-build` build dependency。

---

## 环境变量

| 变量               | 默认值               | 说明                           |
| ------------------ | -------------------- | ------------------------------ |
| `STRATUM_MODE`     | `cli`                | 设为 `grpc` 启动 gRPC 服务     |
| `STRATUM_DB_PATH`  | `.stratum/stratum.db` | SQLite 数据库文件路径          |
| `STRATUM_GRPC_ADDR`| `127.0.0.1:50051`    | gRPC 服务器绑定地址与端口       |

---

## Proto 文件

```protobuf
syntax = "proto3";

package stratum;

service Stratum {
    // Repository lifecycle
    rpc Init(InitRequest) returns (InitResponse);
    rpc Status(Empty) returns (StatusResponse);

    // Edit operations
    rpc Edit(EditRequest) returns (EditResponse);
    rpc AgentEdit(AgentEditRequest) returns (EditResponse);
    rpc AgentSubmit(AgentSubmitRequest) returns (SubmitResponse);
    rpc Approve(ApproveRequest) returns (ApproveResponse);

    // Checkpoint operations
    rpc Commit(CommitRequest) returns (CommitResponse);
    rpc Log(LogRequest) returns (LogResponse);

    // Branch operations
    rpc BranchCreate(BranchCreateRequest) returns (BranchCreateResponse);
    rpc BranchSwitch(BranchSwitchRequest) returns (BranchSwitchResponse);
    rpc BranchList(Empty) returns (BranchListResponse);
    rpc Merge(MergeRequest) returns (MergeResponse);

    // Backup operations
    rpc Backup(BackupRequest) returns (BackupResponse);
    rpc Restore(RestoreRequest) returns (RestoreResponse);

    // Maintenance
    rpc Gc(Empty) returns (GcResponse);
    rpc Push(PushRequest) returns (PushResponse);
    rpc Pull(PullRequest) returns (PullResponse);
}
```

---

## 服务方法参考

### RPC 列表

| RPC 方法        | 请求类型               | 响应类型                | 说明                     |
| --------------- | ---------------------- | ----------------------- | ------------------------ |
| `Init`          | `InitRequest`          | `InitResponse`          | 初始化仓库               |
| `Status`        | `Empty`                | `StatusResponse`        | 查看状态                 |
| `Edit`          | `EditRequest`          | `EditResponse`          | 手动编辑                 |
| `AgentEdit`     | `AgentEditRequest`     | `EditResponse`          | Agent 编辑               |
| `AgentSubmit`   | `AgentSubmitRequest`   | `SubmitResponse`        | Agent 提交审核           |
| `Approve`       | `ApproveRequest`       | `ApproveResponse`       | 审核通过                 |
| `Commit`        | `CommitRequest`        | `CommitResponse`        | 提交检查点               |
| `Log`           | `LogRequest`           | `LogResponse`           | 查看提交历史             |
| `BranchCreate`  | `BranchCreateRequest`  | `BranchCreateResponse`  | 创建分支                 |
| `BranchSwitch`  | `BranchSwitchRequest`  | `BranchSwitchResponse`  | 切换分支                 |
| `BranchList`    | `Empty`                | `BranchListResponse`    | 列出所有分支             |
| `Merge`         | `MergeRequest`         | `MergeResponse`         | 合并分支                 |
| `Backup`        | `BackupRequest`        | `BackupResponse`        | 备份快照                 |
| `Restore`       | `RestoreRequest`       | `RestoreResponse`       | 从备份恢复               |
| `Gc`            | `Empty`                | `GcResponse`            | 垃圾回收                 |
| `Push`          | `PushRequest`          | `PushResponse`          | 推送到 Git               |
| `Pull`          | `PullRequest`          | `PullResponse`          | 从 Git 拉取              |

---

## 消息类型定义

### 通用类型

```protobuf
message Empty {}
```

所有无参数的 RPC 使用 `Empty` 作为请求类型。

---

### Repository Lifecycle

#### Init — 初始化仓库

```protobuf
message InitRequest {
    optional string db_path = 1;
    optional string git_repo = 2;
    optional string git_ref = 3;
}

message InitResponse {
    string db_path = 1;
    string manual_partition_id = 2;
    string staged_partition_id = 3;
    string branch = 4;
}
```

| 字段                    | 类型           | 说明                                |
| ----------------------- | -------------- | ----------------------------------- |
| `InitRequest.db_path`   | string (可选)  | 数据库路径，默认 `.stratum/stratum.db` |
| `InitRequest.git_repo`  | string (可选)  | Git 仓库路径                        |
| `InitRequest.git_ref`   | string (可选)  | 从 Git 初始化时的引用               |
| `InitResponse.branch`   | string         | 当前分支名，默认 `"main"`           |

#### Status — 查看状态

```protobuf
message StatusResponse {
    repeated PartitionInfo partitions = 1;
}

message PartitionInfo {
    string layer = 1;
    string name = 2;
    string current_snapshot = 3;
    uint32 history_len = 4;
}
```

| 字段                        | 类型     | 说明                                        |
| --------------------------- | -------- | ------------------------------------------- |
| `PartitionInfo.layer`       | string   | 层标识：`manual_edit` / `agent_edit` / `staged` |
| `PartitionInfo.current_snapshot` | string   | 当前快照 ID（十六进制）                     |
| `PartitionInfo.history_len` | uint32   | 历史 Delta 数量                             |

---

### Edit Operations

#### Edit / AgentEdit — 编辑文件

```protobuf
message EditRequest {
    string file = 1;
    optional string content = 2;
}

message AgentEditRequest {
    string agent_id = 1;
    string file = 2;
    optional string content = 3;
}

message EditResponse {
    string snapshot_id = 1;
    optional string staged_snapshot_id = 2;
}
```

| 字段                                | 类型             | 说明                                  |
| ----------------------------------- | ---------------- | ------------------------------------- |
| `AgentEditRequest.agent_id`         | string           | Agent 实例 ID                         |
| `EditResponse.snapshot_id`          | string           | 本次编辑生成的快照 ID                 |
| `EditResponse.staged_snapshot_id`   | string (可选)    | 自动合并到 staged 后的快照 ID          |

#### AgentSubmit — Agent 提交审核

```protobuf
message AgentSubmitRequest {
    string agent_id = 1;
}

message SubmitResponse {
    string snapshot_id = 1;
}
```

#### Approve — 审核通过

```protobuf
message ApproveRequest {
    string agent_id = 1;
}

message ApproveResponse {
    string integrated_snapshot_id = 1;
    string staged_snapshot_id = 2;
}
```

---

### Checkpoint Operations

#### Commit — 提交检查点

```protobuf
message CommitRequest {
    string message = 1;
    optional string author = 2;
}

message CommitResponse {
    string checkpoint_id = 1;
    string message = 2;
}
```

#### Log — 查看提交历史

```protobuf
message LogRequest {
    optional uint32 count = 1;
}

message LogResponse {
    repeated CheckpointInfo checkpoints = 1;
    uint32 total = 2;
}

message CheckpointInfo {
    string id = 1;
    string author = 2;
    string message = 3;
    repeated string parents = 4;
    repeated string snapshots = 5;
    int64 created_at = 6;
    optional string git_anchor = 7;
}
```

| 字段                         | 类型          | 说明                        |
| ---------------------------- | ------------- | --------------------------- |
| `LogRequest.count`           | uint32 (可选)  | 最大返回数量，默认 20       |
| `CheckpointInfo.parents`     | string[]      | 父检查点 ID 列表            |
| `CheckpointInfo.snapshots`   | string[]      | 基线快照 ID 列表            |
| `CheckpointInfo.created_at`  | int64         | Unix 时间戳                 |
| `CheckpointInfo.git_anchor`  | string (可选)  | Git 锚点（从 Git 同步时存在） |

---

### Branch Operations

```protobuf
message BranchCreateRequest {
    string name = 1;
}

message BranchCreateResponse {
    string name = 1;
    string head = 2;
}

message BranchSwitchRequest {
    string name = 1;
}

message BranchSwitchResponse {
    string name = 1;
    string checkpoint_id = 2;
}

message BranchListResponse {
    repeated BranchInfo branches = 1;
    optional string current = 2;
}

message BranchInfo {
    string name = 1;
    string head = 2;
    string updated_at = 3;
    bool is_current = 4;
}

message MergeRequest {
    string branch = 1;
    optional string message = 2;
}

message MergeResponse {
    string checkpoint_id = 1;
    string source_branch = 2;
    string target_branch = 3;
}
```

---

### Backup Operations

```protobuf
message BackupRequest {
    string snapshot_id = 1;
    optional string label = 2;
}

message BackupResponse {
    string backup_id = 1;
    string source_snapshot_id = 2;
    optional string label = 3;
}

message RestoreRequest {
    string backup_id = 1;
}

message RestoreResponse {
    string backup_id = 1;
    string file = 2;
    uint32 deltas_restored = 3;
}
```

---

### Maintenance

```protobuf
message GcResponse {
    uint32 removed_checkpoints = 1;
    uint32 removed_snapshots = 2;
    uint64 freed_bytes = 3;
    bool delta_chain_depth_triggered = 4;
}

message PushRequest {
    optional string remote = 1;
    string git_repo = 2;
    optional string message = 3;
}

message PushResponse {
    string remote = 1;
    string git_commit_hash = 2;
}

message PullRequest {
    optional string remote = 1;
    string git_repo = 2;
    optional string git_ref = 3;
}

message PullResponse {
    string remote = 1;
    string git_ref = 2;
}
```

| 字段         | 类型          | 说明                                 |
| ------------ | ------------- | ------------------------------------ |
| `PushRequest.git_repo`  | string | Git 仓库路径（必填）                  |
| `PullRequest.git_repo`  | string | Git 仓库路径（必填）                  |
| `GcResponse.freed_bytes`| uint64 | 释放的字节数                          |
| `GcResponse.delta_chain_depth_triggered` | bool | 是否触发 Delta 链深度压缩 |

---

## 实现架构

gRPC 层与 HTTP 层共享 `ApiService` trait 作为业务逻辑入口：

```
gRPC Client
    │
    ▼
tonic Server (StratumGrpc)
    │  ┌─ Init()        → service.init(InitRequest)
    │  ├─ Status()      → service.status()
    │  ├─ Edit()        → service.edit(EditRequest)
    │  ├─ AgentEdit()   → service.agent_edit(AgentEditRequest)
    │  ├─ AgentSubmit() → service.agent_submit(AgentSubmitRequest)
    │  ├─ Approve()     → service.approve(ApproveRequest)
    │  ├─ Commit()      → service.commit(CommitRequest)
    │  ├─ Log()         → service.log(LogRequest)
    │  ├─ BranchCreate()→ service.branch_create(BranchCreateRequest)
    │  ├─ BranchSwitch()→ service.branch_switch(BranchSwitchRequest)
    │  ├─ BranchList()  → service.branch_list()
    │  ├─ Merge()       → service.merge(MergeRequest)
    │  ├─ Backup()      → service.backup(BackupRequest)
    │  ├─ Restore()     → service.restore(RestoreRequest)
    │  ├─ Gc()          → service.gc(GcRequest)
    │  ├─ Push()        → service.push(PushRequest)
    │  └─ Pull()        → service.pull(PullRequest)
    ▼
ApiServiceImpl → StateMachine → SqliteStorage
```

每个 RPC handler 的处理模式：
1. 从 proto 请求反序列化
2. 转换为 `api::types::*Request`
3. 调用 `ApiService` 对应方法
4. 将 `api::types::*Response` 转换为 proto 响应
5. 错误映射为 `tonic::Status`

---

## 与 HTTP API 的对应关系

gRPC 服务和 HTTP 端点共享相同的业务逻辑，以下为对应关系：

| gRPC RPC        | HTTP 端点                                      |
| --------------- | ---------------------------------------------- |
| `Init`          | `POST /api/v1/init`                            |
| `Status`        | `GET  /api/v1/status`                          |
| `Edit`          | `POST /api/v1/edit`                            |
| `AgentEdit`     | `POST /api/v1/agent/{id}/edit`                 |
| `AgentSubmit`   | `POST /api/v1/agent/{id}/submit`               |
| `Approve`       | `POST /api/v1/approve/{agent_id}`              |
| `Commit`        | `POST /api/v1/commit`                          |
| `Log`           | `GET  /api/v1/log`                             |
| `BranchCreate`  | `POST /api/v1/branches`                        |
| `BranchSwitch`  | `POST /api/v1/branches/{name}/switch`          |
| `BranchList`    | `GET  /api/v1/branches`                        |
| `Merge`         | `POST /api/v1/merge`                           |
| `Backup`        | `POST /api/v1/backup`                          |
| `Restore`       | `POST /api/v1/restore`                         |
| `Gc`            | `POST /api/v1/gc`                              |
| `Push`          | `POST /api/v1/push`                            |
| `Pull`          | `POST /api/v1/pull`                            |
