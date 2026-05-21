//! Merge 引擎 — 三路合并 & Delta 应用
//!
//! 提供两个核心能力：
//! 1. apply_deltas: 从基准内容 + Delta 链重建完整文件内容
//! 2. merge_texts: 三路文本合并（含冲突检测）

use crate::core::delta::{Delta, LineDiff};
#[allow(unused_imports)]
use crate::core::types::{DiffOp, Hunk};
use crate::error::{Result, StratumError};

/// 从基准内容依次应用 Delta，重建完整文件内容
///
/// 每个 Delta 按内部 Hunks 定义的变换依次应用到当前内容。
/// Hunks 按 old_start 排序后顺序处理，每个 Hunk 从旧内容中
/// 定位 `old_start..old_start+old_len` 区域，然后执行：
/// - Equal: 保留对应行
/// - Delete: 跳过对应行
/// - Insert: 插入新行
/// - Replace: 跳过旧行并插入新行
pub fn apply_deltas(content: &str, deltas: &[Delta]) -> Result<String> {
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    for delta in deltas {
        lines = apply_line_diff(&lines, &delta.diff)?;
    }

    Ok(lines.join("\n"))
}

/// 应用单个 LineDiff 到行数组
fn apply_line_diff(lines: &[String], diff: &LineDiff) -> Result<Vec<String>> {
    let mut result: Vec<String> = Vec::with_capacity(lines.len());
    let mut old_pos = 0usize;

    // 按 old_start 排序
    let mut hunks = diff.hunks.clone();
    hunks.sort_by_key(|h| h.old_start);

    for hunk in &hunks {
        let hunk_start = (hunk.old_start.saturating_sub(1)) as usize;
        let hunk_end = hunk_start + hunk.old_len as usize;

        // 确保 hunk 位置不重叠且不越界
        if hunk_start < old_pos {
            return Err(StratumError::Engine(format!(
                "重叠的 hunk: old_start={}, 已处理到位置 {}",
                hunk.old_start, old_pos
            )));
        }
        if hunk_end > lines.len() {
            return Err(StratumError::Engine(format!(
                "Hunk 超出范围: old_start={}, old_len={}, 总行数={}",
                hunk.old_start,
                hunk.old_len,
                lines.len()
            )));
        }

        // 复制 hunk 前未变的部分
        if hunk_start > old_pos {
            result.extend_from_slice(&lines[old_pos..hunk_start]);
        }

        // 处理 hunk 内的操作
        let mut hunk_pos = hunk_start;
        for op in &hunk.ops {
            match op {
                DiffOp::Equal { count } => {
                    let c = *count as usize;
                    result.extend_from_slice(&lines[hunk_pos..hunk_pos + c]);
                    hunk_pos += c;
                }
                DiffOp::Delete { count, .. } => {
                    hunk_pos += *count as usize;
                }
                DiffOp::Insert { lines: new_lines, .. } => {
                    result.extend(new_lines.iter().cloned());
                }
                DiffOp::Replace {
                    old_count,
                    lines: new_lines,
                    ..
                } => {
                    hunk_pos += *old_count as usize;
                    result.extend(new_lines.iter().cloned());
                }
            }
        }

        old_pos = hunk_end;
    }

    // 剩余未变行
    if old_pos < lines.len() {
        result.extend_from_slice(&lines[old_pos..]);
    }

    Ok(result)
}

/// 合并冲突
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeConflict {
    /// 在最终输出中的起始行号（0-indexed）
    pub start_line: usize,
    /// 基准版本的内容
    pub base: Vec<String>,
    /// 我们的版本的内容
    pub ours: Vec<String>,
    /// 他们的版本的内容
    pub theirs: Vec<String>,
}

impl MergeConflict {
    /// 生成类似 Git 的冲突标记格式
    pub fn to_conflict_marker(&self) -> String {
        let mut buf = String::new();
        buf.push_str("<<<<<<< ours\n");
        for line in &self.ours {
            buf.push_str(line);
            buf.push('\n');
        }
        buf.push_str("=======\n");
        for line in &self.theirs {
            buf.push_str(line);
            buf.push('\n');
        }
        buf.push_str(">>>>>>> theirs\n");
        buf
    }
}

