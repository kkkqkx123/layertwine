# API 模块设计方案

## 1. 现状分析

### 1.1 当前架构

Stratum 目前是一个 **纯 Rust 库 crate**（`lib.rs`），没有二进制入口（无 `main.rs` 或 `[[bin]]` 目标）。所有用户交互通过 `cli/` 模块暴露：

```
stratum::cli::run()   →   命令解析 (clap)   →   执行器   →   println!/eprintln!
```

### 1.2 接口层现状

| 层面 | 实现 | 问题 |
|------|------|------|
| CLI | `cli/mod.rs` + `commands.rs` + `output.rs` | 输出绑定 `println!`，嵌入方无法捕获结构化结果 |
| 库 API | `state_machine::StateMachine` + `storage::repository::*` traits | 粒度太细，嵌入方需了解内部模块分工 |
| 结构化输出 | `OutputFormat::Json` 枚举 | 仅在 CLI 层存在，非一等公民 |
| 错误返回 | `Result<T, StratumError>` | 良好，但 CLI 层将错误转为 `eprintln!` 丢失类型信息 |

### 1.3 关键发现

**当前 CLI 的耦合问题：**

1. `cli/mod.rs` 中的执行器函数（`execute_edit`、`execute_commit` 等）**直接调用底层模块**，而不是通过统一服务接口
2. 格式化输出（`output.rs`）与执行器混合在同一个模块，传输层逻辑与业务逻辑未分离
3. `println!` 直接写 stdout，嵌入方无法拦截或重定向（除非替换全局 stdout）
4. 无 `async` 支持，当前所有操作为同步

**嵌入场景缺失：**

- 其他 Rust 应用若需嵌入式 Stratum，必须直接依赖 `state_machine` + `storage` 等内部模块
- 非 Rust 应用（Python Agent 框架、TypeScript 前端）完全无法访问
- 分布式场景（多 Agent 在不同进程中运行）缺乏远程调用能力

---

## 2. 需求评估

### 2.1 是否需要 HTTP/gRPC 服务

| 场景 | 需求等级 | 说明 |
|------|---------|------|
| Rust 库嵌入（同一进程） | **P0** | 当前无统一服务接口，嵌入方被迫依赖内部模块 |
| 跨语言调用（Python Agent 等） | **P1** | 多 Agent 协同是核心场景，Agent 可能用 Python 编写 |
| 远程分布式部署 | **P2** | Agent 分布在多台机器时需网络通信 |
| IDE/Web 前端集成 | **P2** | 可能需要 WebSocket 实时推送状态变更 |
| CI/CD 工具链集成 | **P1** | 通过 HTTP API 进行自动化操作 |

### 2.2 条件编译的必要性

| 特性 | 依赖膨胀 | 编译时间影响 | 嵌入式场景是否需要 |
|------|---------|-------------|------------------|
| `clap` + CLI | ~100kB | 低 | 部分场景需要（独立工具） |
| `axum` + `tokio` (HTTP) | ~500kB+ | 中 | 跨语言/远程场景需要 |
| `tonic` + `prost` (gRPC) | ~1MB+ | 高 | 高性能远程调用需要 |
| `git2` | libgit2 C 库 | 中 | 当前已存在，作为公共依赖 |

**结论：条件编译（Cargo features）是必需的**。纯库嵌入场景不应被迫编译 HTTP/gRPC 依赖。

---

## 3. 设计目标

1. **统一服务接口**：定义 `ApiService` trait 作为所有操作的单一入口，CLI/HTTP/gRPC 共用
2. **传输层与业务层分离**：`api/` 模块定义接口和数据结构，三种传输实现放在子模块中
3. **条件编译**：通过 Cargo features 控制各传输实现的编译，默认仅启用 CLI
4. **结构化输出**：所有 API 返回 `Result<ApiResponse, ApiError>`，不再依赖 `println!`
5. **异步优先**：`ApiService` trait 使用 `async fn`（Rust 1.88 内置 async trait 支持）
6. **向前兼容**：现有 CLI 行为不变，内部重构为调用 `ApiService`

