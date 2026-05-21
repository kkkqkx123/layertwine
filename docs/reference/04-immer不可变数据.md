# 4. Immer — JS 不可变数据与补丁系统参考

> 源码路径：`ref/immer-11.1.8/`
> 核心文件：`src/immer.ts`, `src/core/immerClass.ts`, `src/core/proxy.ts`, `src/plugins/patches.ts`

## 4.1 设计哲学

Immer 的核心思想：**通过 Proxy 代理，让开发者以"可变"的方式操作不可变数据**。

### produce 流程

```typescript
const nextState = produce(baseState, (draft) => {
    draft.user.name = "Alice"      // 看起来是可变操作
    draft.items.push(newItem)       // 看起来是可变操作
})
// 实际上 draft 是 Proxy，记录所有变更，生成新的不可变对象
```

## 4.2 Immer 的核心实现

### 4.2.1 代理机制

```typescript
// proxy.ts — createProxyProxy
function createProxyProxy<T>(base: T, parent?: ImmerState) {
    const state = {
        type_: isArray(base) ? ArchType.Array : ArchType.Object,
        scope_: parent ? parent.scope_ : getCurrentScope(),
        modified_: false,           // 是否修改
        finalized_: false,
        base_: base,                // 原始对象（只读）
        copy_: null,                // 修改后的副本（懒创建）
        draft_: null,               // proxy 自身
        assigned_: undefined,       // 赋值跟踪 Map
        parent_: parent,            // 父级状态（用于嵌套）
        revoke_: null,
    }
    
    const {revoke, proxy} = Proxy.revocable(target, traps)
    state.draft_ = proxy
    return [proxy, state]
}
```

**核心设计模式**：柯里化状态保存。
- Proxy 的 target 就是 `state` 自身（array 模式用 `[state]` 包裹一层）
- 所有 trap（get/set/deleteProperty）从 target 读取实际状态
- 避免为每个代理创建闭包，提高性能

### 4.2.2 懒复制 — COW 实现

```typescript
// 关键优化：只有在真正修改时才创建副本
function prepareCopy(state) {
    if (!state.copy_) {
        state.assigned_ = new Map()
        state.copy_ = shallowCopy(state.base_, state.scope_.immer_.useStrictShallowCopy_)
    }
}

// 读操作：从未修改时从 base_ 读取，修改后从 copy_ 读取
function latest(state) {
    return state.copy_ || state.base_
}

// 写操作触发 markChanged — 冒泡标记所有祖先
function markChanged(state) {
    if (!state.modified_) {
        state.modified_ = true
        if (state.parent_) {
            markChanged(state.parent_)  // 冒泡标记父级
        }
    }
}
```

**本项目对应**：这个模式与本项目的 `copy_on_write` 完全一致！
```
Immer:  base_ (原始) ↔ copy_ (修改后副本)
本项目: old_snapshot (不可变) ↔ current_partition (可变副本)
```

### 4.2.3 嵌套代理

当访问嵌套对象时，Immer **递归创建代理**（仅在必要时）：

```typescript
// get trap 中惰性地创建子代理
if (value === peek(state.base_, prop)) {
    prepareCopy(state)
    const childDraft = createProxy(state.scope_, value, state, childKey)
    return (state.copy_![childKey] = childDraft)
}
```

### 4.2.4 finalize — 最终化

```typescript
// finalize 递归处理所有子代理
function processResult(result, scope) {
    // 应用所有修改
    // 冻结结果（如果 autoFreeze 开启）
    // 生成 patches（如果启用 patches 插件）
    // 撤销所有 proxy
}
```

## 4.3 Patches 系统

Immer 的 patches 插件实现了 **双向补丁**（patches + inversePatches），这是本项目状态机回退功能的核心参考。

### 4.3.1 Patch 格式

```typescript
// patches.ts
export interface Patch {
    op: "replace" | "add" | "remove"
    path: (string | number)[]     // JSON path
    value?: any
}
```

示例：
```typescript
// 原始: {a: 1, b: [1, 2, 3]}
// 操作: draft.b.push(4)
// 产生的 patches:
[
    { op: "add", path: ["b", 3], value: 4 }
]
// 产生的 inversePatches:
[
    { op: "remove", path: ["b", 3] }  // 可精确回退
]
```

### 4.3.2 补丁生成