/// 三路文本合并
///
/// 基于 diff_to_line_diff 计算 base→ours 和 base→theirs 的差异，
/// 然后同步应用两个差异。当同一区域被两边以不同方式修改时产生冲突。
///
/// 返回合并后的文本和冲突列表（如有冲突）。
pub fn merge_texts(base: &str, ours: &str, theirs: &str) -> (String, Vec<MergeConflict>) {
    use crate::engine::diff::diff_to_line_diff;

    let diff_ours = diff_to_line_diff(base, ours);
    let diff_theirs = diff_to_line_diff(base, theirs);

    let base_lines: Vec<&str> = base.lines().collect();
    let ours_lines: Vec<&str> = ours.lines().collect();
    let theirs_lines: Vec<&str> = theirs.lines().collect();

    // 收集两边在 base 上的修改
    let mut our_changes: Vec<ChangeRange> = Vec::new();
    let mut their_changes: Vec<ChangeRange> = Vec::new();

    collect_changes_from_diff(&diff_ours, &ours_lines, &mut our_changes);
    collect_changes_from_diff(&diff_theirs, &theirs_lines, &mut their_changes);

    // 合并两边变更（使用类似 jj 的逐行标记法）
    let mut result: Vec<String> = Vec::new();
    let mut conflicts: Vec<MergeConflict> = Vec::new();
    let mut base_pos = 0usize;

    let mut our_idx = 0usize;
    let mut their_idx = 0usize;

    while our_idx < our_changes.len() || their_idx < their_changes.len() {
        // 取当前位置更早的变更
        let our_change = our_changes.get(our_idx);
        let their_change = their_changes.get(their_idx);

        match (our_change, their_change) {
            (None, Some(tc)) => {
                // 只有他们改了
                append_unchanged(&mut result, &base_lines, base_pos, tc.base_start);
                append_change(&mut result, tc);
                base_pos = tc.base_start + tc.base_len;
                their_idx += 1;
            }
            (Some(oc), None) => {
                // 只有我们改了
                append_unchanged(&mut result, &base_lines, base_pos, oc.base_start);
                append_change(&mut result, oc);
                base_pos = oc.base_start + oc.base_len;
                our_idx += 1;
            }
            (Some(oc), Some(tc)) => {
                if oc.base_start < tc.base_start {
                    // 我们的变更更早
                    append_unchanged(&mut result, &base_lines, base_pos, oc.base_start);
                    append_change(&mut result, oc);
                    base_pos = oc.base_start + oc.base_len;
                    our_idx += 1;
                } else if tc.base_start < oc.base_start {
                    // 他们的变更更早
                    append_unchanged(&mut result, &base_lines, base_pos, tc.base_start);
                    append_change(&mut result, tc);
                    base_pos = tc.base_start + tc.base_len;
                    their_idx += 1;
                } else {
                    // 两边从同一位置开始修改 — 检查是否冲突
                    let oc_end = oc.base_start + oc.base_len;
                    let tc_end = tc.base_start + tc.base_len;

                    if oc.base_len == tc.base_len && oc.new_lines == tc.new_lines {
                        // 两边做了相同的修改 — 只应用一次
                        append_unchanged(&mut result, &base_lines, base_pos, oc.base_start);
                        append_change(&mut result, oc);
                        base_pos = oc_end;
                    } else {
                        // 重叠或相同范围但不同内容 = 冲突
                        append_unchanged(&mut result, &base_lines, base_pos, oc.base_start);

                        let conflict_start_line = result.len();
                        // 输出 ours
                        for line in &oc.new_lines {
                            result.push(line.clone());
                        }
                        base_pos = oc_end.max(tc_end);

                        conflicts.push(MergeConflict {
                            start_line: conflict_start_line,
                            base: base_lines[oc.base_start..oc.base_start + oc.base_len]
                                .iter()
                                .map(|s| s.to_string())
                                .collect(),
                            ours: oc.new_lines.clone(),
                            theirs: tc.new_lines.clone(),
                        });
                    }
                    our_idx += 1;
                    their_idx += 1;
                }
            }
            (None, None) => break,
        }
    }

    // 剩余未变行
    if base_pos < base_lines.len() {
        for line in &base_lines[base_pos..] {
            result.push(line.to_string());
        }
    }

    (result.join("\n"), conflicts)
}