---

## 4. 模块架构

```
src/
├── api/                          # [新增] API 统一模块
│   ├── mod.rs                    #     ApiService trait、公共类型
│   ├── service.rs                #     ApiServiceImpl (默认实现)
│   ├── types.rs                  #     ApiRequest / ApiResponse / ApiError
│   │
│   ├── cli/                      # [重构] CLI 传输层（依赖 clap）
│   │   ├── mod.rs                #     命令解析 + 路由到 ApiService
│   │   ├── commands.rs           #     从旧 cli/commands.rs 迁移
│   │   └── output.rs             #     从旧 cli/output.rs 迁移（仅 Plain 格式）
│   │
│   ├── http/                     # [新增] HTTP 传输层（feature = "http"）
│   │   ├── mod.rs                #     axum 路由定义 + 服务器启动
│   │   └── routes.rs             #     各端点处理函数
│   │
│   └── rpc/                      # [新增] gRPC 传输层（feature = "grpc"）
│       ├── mod.rs                #     tonic 服务定义 + 服务器启动
│       └── proto/                #     .proto 文件（编译产物）
│
├── cli/                          # [废弃] 旧模块 → 保留为薄封装调用 api::cli
│   └── mod.rs                    #     pub fn run() → api::cli::run()
│
└── ...其余模块不变...
```

### 4.1 模块职责

| 模块 | 职责 | 条件编译 feature |
|------|------|-----------------|
| `api::mod` | 导出 `ApiService` trait、`ApiServiceImpl`、公共类型 | 始终编译 |
| `api::service` | `ApiServiceImpl` — 调用 `state_machine` + `storage` 实现业务逻辑 | 始终编译 |
| `api::types` | `ApiRequest`、`ApiResponse`、`ApiError` 等公共数据结构 | 始终编译 |
| `api::cli` | CLI 传输 — clap 解析 → 调用 `ApiService` | `default`（始终编译） |
| `api::http` | HTTP 传输 — axum 路由 → 调用 `ApiService` | `feature = "http"` |
| `api::rpc` | gRPC 传输 — tonic 服务 → 调用 `ApiService` | `feature = "grpc"` |

---

## 5. ApiService Trait 设计

### 5.1 核心接口

```rust
/// API 统一服务接口 — 所有业务操作的单一入口
///
/// CLI、HTTP、gRPC 三种传输层均通过此 trait 调用业务逻辑。
/// 使用 Rust 1.88 内置 async trait 支持，无需 async-trait crate。
#[async_trait]
pub trait ApiService: Send + Sync {
    // ── 仓库生命周期 ──

    /// 初始化 Stratum 仓库
    async fn init(&self, req: InitRequest) -> Result<InitResponse>;

    /// 查看当前状态
    async fn status(&self) -> Result<StatusResponse>;

    // ── 编辑操作 ──

    /// 手动编辑文件（manual_edit 层）
    async fn edit(&self, req: EditRequest) -> Result<EditResponse>;

    /// Agent 编辑文件（agent_edit 层）
    async fn agent_edit(&self, req: AgentEditRequest) -> Result<EditResponse>;

    /// Agent 提交审核
    async fn agent_submit(&self, req: AgentSubmitRequest) -> Result<SubmitResponse>;

    /// 审核通过 Agent 修改
    async fn approve(&self, req: ApproveRequest) -> Result<ApproveResponse>;

    // ── 检查点操作 ──

    /// 提交检查点
    async fn commit(&self, req: CommitRequest) -> Result<CommitResponse>;

    /// 查看提交历史
    async fn log(&self, req: LogRequest) -> Result<LogResponse>;

    // ── 分支操作 ──

    /// 创建分支
    async fn branch_create(&self, req: BranchCreateRequest) -> Result<BranchCreateResponse>;

    /// 切换分支
    async fn branch_switch(&self, req: BranchSwitchRequest) -> Result<BranchSwitchResponse>;

    /// 列出分支
    async fn branch_list(&self) -> Result<BranchListResponse>;

    /// 合并分支
    async fn merge(&self, req: MergeRequest) -> Result<MergeResponse>;

    // ── 备份操作 ──

    /// 备份快照
    async fn backup(&self, req: BackupRequest) -> Result<BackupResponse>;

    /// 从备份恢复
    async fn restore(&self, req: RestoreRequest) -> Result<RestoreResponse>;

    // ── 维护操作 ──

    /// 垃圾回收
    async fn gc(&self) -> Result<GcResponse>;

    /// Git 推送
    async fn push(&self, req: PushRequest) -> Result<PushResponse>;

    /// Git 拉取
    async fn pull(&self, req: PullRequest) -> Result<PullResponse>;
}
```

