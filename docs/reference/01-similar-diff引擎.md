# 1. similar — Rust Diff 引擎参考分析

> 源码路径：`ref/similar-main/`
> 版本：main 分支

## 1.1 总体架构

similar 是一个纯 Rust 的差异比较引擎，不依赖外部 C 库。设计初期受 Pijul 的 diff 库启发。

```
┌─────────────────────────────────────────────┐
│                 similar crate                │
│                                             │
│  ┌─────────────┐  ┌──────────────────────┐  │
│  │  algorithms  │  │  text (TextDiff)     │  │
│  │  ─低阶算法   │  │  ─高阶文本diff API   │  │
│  │             │  │                      │  │
│  │  Myers      │  │  TextDiff            │  │
│  │  Patience   │  │    from_lines        │  │
│  │  Histogram  │  │    from_words        │  │
│  │  Hunt       │  │    from_chars        │  │
│  │  LCS        │  │    diff_slices       │  │
│  └──────┬──────┘  └──────────┬───────────┘  │
│         │                    │               │
│         └─────────┬──────────┘               │
│                   ▼                          │
│  ┌────────────────────────────┐              │
│  │     types  (DiffOp)        │              │
│  │     common (ChangeTag)     │              │
│  └────────────────────────────┘              │
└─────────────────────────────────────────────┘
```

## 1.2 核心数据结构

### DiffOp — diff 操作抽象

```rust
pub enum DiffOp {
    Equal { old_index, new_index, len },
    Delete { old_index, old_len, new_index },
    Insert { old_index, new_index, new_len },
    Replace { old_index, old_len, new_index, new_len },
}
```

这是 **整个 diff 引擎的通用输出格式**。无论使用何种算法，最终结果都是 `Vec<DiffOp>`。

### ChangeTag — 变更类型标签

```rust
pub enum ChangeTag {
    Equal,
    Delete,
    Insert,
}
```

### Change — 带值的变更项

```rust
pub struct Change<T> {
    pub tag: ChangeTag,
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    pub value: T,
}
```

## 1.3 Diff 算法总览

| 算法 | 特点 | 适用场景 | 时间复杂度 |
|------|------|---------|-----------|
| **Myers** (默认) | 最短编辑脚本，通用性好 | 大多数场景 | O(ND) |
| **Patience** | 锚定唯一行，生成可读diff | 重构、代码移动 | O(N log N) |
| **Histogram** | 优先低频锚点 | 日志、重复行多的输入 | O(N log N) |
| **Hunt** | LCS 锚定链 | 匹配对稀疏时 | 可变 |
| **LCS** | 经典 LCS 算法 | 小输入、调试 | O(N×M) |

## 1.4 本项目适配方案

### 从 TextDiff 到 Delta

```rust
// similar 的输出格式
pub struct LineDiff {
    pub hunks: Vec<Hunk>,
}

pub struct Hunk {
    pub old_start: usize,    // 对应 DiffOp::old_index
    pub old_lines: usize,    // 对应 Equal::len 或 Delete::old_len
    pub new_start: usize,    // 对应 DiffOp::new_index
    pub new_lines: usize,    // 对应 Insert::new_len 或 Replace::new_len
    pub lines: Vec<LineChange>,
}

pub enum LineChange {
    Equal(String),
    Delete(String),
    Insert(String),
}

// 转换逻辑
fn diff_to_hunks(diff: &TextDiff) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    for group in diff.grouped_ops(3) {
        if group.is_empty() { continue; }
        let first = group.first().unwrap();
        let last = group.last().unwrap();
        hunks.push(Hunk {
            old_start: first.old_range().start,
            old_lines: last.old_range().end - first.old_range().start,
            new_start: first.new_range().start,
            new_lines: last.new_range().end - first.new_range().start,
            lines: group.iter().flat_map(|op| {
                diff.iter_changes(op).map(|change| match change.tag() {
                    ChangeTag::Equal => LineChange::Equal(change.value().to_string()),
                    ChangeTag::Delete => LineChange::Delete(change.value().to_string()),
                    ChangeTag::Insert => LineChange::Insert(change.value().to_string()),
                })
            }).collect(),
        });
    }
    hunks
}
```

### 可复用函数

| similar API | 本项目用途 |
|-------------|-----------|
| `TextDiff::from_lines` | 行级 diff 生成 `Delta.LineDiff` |
| `TextDiff::from_words` | 字词级 diff（用于 inline 变更展示） |
| `TextDiff::unified_diff` | 统一 diff 格式输出（用于 Git 同步） |
| `DiffOp::old_range / new_range` | 行号映射（old ↔ new） |
| `capture_diff_slices` | 非文本序列的差异比较 |

## 1.5 关键技术细节

### Deadline 机制

similar 支持设置 deadline，当 diff 超过时限会 fallback 到简化结果：
```rust
let mut config = TextDiff::configure();
config.timeout(Duration::from_secs(1));
let diff = config.diff_lines(&old, &new);
```

**本项目建议**：在大文件 diff 时设置 deadline，避免阻塞 Agent 操作。

### 启发式优化

- **前缀/后缀裁剪**：匹配的头尾立即发出，只 diff 变更的中间区域
- **不相交范围快速路径**：两个范围无共同元素时跳过搜索
- **Myers 前锚分离**：对不平衡偏移做前锚剥离

### 文件大小的行数阈值参考

| 文件大小 | 建议算法 | 是否设 deadline |
|---------|---------|----------------|
| < 1000 行 | Myers | 否 |
| 1000-10000 行 | Histogram | 建议 500ms |
| > 10000 行 | Histogram | 建议 2s |

## 1.6 与项目架构的关联

```
Snapshot 构造流程：
old_content ──┐
              ├── TextDiff::from_lines → Vec<DiffOp> → Delta (LineDiff)
new_content ──┘

merge 冲突检测：
base ──┐
       ├── TextDiff 比对 → 三路合并
ours ──┤
       └── ...
theirs ─┘
```