/// 变更范围：标记 base 中一段被替换为新内容
#[derive(Debug, Clone)]
struct ChangeRange {
    base_start: usize,
    base_len: usize,
    new_lines: Vec<String>,
}

/// 从 LineDiff（我们的内部类型）收集变更
///
/// 跳过 Equal 上下文，只提取实际的变更区域（Delete/Insert/Replace）。
/// 每个 hunk 可能产生多个 ChangeRange（Equal 分隔的多个变更区段）。
fn collect_changes_from_diff(
    diff: &LineDiff,
    _new_lines: &[&str],
    changes: &mut Vec<ChangeRange>,
) {
    for hunk in &diff.hunks {
        let has_change = hunk.ops.iter().any(|op| !matches!(op, DiffOp::Equal { .. }));
        if !has_change {
            continue;
        }

        let base_offset = (hunk.old_start.saturating_sub(1)) as usize;
        let mut old_cursor = 0usize;

        let mut current_base_start: Option<usize> = None;
        let mut current_base_len: usize = 0;
        let mut current_new_lines: Vec<String> = Vec::new();

        fn flush_change(
            changes: &mut Vec<ChangeRange>,
            _base_offset: usize,
            current_base_start: &mut Option<usize>,
            current_base_len: &mut usize,
            current_new_lines: &mut Vec<String>,
        ) {
            if let Some(start) = *current_base_start {
                changes.push(ChangeRange {
                    base_start: start,
                    base_len: *current_base_len,
                    new_lines: std::mem::take(current_new_lines),
                });
            }
            *current_base_start = None;
            *current_base_len = 0;
        }

        for op in &hunk.ops {
            match op {
                DiffOp::Equal { count } => {
                    let c = *count as usize;
                    flush_change(
                        changes,
                        base_offset,
                        &mut current_base_start,
                        &mut current_base_len,
                        &mut current_new_lines,
                    );
                    old_cursor += c;
                }
                DiffOp::Insert { lines, .. } => {
                    if current_base_start.is_none() {
                        current_base_start = Some(base_offset + old_cursor);
                    }
                    current_new_lines.extend(lines.iter().cloned());
                }
                DiffOp::Delete { count, .. } => {
                    if current_base_start.is_none() {
                        current_base_start = Some(base_offset + old_cursor);
                    }
                    let c = *count as usize;
                    current_base_len += c;
                    old_cursor += c;
                }
                DiffOp::Replace { old_count, lines, .. } => {
                    if current_base_start.is_none() {
                        current_base_start = Some(base_offset + old_cursor);
                    }
                    let oc = *old_count as usize;
                    current_base_len += oc;
                    current_new_lines.extend(lines.iter().cloned());
                    old_cursor += oc;
                }
            }
        }

        flush_change(
            changes,
            base_offset,
            &mut current_base_start,
            &mut current_base_len,
            &mut current_new_lines,
        );
    }
}
fn append_unchanged(
    result: &mut Vec<String>,
    base_lines: &[&str],
    from: usize,
    to: usize,
) {
    if to > from {
        for line in &base_lines[from..to.min(base_lines.len())] {
            result.push(line.to_string());
        }
    }
}