### 5.2 请求/响应类型

所有请求和响应均为具名结构体，便于序列化（`serde::Serialize`/`Deserialize`）和版本演进：

```rust
// ── 请求类型 ──

#[derive(Debug, Serialize, Deserialize)]
pub struct InitRequest {
    pub db_path: Option<String>,      // 默认 ".stratum/stratum.db"
    pub git_repo: Option<String>,     // 可选的 Git 仓库路径
    pub git_ref: Option<String>,      // Git 引用（如 "HEAD"）
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EditRequest {
    pub file: String,                 // 文件路径
    pub content: Option<String>,      // 新文件内容
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentEditRequest {
    pub agent_id: String,
    pub file: String,
    pub content: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentSubmitRequest {
    pub agent_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApproveRequest {
    pub agent_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommitRequest {
    pub message: String,
    pub author: Option<String>,       // 默认 "user"
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogRequest {
    pub count: Option<usize>,         // 默认 20
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BranchCreateRequest {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BranchSwitchRequest {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MergeRequest {
    pub branch: String,
    pub message: Option<String>,      // 默认 "merge"
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupRequest {
    pub snapshot_id: String,          // 十六进制
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RestoreRequest {
    pub backup_id: String,            // 十六进制
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PushRequest {
    pub remote: Option<String>,       // 默认 "origin"
    pub message: Option<String>,      // 默认 "sync from stratum"
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PullRequest {
    pub remote: Option<String>,       // 默认 "origin"
    pub git_ref: Option<String>,      // 默认 "HEAD"
}


// ── 响应类型 ──

#[derive(Debug, Serialize, Deserialize)]
pub struct InitResponse {
    pub db_path: String,
    pub manual_partition_id: String,
    pub staged_partition_id: String,
    pub branch: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub partitions: Vec<PartitionInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PartitionInfo {
    pub layer: String,
    pub name: String,
    pub current_snapshot: String,     // 十六进制前缀
    pub history_len: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EditResponse {
    pub snapshot_id: String,          // 十六进制
    pub staged_snapshot_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitResponse {
    pub snapshot_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApproveResponse {
    pub integrated_snapshot_id: String,
    pub staged_snapshot_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommitResponse {
    pub checkpoint_id: String,        // 十六进制
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogResponse {
    pub checkpoints: Vec<CheckpointInfo>,
    pub total: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CheckpointInfo {
    pub id: String,
    pub author: String,
    pub message: String,
    pub parents: Vec<String>,
    pub snapshots: Vec<String>,
    pub created_at: i64,
    pub git_anchor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BranchCreateResponse {
    pub name: String,
    pub head: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BranchSwitchResponse {
    pub name: String,
    pub checkpoint_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BranchListResponse {
    pub branches: Vec<BranchInfo>,
    pub current: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub head: String,
    pub updated_at: String,
    pub is_current: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MergeResponse {
    pub checkpoint_id: String,
    pub source_branch: String,
    pub target_branch: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupResponse {
    pub backup_id: String,
    pub source_snapshot_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RestoreResponse {
    pub backup_id: String,
    pub file: String,
    pub deltas_restored: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GcResponse {
    pub removed_checkpoints: usize,
    pub removed_snapshots: usize,
    pub freed_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PushResponse {
    pub remote: String,
    pub git_commit_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PullResponse {
    pub remote: String,
    pub git_ref: String,
}
```

