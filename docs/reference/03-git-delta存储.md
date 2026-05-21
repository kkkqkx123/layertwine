# 3. Git 对象存储与 Delta 压缩参考

> 源码路径：`ref/git-2.54.0/`
> 核心文件：`delta.h`, `diff-delta.c`, `packfile.c`, `pack-objects.c`, `Documentation/gitformat-pack.adoc`, `Documentation/gitformat-commit-graph.adoc`

## 3.1 Git Delta 压缩格式

Git 的 delta 压缩是一种 **二进制 diff 格式**，用于在 pack 文件中存储对象的增量版本。

### 3.1.1 Delta 数据格式

```
┌─────────────────────────────────┐
│   源对象大小 (变长整数)          │
├─────────────────────────────────┤
│   目标对象大小 (变长整数)          │
├─────────────────────────────────┤
│   指令序列 (copy/insert)         │
│   ┌───────────┐                 │
│   │ Copy 指令  │ (1xxxxxxx)     │
│   │   从源拷贝字节范围           │
│   ├───────────┤                 │
│   │ Insert 指令│ (0xxxxxxx)     │
│   │   内嵌新数据                │
│   └───────────┘                 │
└─────────────────────────────────┘
```

**Copy 指令** (`1xxxxxxx`)：
- 高位置 1 表示 copy
- 剩余 7 bit 标记 offset 和 size 各字段是否存在
- offset: 4 bytes (小端序，可选)
- size: 3 bytes (小端序，可选，0 → 65536)

**Insert 指令** (`0xxxxxxx`)：
- 高位置 0 表示 insert
- 剩余 7 bit 表示内嵌数据长度
- 后续直接跟 `length` 字节的数据

### 3.1.2 Delta 索引结构

```c
struct delta_index {
    unsigned long memsize;
    const void *src_buf;
    unsigned long src_size;
    unsigned int hash_mask;
    struct index_entry *hash[FLEX_ARRAY];
};
```

`diff-delta.c` 的算法：
1. 对源缓冲区的 **每个 16 字节对齐位置** 计算哈希
2. 构建哈希表，冲突链解决
3. 对目标缓冲区，滑动窗口查找匹配
4. 对匹配不到的部分输出 Insert 指令

**本项目参考**：Git 的 delta 是**二进制字节级**，我们的 Delta 是**行级**，更高级。但 Git 的 delta 指令格式（copy + insert）的设计思想可以指导 `LineDiff` 的 hunk 组织。

## 3.2 Pack 文件格式

### 3.2.1 整体结构

```
pack 文件:
  ┌─────────────────┐
  │ Header          │ 12 bytes (signature, version, object count)
  ├─────────────────┤
  │ Object Entry 1  │
  │ Object Entry 2  │
  │ ...             │
  │ Object Entry N  │
  ├─────────────────┤
  │ Trailer         │ 20/32 bytes (pack checksum)
  └─────────────────┘

Object Entry (未增量):
  ┌─────────────────┐
  │ Type + Size     │ 变长
  │ Compressed Data │ zlib
  └─────────────────┘

Object Entry (增量, OFS_DELTA):
  ┌─────────────────┐
  │ Type + Size     │ 变长 (type=6)
  │ Base 偏移       │ 变长 (从当前对象的负偏移)
  │ Compressed Delta│ zlib 压缩的 delta 数据
  └─────────────────┘

Object Entry (增量, REF_DELTA):
  ┌─────────────────┐
  │ Type + Size     │ 变长 (type=7)
  │ Base 对象名     │ 20/32 bytes SHA
  │ Compressed Delta│ zlib 压缩的 delta 数据
  └─────────────────┘
```

### 3.2.2 变长整数编码

```c
// 每个字节的低 7 位是有效数据，高 1 位表示是否继续
// (与 protobuf 的 varint 编码相同)
static inline unsigned long get_delta_hdr_size(const unsigned char **datap,
                                               const unsigned char *top) {
    const unsigned char *data = *datap;
    size_t cmd, size = 0;
    int i = 0;
    do {
        cmd = *data++;
        size |= (cmd & 0x7f) << i;
        i += 7;
    } while (cmd & 0x80 && data < top);
    *datap = data;
    return size;
}
```

### 3.2.3 对象类型

| 类型值 | 名称 | 说明 |
|-------|------|------|
| 1 | OBJ_COMMIT | Commit 对象 |
| 2 | OBJ_TREE | Tree 对象 |
| 3 | OBJ_BLOB | Blob（文件内容） |
| 4 | OBJ_TAG | Tag 对象 |
| 6 | OBJ_OFS_DELTA | 基于偏移的增量 |
| 7 | OBJ_REF_DELTA | 基于引用的增量 |

## 3.3 本项目适配方案

### 3.3.1 行级 Delta 存储

Git 的 delta 是**二进制字节级** → 本项目使用**行级 delta**，行级相比字节级有以下优势：

| 特性 | Git 二进制 Delta | 本项目行级 Delta |
|------|-----------------|-----------------|
| 粒度 | 字节 | 行 |
| 可读性 | 不可读 | 可读、可调试 |
| 合并复杂度 | 高（需三路合并） | 低（行级三路合并） |
| 存储效率 | 高 | 较高（相近） |
| diff 算法 | 自研滑动窗口 | similar crate |

### 3.3.2 增量链与 GC 参考

Git 的 pack 文件将多个对象打包，形成增量链：
```
Commit A (base) → Commit B (delta) → Commit C (delta)
                                      ↑ 引用链最大 250 层
```

**本项目 GC**：
```
Snapshot A (full) → Snapshot B (delta) → Snapshot C (delta)
                                           ↑ 建议最大 100 层
                                           ↑ 超限时触发全量重打包
```

### 3.3.3 Pack 文件概念的简化替代

Git 使用 `.idx` 文件做对象索引 → 本项目直接用 SQLite B-tree 替代：

```
Git: 对象名 → pack + offset   (O(1) 哈希)
本项目: Blake3 hash → SQLite 行 (O(log N) B-tree)
```

## 3.4 Commit-Graph 格式参考

`gitformat-commit-graph.adoc` 定义了 commit 的增量可达性加速结构：

| 特性 | 本项目对应 |
|------|-----------|
| OID 查找表 | `CheckpointDag.nodes: HashMap<CheckpointId, Node>` |
| Generation number | `Node::gen_number: u32` |
| 父引用压缩 | `parents: Vec<CheckpointId>` |
| Bloom 过滤器 | 可选加速路径 |

## 3.5 关键代码索引

| Git 源码文件 | 功能 | 本项目参考价值 |
|-------------|------|--------------|
| `delta.h` | Delta 格式定义 + 变长整数 API | 高 — 指令格式设计 |
| `diff-delta.c` | Delta 计算算法 | 中 — 哈希索引 + 滑动窗口思路 |
| `packfile.c` | Pack 文件读写 | 低 — 直接使用 SQLite |
| `pack-objects.c` | 打包对象（增量选择） | 高 — 增量链深度控制 |
| `refs/` | 引用管理 | 低 — 直接使用 SQLite |

## 3.6 与项目架构的关系

```
Snapshot ↔ Git Pack 概念映射：

Delta (行级) ↔ Git OBJ_OFS_DELTA (但行级 vs 字节级)
Snapshot ↔ Git Object (Blob)
Checkpoint ↔ Git Commit
Branch ↔ Git Ref (HEAD / branches)

GC 流程：
1. 收集未引用的 Snapshot → Git 的 unreachable objects
2. 增量链深度检查 → Git 的 delta depth 限制
3. 重打包：全量 + 增量 → Git 的 repack
```