/// 追加一个变更的内容
fn append_change(result: &mut Vec<String>, change: &ChangeRange) {
    for line in &change.new_lines {
        result.push(line.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::LineDiff;
    use crate::core::file_node::FileNode;
    use crate::core::types::{DiffOp, Hunk, SourceType};
    use std::path::PathBuf;

    #[test]
    fn test_apply_empty_deltas() {
        let content = "hello\nworld\n";
        let result = apply_deltas(content, &[]).unwrap();
        assert_eq!(result, "hello\nworld");
    }

    #[test]
    fn test_apply_single_insert() {
        let content = "line1\nline3\n";
        let hunk = Hunk {
            old_start: 2,
            old_len: 1,
            new_start: 2,
            new_len: 2,
            ops: vec![
                DiffOp::Insert {
                    new_start: 2,
                    lines: vec!["line2".to_string()],
                },
                DiffOp::Equal { count: 1 }, // keep "line3"
            ],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b""),
            diff,
            SourceType::Manual,
        );
        let result = apply_deltas(content, &[delta]).unwrap();
        assert_eq!(result, "line1\nline2\nline3");
    }

    #[test]
    fn test_apply_delete() {
        let content = "line1\nline2\nline3\n";
        let hunk = Hunk {
            old_start: 2,
            old_len: 1,
            new_start: 2,
            new_len: 0,
            ops: vec![DiffOp::Delete {
                old_start: 2,
                count: 1,
            }],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b""),
            diff,
            SourceType::Manual,
        );
        let result = apply_deltas(content, &[delta]).unwrap();
        assert_eq!(result, "line1\nline3");
    }

    #[test]
    fn test_apply_replace() {
        let content = "aaa\nbbb\nccc\n";
        let hunk = Hunk {
            old_start: 2,
            old_len: 1,
            new_start: 2,
            new_len: 1,
            ops: vec![DiffOp::Replace {
                old_start: 2,
                old_count: 1,
                new_start: 2,
                lines: vec!["xxx".to_string()],
            }],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b""),
            diff,
            SourceType::Manual,
        );
        let result = apply_deltas(content, &[delta]).unwrap();
        assert_eq!(result, "aaa\nxxx\nccc");
    }

    #[test]
    fn test_apply_chain() {
        let content = "a\nb\nc\n";
        let hunk1 = Hunk {
            old_start: 2,
            old_len: 1,
            new_start: 2,
            new_len: 2,
            ops: vec![
                DiffOp::Insert {
                    new_start: 2,
                    lines: vec!["x".to_string()],
                },
                DiffOp::Equal { count: 1 }, // keep "b"
            ],
        };
        // 注意：插入后内容变为 a\nx\nb\nc\n
        // 再次修改行 3 (原 b)
        let hunk2 = Hunk {
            old_start: 3,
            old_len: 1,
            new_start: 3,
            new_len: 1,
            ops: vec![DiffOp::Replace {
                old_start: 3,
                old_count: 1,
                new_start: 3,
                lines: vec!["y".to_string()],
            }],
        };
        let diff1 = LineDiff::new(vec![hunk1]);
        let diff2 = LineDiff::new(vec![hunk2]);
        let delta1 = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b""),
            diff1,
            SourceType::Manual,
        );
        let delta2 = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b""),
            diff2,
            SourceType::Manual,
        );
        let result = apply_deltas(content, &[delta1, delta2]).unwrap();
        assert_eq!(result, "a\nx\ny\nc");
    }

    #[test]
    fn test_merge_identical() {
        let base = "a\nb\nc\n";
        let (merged, conflicts) = merge_texts(base, base, base);
        assert_eq!(merged, "a\nb\nc");
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_merge_non_conflicting() {
        let base = "a\nb\nc\n";
        let ours = "x\nb\nc\n";
        let theirs = "a\nb\ny\n";
        let (merged, conflicts) = merge_texts(base, ours, theirs);
        assert!(conflicts.is_empty());
        assert!(merged.contains('x'));
        assert!(merged.contains('y'));
    }

    #[test]
    fn test_merge_with_conflict() {
        let base = "a\nb\nc\n";
        let ours = "a\nX\nc\n";
        let theirs = "a\nY\nc\n";
        let (_, conflicts) = merge_texts(base, ours, theirs);
        assert!(!conflicts.is_empty());
        assert_eq!(conflicts[0].ours, vec!["X"]);
        assert_eq!(conflicts[0].theirs, vec!["Y"]);
    }

    #[test]
    fn test_merge_same_change() {
        let base = "a\nb\nc\n";
        let ours = "a\nX\nc\n";
        let theirs = "a\nX\nc\n";
        let (merged, conflicts) = merge_texts(base, ours, theirs);
        assert!(conflicts.is_empty());
        assert_eq!(merged, "a\nX\nc");
    }
}