### 5.3 API 错误类型

```rust
/// API 层错误 — 结构化的、可序列化的错误信息
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,               // 机器可读的错误码
    pub message: String,            // 人类可读的描述
    pub suggestion: Option<String>, // 修复建议
    pub details: Option<Value>,     // 可选的额外上下文
}

impl ApiError {
    // 工厂方法
    pub fn not_found(entity: &str) -> Self;
    pub fn invalid_params(msg: &str) -> Self;
    pub fn storage_error(msg: &str) -> Self;
    pub fn engine_error(msg: &str) -> Self;
    pub fn internal(msg: &str) -> Self;
}

/// API 操作结果
pub type ApiResult<T> = std::result::Result<T, ApiError>;
```

---

## 6. ApiServiceImpl 默认实现

```rust
/// ApiService 的默认实现 — 包装 StateMachine + Storage
pub struct ApiServiceImpl {
    state_machine: StateMachine,
    storage: Arc<SqliteStorage>,
    db_path: String,
    git_repo: Option<String>,
}

impl ApiServiceImpl {
    /// 创建并初始化（或打开）Stratum 仓库
    pub async fn new(config: ServiceConfig) -> ApiResult<Self>;

    /// 打开已有仓库
    pub async fn open(config: ServiceConfig) -> ApiResult<Self>;
}

/// 服务配置
pub struct ServiceConfig {
    pub db_path: String,
    pub git_repo: Option<String>,
}

#[async_trait]
impl ApiService for ApiServiceImpl {
    async fn edit(&self, req: EditRequest) -> ApiResult<EditResponse> {
        // 1. 调用 state_machine::manual::apply_manual_edit()
        // 2. 调用 state_machine::manual::merge_manual_to_staged()
        // 3. 返回结构化 EditResponse
        // 不再 println!，不再处理输出格式
    }

    // ...其他方法类似...
}
```

这一层是 **所有传输方式的唯一业务实现**。CLI、HTTP、gRPC 都是此实现的薄包装。

---

## 7. 传输层设计

### 7.1 CLI 传输层（始终编译）

从当前的 `cli/` 模块重构而来，移除业务逻辑，仅保留 clap 命令定义和输出格式化：

```rust
// src/api/cli/mod.rs
#[cfg(feature = "cli")]
pub mod cli {
    use super::*;

    /// CLI 入口 — 解析参数、调用 ApiService、格式化输出
    pub async fn run() -> i32 {
        let args = commands::parse_args();
        let service = ApiServiceImpl::open(ServiceConfig {
            db_path: args.db_path,
            git_repo: args.git_repo,
        }).await;
        // 匹配子命令 → 调用 service.xxx() → 输出格式化
    }
}
```

**旧 `cli/mod.rs` 保持向后兼容：**

```rust
// src/cli/mod.rs — 保留为薄转发层
pub fn run() -> i32 {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(api::cli::run())
}
```

### 7.2 HTTP 传输层（feature = "http"）

