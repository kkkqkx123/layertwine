//! 逆向 Delta 引擎
//!
//! 生成可撤销一个 Delta 操作的逆向 Delta。
//! 参考 Immer 的 inversePatches 设计。

use crate::core::delta::{Delta, LineDiff};
use crate::core::types::{DiffOp, Hunk};
use crate::error::{Result, StratumError};

/// 生成 Delta 的逆向操作
///
/// 对于 Insert → Delete，Delete → Insert，Replace → 内容互换。
/// 由于 `Delete` 操作不保存被删除的行内容，需要提供 `old_content`，
/// 即产生该 Delta 时的旧文本，用于提取被删除的行。
///
/// 如果 `old_content` 为 None，Delete 的逆向 Insert 将包含空字符串。
pub fn inverse_delta(delta: &Delta, old_content: Option<&str>) -> Result<Delta> {
    let old_lines: Vec<&str> = old_content
        .map(|c| c.lines().collect())
        .unwrap_or_default();

    let mut inv_hunks = Vec::new();

    for hunk in &delta.diff.hunks {
        let mut inv_ops = Vec::new();

        for op in &hunk.ops {
            match op {
                DiffOp::Equal { count } => {
                    inv_ops.push(DiffOp::Equal { count: *count });
                }
                DiffOp::Insert {
                    new_start,
                    lines,
                } => {
                    // Insert 的逆是 Delete（从新文本中删除这些行）
                    inv_ops.push(DiffOp::Delete {
                        old_start: *new_start,
                        count: lines.len() as u32,
                    });
                }
                DiffOp::Delete {
                    old_start,
                    count,
                } => {
                    // Delete 的逆是 Insert（需要知道删了什么）
                    let deleted_lines: Vec<String> = if !old_lines.is_empty() {
                        let start = (*old_start as usize).saturating_sub(1);
                        let end = (start + *count as usize).min(old_lines.len());
                        old_lines[start..end]
                            .iter()
                            .map(|s| s.to_string())
                            .collect()
                    } else {
                        // 没有旧文本信息，生成占位
                        (0..*count).map(|_| String::new()).collect()
                    };
                    inv_ops.push(DiffOp::Insert {
                        new_start: *old_start,
                        lines: deleted_lines,
                    });
                }
                DiffOp::Replace {
                    old_start,
                    old_count,
                    new_start,
                    lines,
                } => {
                    // Replace 的逆是逆向 Replace
                    // 需要知道原来被替换掉的内容
                    let original_lines: Vec<String> = if !old_lines.is_empty() {
                        let start = (*old_start as usize).saturating_sub(1);
                        let end = (start + *old_count as usize).min(old_lines.len());
                        old_lines[start..end]
                            .iter()
                            .map(|s| s.to_string())
                            .collect()
                    } else {
                        lines.iter().map(|_| String::new()).collect()
                    };
                    inv_ops.push(DiffOp::Replace {
                        old_start: *new_start,
                        old_count: lines.len() as u32,
                        new_start: *old_start,
                        lines: original_lines,
                    });
                }
            }
        }

        inv_hunks.push(Hunk {
            old_start: hunk.new_start,
            old_len: hunk.new_len,
            new_start: hunk.old_start,
            new_len: hunk.old_len,
            ops: inv_ops,
        });
    }

    let inv_diff = LineDiff::new(inv_hunks);
    Ok(Delta::new(
        delta.file.clone(),
        inv_diff,
        delta.source.clone(),
    ))
}

/// 为 Snapshot 的增量链生成逆向 Delta 列表
///
/// 需要提供构建 snapshot 时各阶段对应的旧内容。
/// 从最新到最旧生成逆向 Delta，使得按顺序应用后回到初始状态。
pub fn inverse_snapshot(
    _snapshot: &crate::core::snapshot::Snapshot,
    _contents: &[&str],
) -> Result<Vec<Delta>> {
    // 此函数需要从存储层读取内容来构建逆向链
    // 当前返回空列表（占位），实际使用需与存储层集成
    Err(StratumError::Engine(
        "inverse_snapshot 尚未完整实现：需要存储层提供各版本内容".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::LineDiff;
    use crate::core::file_node::FileNode;
    use crate::core::types::{DiffOp, Hunk, SourceType};
    use std::path::PathBuf;

    #[test]
    fn test_inverse_insert_becomes_delete() {
        // Insert "new_line" → Delete "new_line"
        let hunk = Hunk {
            old_start: 1,
            old_len: 1,
            new_start: 1,
            new_len: 2,
            ops: vec![
                DiffOp::Equal { count: 1 },
                DiffOp::Insert {
                    new_start: 2,
                    lines: vec!["new_line".to_string()],
                },
            ],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b"content"),
            diff,
            SourceType::Manual,
        );
        let inv = inverse_delta(&delta, None).unwrap();

        // 逆 delta 应该有一个 Delete 操作
        let has_delete = inv.diff.hunks.iter().any(|h| {
            h.ops
                .iter()
                .any(|op| matches!(op, DiffOp::Delete { .. }))
        });
        assert!(has_delete);
    }

    #[test]
    fn test_inverse_delete_requires_content() {
        // Delete 需要 old_content 来知道删除了什么
        let hunk = Hunk {
            old_start: 1,
            old_len: 1,
            new_start: 1,
            new_len: 0,
            ops: vec![DiffOp::Delete {
                old_start: 1,
                count: 1,
            }],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b"content"),
            diff,
            SourceType::Manual,
        );

        // 没有 old_content，会生成空行
        let inv = inverse_delta(&delta, None).unwrap();
        let has_insert = inv.diff.hunks.iter().any(|h| {
            h.ops
                .iter()
                .any(|op| matches!(op, DiffOp::Insert { .. }))
        });
        assert!(has_insert);

        // 有 old_content，应该能提取被删除的行
        let inv2 = inverse_delta(&delta, Some("deleted_line\n")).unwrap();
        let insert_lines: Vec<&str> = inv2
            .diff
            .hunks
            .iter()
            .flat_map(|h| &h.ops)
            .filter_map(|op| {
                if let DiffOp::Insert { lines, .. } = op {
                    Some(lines.iter().map(|s| s.as_str()).collect::<Vec<_>>())
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        assert_eq!(insert_lines, vec!["deleted_line"]);
    }

    #[test]
    fn test_inverse_delta_roundtrip() {
        // Insert → Delete 应用后回到原始
        let content = "line1\nline3\n";
        let file = FileNode::new(PathBuf::from("test.txt"), b"");

        // 创建 insert delta: 在 line1 后插入 line2
        let hunk = Hunk {
            old_start: 2,
            old_len: 0,
            new_start: 2,
            new_len: 1,
            ops: vec![DiffOp::Insert {
                new_start: 2,
                lines: vec!["line2".to_string()],
            }],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(file.clone(), diff, SourceType::Manual);

        // 应用 insert → 得到 line1\nline2\nline3
        let new_content = crate::engine::merge::apply_deltas(content, &[delta.clone()]).unwrap();
        assert_eq!(new_content, "line1\nline2\nline3");

        // 生成逆 delta 并应用到新内容 → 回到原始
        let inv = inverse_delta(&delta, Some(content)).unwrap();
        let restored = crate::engine::merge::apply_deltas(&new_content, &[inv]).unwrap();
        assert_eq!(restored, content.trim_end_matches('\n'));
    }
}
