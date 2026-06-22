use layertwine::core::delta::Delta;
use layertwine::core::file_node::FileNode;
use layertwine::core::types::{DiffOp, Hunk, LineDiff, SourceType};
use layertwine::engine::diff::diff_to_line_diff;
use layertwine::engine::inverse::inverse_delta;
use std::path::PathBuf;

fn generate_test_text(lines: usize) -> String {
    (0..lines).map(|i| format!("line {}\n", i)).collect()
}

fn generate_modified_text(base: &str, change_rate: f64) -> String {
    let mut result = String::new();
    let mut rng = 0u64;

    for line in base.lines() {
        rng = rng.wrapping_mul(1103515245).wrapping_add(12345);
        let random = (rng % 100) as f64 / 100.0;

        if random < change_rate {
            result.push_str(&format!("MODIFIED: {}\n", line));
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

fn create_delta_from_texts(old: &str, new: &str) -> (Delta, String) {
    let file_node = FileNode::new(PathBuf::from("test.txt"), old.as_bytes());
    let diff = diff_to_line_diff(old, new);
    (
        Delta::new(file_node, diff, SourceType::Manual),
        old.to_string(),
    )
}

fn create_delta_with_only_inserts(old: &str, insert_lines: Vec<String>) -> (Delta, String) {
    let file_node = FileNode::new(PathBuf::from("test.txt"), old.as_bytes());
    let old_len = old.lines().count() as u32;

    let diff = LineDiff {
        hunks: vec![Hunk {
            old_start: old_len,
            old_len: 0,
            new_start: old_len,
            new_len: insert_lines.len() as u32,
            ops: vec![DiffOp::Insert {
                new_start: old_len,
                lines: insert_lines,
            }],
        }],
    };
    (
        Delta::new(file_node, diff, SourceType::Manual),
        old.to_string(),
    )
}

fn create_delta_with_only_deletes(old: &str, delete_count: u32) -> (Delta, String) {
    let file_node = FileNode::new(PathBuf::from("test.txt"), old.as_bytes());
    let diff = LineDiff {
        hunks: vec![Hunk {
            old_start: 1,
            old_len: delete_count,
            new_start: 1,
            new_len: 0,
            ops: vec![DiffOp::Delete {
                old_start: 1,
                count: delete_count,
            }],
        }],
    };
    (
        Delta::new(file_node, diff, SourceType::Manual),
        old.to_string(),
    )
}

fn create_delta_with_replaces(old: &str, replace_count: u32) -> (Delta, String) {
    let file_node = FileNode::new(PathBuf::from("test.txt"), old.as_bytes());

    let ops = (0..replace_count)
        .map(|i| DiffOp::Replace {
            old_start: i + 1,
            old_count: 1,
            new_start: i + 1,
            lines: vec![format!("REPLACED line {}", i + 1)],
        })
        .collect();

    let diff = LineDiff {
        hunks: vec![Hunk {
            old_start: 1,
            old_len: replace_count,
            new_start: 1,
            new_len: replace_count,
            ops,
        }],
    };
    (
        Delta::new(file_node, diff, SourceType::Manual),
        old.to_string(),
    )
}

fn benchmark_inverse_delta(
    c: &mut criterion::Criterion,
    name: &str,
    lines: usize,
    change_rate: f64,
) {
    let old_text = generate_test_text(lines);
    let new_text = generate_modified_text(&old_text, change_rate);
    let (delta, old_content) = create_delta_from_texts(&old_text, &new_text);

    c.bench_function(
        &format!(
            "inverse_delta_{}_{}_lines_{}_percent",
            name,
            lines,
            (change_rate * 100.0) as usize
        ),
        |b| b.iter(|| inverse_delta(&delta, Some(old_content.as_str()))),
    );
}

pub fn bench_inverse_delta_small(c: &mut criterion::Criterion) {
    benchmark_inverse_delta(c, "small", 10, 0.1);
    benchmark_inverse_delta(c, "small", 10, 0.3);
    benchmark_inverse_delta(c, "small", 10, 0.5);
}

pub fn bench_inverse_delta_medium(c: &mut criterion::Criterion) {
    benchmark_inverse_delta(c, "medium", 100, 0.1);
    benchmark_inverse_delta(c, "medium", 100, 0.3);
    benchmark_inverse_delta(c, "medium", 100, 0.5);
}

pub fn bench_inverse_delta_large(c: &mut criterion::Criterion) {
    benchmark_inverse_delta(c, "large", 1000, 0.1);
    benchmark_inverse_delta(c, "large", 1000, 0.3);
}

pub fn bench_inverse_operations(c: &mut criterion::Criterion) {
    let base_text = generate_test_text(100);

    let (delta_insert, old_insert) = create_delta_with_only_inserts(
        &base_text,
        (0..10).map(|i| format!("inserted line {}\n", i)).collect(),
    );
    c.bench_function("inverse_delta_insert_only_10_lines", |b| {
        b.iter(|| inverse_delta(&delta_insert, Some(old_insert.as_str())))
    });

    let (delta_delete, old_delete) = create_delta_with_only_deletes(&base_text, 10);
    c.bench_function("inverse_delta_delete_only_10_lines", |b| {
        b.iter(|| inverse_delta(&delta_delete, Some(old_delete.as_str())))
    });

    let (delta_replace, old_replace) = create_delta_with_replaces(&base_text, 10);
    c.bench_function("inverse_delta_replace_only_10_lines", |b| {
        b.iter(|| inverse_delta(&delta_replace, Some(old_replace.as_str())))
    });
}

pub fn bench_inverse_without_old_content(c: &mut criterion::Criterion) {
    let base_text = generate_test_text(100);
    let new_text = generate_modified_text(&base_text, 0.3);
    let (delta, _) = create_delta_from_texts(&base_text, &new_text);

    c.bench_function(
        "inverse_delta_without_old_content_100_lines_30_percent",
        |b| b.iter(|| inverse_delta(&delta, None::<&str>)),
    );
}

criterion::criterion_group!(inverse_small, bench_inverse_delta_small);
criterion::criterion_group!(inverse_medium, bench_inverse_delta_medium);
criterion::criterion_group!(inverse_large, bench_inverse_delta_large);
criterion::criterion_group!(inverse_operations, bench_inverse_operations);
criterion::criterion_group!(inverse_no_content, bench_inverse_without_old_content);

criterion::criterion_main!(
    inverse_small,
    inverse_medium,
    inverse_large,
    inverse_operations,
    inverse_no_content
);