```rust
// src/api/http/mod.rs
#[cfg(feature = "http")]
pub mod http {
    use super::*;

    /// 启动 HTTP 服务器
    pub async fn serve(service: Arc<dyn ApiService>, addr: SocketAddr) -> ApiResult<()> {
        let app = Router::new()
            .route("/api/v1/init", post(handle_init))
            .route("/api/v1/status", get(handle_status))
            .route("/api/v1/edit", post(handle_edit))
            .route("/api/v1/agent/:id/edit", post(handle_agent_edit))
            .route("/api/v1/agent/:id/submit", post(handle_agent_submit))
            .route("/api/v1/approve/:agent_id", post(handle_approve))
            .route("/api/v1/commit", post(handle_commit))
            .route("/api/v1/log", get(handle_log))
            .route("/api/v1/branches", get(handle_branch_list))
            .route("/api/v1/branches", post(handle_branch_create))
            .route("/api/v1/branches/:name/switch", post(handle_branch_switch))
            .route("/api/v1/merge", post(handle_merge))
            .route("/api/v1/backup", post(handle_backup))
            .route("/api/v1/restore", post(handle_restore))
            .route("/api/v1/gc", post(handle_gc))
            .route("/api/v1/push", post(handle_push))
            .route("/api/v1/pull", post(handle_pull))
            .with_state(service);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }
}
```

**API 端点设计：**

| 方法 | 路径 | 请求体 | 说明 |
|------|------|--------|------|
| POST | `/api/v1/init` | `InitRequest` | 初始化仓库 |
| GET | `/api/v1/status` | — | 当前状态 |
| POST | `/api/v1/edit` | `EditRequest` | 手动编辑 |
| POST | `/api/v1/agent/{id}/edit` | `EditRequest` | Agent 编辑 |
| POST | `/api/v1/agent/{id}/submit` | — | Agent 提交审核 |
| POST | `/api/v1/approve/{agent_id}` | — | 审核通过 |
| POST | `/api/v1/commit` | `CommitRequest` | 提交检查点 |
| GET | `/api/v1/log?count=20` | — | 提交历史 |
| GET | `/api/v1/branches` | — | 分支列表 |
| POST | `/api/v1/branches` | `BranchCreateRequest` | 创建分支 |
| POST | `/api/v1/branches/{name}/switch` | — | 切换分支 |
| POST | `/api/v1/merge` | `MergeRequest` | 合并分支 |
| POST | `/api/v1/backup` | `BackupRequest` | 备份快照 |
| POST | `/api/v1/restore` | `RestoreRequest` | 恢复备份 |
| POST | `/api/v1/gc` | — | 垃圾回收 |
| POST | `/api/v1/push` | `PushRequest` | Git 推送 |
| POST | `/api/v1/pull` | `PullRequest` | Git 拉取 |

所有响应统一包装：

```rust
#[derive(Serialize)]
struct ApiEnvelope<T: Serialize> {
    success: bool,
    data: Option<T>,
    error: Option<ApiError>,
}
```

### 7.3 gRPC 传输层（feature = "grpc"）

```protobuf
// proto/stratum.proto

service Stratum {
    rpc Init(InitRequest) returns (InitResponse);
    rpc Status(Empty) returns (StatusResponse);
    rpc Edit(EditRequest) returns (EditResponse);
    rpc AgentEdit(AgentEditRequest) returns (EditResponse);
    rpc AgentSubmit(AgentSubmitRequest) returns (SubmitResponse);
    rpc Approve(ApproveRequest) returns (ApproveResponse);
    rpc Commit(CommitRequest) returns (CommitResponse);
    rpc Log(LogRequest) returns (LogResponse);
    rpc BranchCreate(BranchCreateRequest) returns (BranchCreateResponse);
    rpc BranchSwitch(BranchSwitchRequest) returns (BranchSwitchResponse);
    rpc BranchList(Empty) returns (BranchListResponse);
    rpc Merge(MergeRequest) returns (MergeResponse);
    rpc Backup(BackupRequest) returns (BackupResponse);
    rpc Restore(RestoreRequest) returns (RestoreResponse);
    rpc Gc(Empty) returns (GcResponse);
    rpc Push(PushRequest) returns (PushResponse);
    rpc Pull(PullRequest) returns (PullResponse);
}
```

gRPC 的 tonic 服务实现将 Protobuf 消息类型映射到 `ApiService` 的 Rust 类型：