```typescript
// generatePatches_ — 根据 state 的修改生成 patches
function generatePatches_(state, basePath, scope) {
    switch (state.type_) {
        case ArchType.Object:
        case ArchType.Map:
            return generatePatchesFromAssigned(state, basePath, patches_, inversePatches_)
        case ArchType.Array:
            return generateArrayPatches(state, basePath, patches_, inversePatches_)
        case ArchType.Set:
            return generateSetPatches(state, basePath, patches_, inversePatches_)
    }
}

// generatePatchesFromAssigned — 对象/Map 补丁
function generatePatchesFromAssigned(state, basePath, patches, inversePatches) {
    // 遍历 assigned_ 中的所有键
    // 对比 base_[key] 和 copy_[key]
    // 新增: op: "add"
    // 删除: op: "remove"
    // 修改: op: "replace"
}
```

### 4.3.3 补丁应用

```typescript
// applyPatches_ — 将 patches 应用到 draft
function applyPatches_<T>(draft: T, patches: readonly Patch[]): T {
    for (const patch of patches) {
        const {op, path, value} = patch
        const parentPath = path.slice(0, -1)
        const key = path[path.length - 1]
        
        switch (op) {
            case "replace":  // parent[key] = value
            case "add":      // if array → splice, else → parent[key] = value
            case "remove":   // if array → splice, else → delete parent[key]
        }
    }
}
```

## 4.4 本项目适配 — 从 JS Patches 到 Rust Delta

### 4.4.1 Patch 格式映射

```typescript
// Immer Patch (JS)
{ op: "replace" | "add" | "remove", path: (string | number)[], value: any }

// 本项目 Delta (Rust)
pub enum HunkOp {
    Insert { start_line: usize, lines: Vec<String> },
    Delete { start_line: usize, count: usize },
    Replace { start_line: usize, old_count: usize, new_lines: Vec<String> },
}

pub struct LineDiff {
    pub hunks: Vec<Hunk>,
}
```

### 4.4.2 逆向补丁（inversePatches）— 状态回退

Immer 的 inversePatches 是**回滚操作**的核心参考：

```typescript
// patches:    [["add",    ["b", 3], 4]]
// inverse:    [["remove", ["b", 3]]]

// 本项目回滚：
// forward delta:   Insert { start: 10, lines: ["new line"] }
// inverse delta:   Delete { start: 10, count: 1 }
```

### 4.4.3 produce → transition 映射

```
Immer: produce(base, recipe) → nextState + patches + inversePatches
本项目: Layer::transition(Snapshot, EditDelta) → NewSnapshot + forward + inverse
```

### 4.4.4 COW 策略对比

| 特性 | Immer | 本项目 |
|------|-------|--------|
| 触发 COW | 首次 `draft.x = y` | 首次 `edit_file` |
| 副本粒度 | 对象级浅拷贝 | Snapshot 级（全量+Delta） |
| 追踪方式 | Proxy trap | 显式 Delta 记录 |
| 嵌套处理 | 递归代理 | 文件级（展开 | 平铺 |

## 4.5 关键技术提炼

### 4.5.1 修改追踪模式

Immer 用 `assigned_` Map 追踪每个属性的变更：
```
assigned_.has(prop) → 该属性被操作过
assigned_.get(prop) → true=赋值, false=删除
```

本项目通过 `Delta` 结构显式追踪修改，**不需要** assigned Map。

### 4.5.2 数组补丁 — 阵列操作

Immer 的 `generateArrayPatches` 处理 `push/pop/splice/sort/reverse`：
- 计算数组长度变化
- 对比每个索引上的值
- 检测 `shift` / `unshift` 导致的偏移

本项目不需要数组操作，只处理**行级文本**，复杂度较低。

### 4.5.3 补丁路径与 JSON Patch

Immer 使用 JSON Patch (RFC 6902) 风格的路径：
```
path: ["users", 0, "name"]
```

本项目使用行号代替路径：
```
hunk: { old_start: 42, old_lines: 3, new_lines: ["line1", "line2"] }
```

## 4.6 与项目架构的关联

```
┌─────────────┐    ┌──────────────────┐
│  Immer      │    │  本项目           │
│             │    │                  │
│ produce(    │    │ Layer::apply(   │
│   base,     │    │   snapshot,     │
│   recipe)   │    │   manual_edit   │
│ ) → next    │    │ ) → new_snap    │
│   + patches │    │   + Delta       │
│   + inverse │    │   + inverse     │
└─────────────┘    └──────────────────┘

逆向补丁的应用场景：
1. 用户取消编辑 → apply(inverse_patches)
2. Agent 回退到上一步 → 重置 Snapshot 指针
3. 冲突回滚 → 使用 inverse 恢复到冲突前状态
```