```rust
#[cfg(feature = "grpc")]
pub mod rpc {
    use super::*;

    pub struct StratumGrpcService {
        api: Arc<dyn ApiService>,
    }

    #[tonic::async_trait]
    impl stratum_proto::stratum_server::Stratum for StratumGrpcService {
        async fn edit(
            &self,
            request: tonic::Request<stratum_proto::EditRequest>,
        ) -> tonic::Response<stratum_proto::EditResponse> {
            let req = EditRequest::from_proto(request.into_inner());
            match self.api.edit(req).await {
                Ok(resp) => tonic::Response::new(resp.into_proto()),
                Err(e) => tonic::Response::with_status(/* 映射错误码 */),
            }
        }
        // ...
    }

    /// 启动 gRPC 服务器
    pub async fn serve(service: Arc<dyn ApiService>, addr: SocketAddr) -> ApiResult<()> {
        // tonic transport + reflection
    }
}
```

---

## 8. Cargo Features 设计

### 8.1 Feature 定义

```toml
[features]
default = ["cli"]

# API 传输层
cli = ["dep:clap"]                          # CLI 模式（默认）
http = ["dep:axum", "dep:tokio"]            # HTTP REST 服务
grpc = ["dep:tonic", "dep:prost", "dep:tokio"]  # gRPC 服务

# 便捷组合
api-full = ["cli", "http", "grpc"]          # 所有传输方式

# 核心依赖（始终编译的核心库不需要 feature gate）
# rusqlite, blake3, serde, etc. 始终存在
```

### 8.2 依赖条件化

```toml
[dependencies]
# 核心依赖 — 始终编译
rusqlite = { version = "0.39", features = ["bundled"] }
blake3 = "1.8"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
chrono = "0.4"
uuid = { version = "1", features = ["v4", "v7", "serde"] }
similar = "3"
git2 = "0.21"

# API 传输层依赖 — 条件编译
clap = { version = "4", features = ["derive"], optional = true }
tokio = { version = "1", features = ["full"], optional = true }
axum = { version = "0.7", optional = true }
tonic = { version = "0.12", optional = true }
prost = { version = "0.13", optional = true }
```

### 8.3 条件编译使用模式

```rust
// src/api/mod.rs

// 始终编译 — ApiService trait + ApiServiceImpl + types
pub mod service;
pub mod types;

mod private {
    pub use super::service::ApiServiceImpl;
    pub use super::types::*;
}

#[cfg(feature = "cli")]
pub mod cli;

#[cfg(feature = "http")]
pub mod http;

#[cfg(feature = "grpc")]
pub mod rpc;
```

### 8.4 使用示例

```bash
# 最小化嵌入（仅核心库，无 CLI）
cargo build --no-default-features

# 默认（带 CLI）
cargo build

# CLI + HTTP
cargo build --features http

# 全功能
cargo build --features api-full
```

```rust
// 嵌入方使用示例

// 1. 纯库嵌入 — 无任何传输层
// Cargo.toml: stratum = { path = "../stratum", default-features = false }
use stratum::api::{ApiService, ApiServiceImpl, ServiceConfig};

let service = ApiServiceImpl::open(ServiceConfig {
    db_path: ".stratum/stratum.db".into(),
    git_repo: None,
}).await?;

let resp = service.commit(CommitRequest {
    message: "自动提交".into(),
    author: Some("ci-bot".into()),
}).await?;

// 2. HTTP 服务嵌入
// Cargo.toml: stratum = { path = "../stratum", features = ["http"] }
use stratum::api::http;

let service = Arc::new(service);
http::serve(service, "127.0.0.1:8080".parse().unwrap()).await?;
```

---

## 9. 迁移路径

### 9.1 分步实施

| 阶段 | 内容 | 影响 |
|------|------|------|
| **M1** | 创建 `api/` 模块，定义 `ApiService` trait + 请求/响应类型 | 新文件，无破坏性变更 |
| **M2** | 实现 `ApiServiceImpl` — 将 `cli/mod.rs` 中的执行器函数迁移过来 | 内部重构，对外无影响 |
| **M3** | 改写 `cli/mod.rs` 为 `ApiService` 调用的薄封装 | 行为不变，代码结构变化 |
| **M4** | 添加条件编译 feature flags（将 clap 标记为 optional） | 仓库配置变更 |
| **M5** | 实现 HTTP 传输层（feature = "http"） | 新增功能 |
| **M6** | 实现 gRPC 传输层（feature = "grpc"） | 新增功能 |
| **M7** | 移除旧 `cli/` 模块（或标记 deprecated） | 潜在破坏性变更 |

### 9.2 兼容性保证

- M1-M3：完全向后兼容，旧 `stratum::cli::run()` 继续可用
- M4：`default = ["cli"]` 确保 `cargo build` 行为不变
- M5-M6：纯新功能，不影响现有用户

---

## 10. 与设计方案中的技术栈对照

| 设计方案中声明的技术栈 | 当前状态 | api 模块方案 |
|-----------------------|---------|-------------|
| `tokio` | 未使用（声明但未引入） | M2 引入，用于 `ApiService` async trait |
| `clap` v4 | 已使用（作为必需依赖） | M4 改为 optional |
| `tracing` | 声明的日志框架 | 新增，用于 HTTP/gRPC 请求追踪 |
| 异步 trait | 声明 Rust 1.88 内置支持 | M2 真正利用 |

---

## 11. 开放性讨论

### 11.1 HTTP vs gRPC 优先级

**建议优先实现 HTTP（REST/JSON）**，理由：
- 对非 Rust 调用方最友好（Python `requests`、JS `fetch` 即可调用）
- 调试方便（curl、浏览器）
- 与当前 CLI 的 `--json` 输出模式精神一致
- gRPC 的 proto 编译流程增加构建复杂度

gRPC 适用于：
- 高性能、低延迟的 Agent ↔ Stratum 通信
- 需要双向流推送状态变更的场景
- 已有 Protobuf 基础设施的团队

### 11.2 二进制入口

当前无 `main.rs`，API 模块完成后可选择性添加：

```rust
// src/bin/stratum.rs（新文件）
#[cfg(feature = "cli")]
fn main() {
    stratum::cli::run();
}

// src/bin/stratumd.rs（新文件）
#[cfg(feature = "http")]
#[tokio::main]
async fn main() {
    // 解析 --port、--db 等参数
    // 启动 HTTP 服务器
}
```

### 11.3 认证与安全

HTTP/gRPC 接口当前设计为**本地/内网服务**，本方案不涉及认证。生产环境建议：
- Unix socket 绑定（避免 TCP 端口暴露）
- 反向代理 + TLS（nginx/caddy）
- API 密钥验证（如有跨网络需求）

### 11.4 多实例

当前 `ApiServiceImpl` 持有 `StateMachine`，后者持有 `Arc<SqliteStorage>`。SQLite 支持 WAL 模式的多读单写，但 HTTP/gRPC 的多线程并发访问需要关注：
- HTTP 场景：每个请求新建 `ApiServiceImpl` 或使用连接池管理
- gRPC 场景：tonic 默认多线程，需确保 `SqliteStorage` 线程安全（当前实现 `rusqlite` 的 `Connection` 非 `Send`，已通过 `with_conn` 模式封装）

---

## 12. 总结

| 维度 | 结论 |
|------|------|
| 是否需要 HTTP/gRPC | **是** — P1 需求（跨语言调用），P2 需求（远程部署） |
| 是否需条件编译 | **是** — 依赖膨胀显著，嵌入式场景不应被迫引入 |
| 是否创建 api 模块 | **是** — 统一接口 + 三种传输实现 + 条件编译 |
| 迁移成本 | **低** — M1-M3 为纯新增代码，M4 调整 Cargo.toml |
| 向前兼容 | **完全保证** — 旧 `stratum::cli::run()` 继续可用 |