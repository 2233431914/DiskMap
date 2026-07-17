//! Large-tree benchmark suite with fixed on-disk fixtures.
//!
//! Generates three synthetic directory trees (1k, 10k, 100k entries) under
//! `target/bench-fixtures/` once, then measures `scan_path_to_tree` and
//! `layout_treemap` wall time on each. Recorded baselines live in
//! `benches/baselines/large_tree.txt` and are **directional** only — this
//! suite catches catastrophic regressions (e.g. a 10x slowdown) but not
//! fine-grained drift.
//!
//! Run with:
//!   cargo bench --bench large_tree
//!
//! To regenerate fixtures, delete the matching `disk-map-tree-*` directories
//! and their `.complete` marker files under `target/bench-fixtures/`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use disk_map::scanner::{scan_path_to_tree, ScanOptions};
use disk_map::tree::TreeStore;
use disk_map::treemap::{layout_treemap, LayoutScratch, SearchState, TreemapLayoutParams};
use egui::Rect;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

const FIXTURE_SIZES: &[usize] = &[1_000, 10_000, 100_000];

/// Build a synthetic tree with exactly `target_count` entries at `dest`.
/// A sibling completion marker makes fixture reuse deterministic.
fn generate_tree_if_missing(target_count: usize, dest: &Path) {
    let marker = dest.with_extension("complete");
    if dest.is_dir()
        && fs::read_to_string(&marker)
            .ok()
            .is_some_and(|value| value.trim() == target_count.to_string())
    {
        return;
    }
    if dest.exists() {
        fs::remove_dir_all(dest).expect("remove stale benchmark fixture");
    }
    fs::create_dir_all(dest).expect("create fixture root");
    let directory_count = (target_count / 100).max(1);
    let mut directories = Vec::with_capacity(directory_count);
    for index in 0..directory_count {
        let directory = dest.join(format!("d{index:04}"));
        fs::create_dir(&directory).expect("mkdir fixture directory");
        directories.push(directory);
    }

    let mut count = 1 + directory_count;
    let mut file_index = 0usize;
    while count < target_count {
        let directory = &directories[file_index % directories.len()];
        fs::write(directory.join(format!("f{file_index:06}.txt")), b"x")
            .expect("write fixture file");
        file_index += 1;
        count += 1;
    }
    fs::write(marker, target_count.to_string()).expect("write fixture marker");
}

fn fixture_path(target_count: usize) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/bench-fixtures")
        .join(format!("disk-map-tree-{target_count}"))
}

fn scan_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_tree_scan");
    for &size in FIXTURE_SIZES {
        let dest = fixture_path(size);
        generate_tree_if_missing(size, &dest);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| {
                let options = ScanOptions::default();
                let tree: TreeStore = black_box(
                    scan_path_to_tree(black_box(dest.clone()), options)
                        .expect("scan should succeed"),
                );
                tree
            });
        });
    }
    group.finish();
}

fn layout_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_tree_layout");
    for &size in FIXTURE_SIZES {
        let dest = fixture_path(size);
        generate_tree_if_missing(size, &dest);
        // Pre-scan once to get a tree to layout
        let mut tree = scan_path_to_tree(dest.clone(), ScanOptions::default())
            .expect("pre-scan should succeed");
        let root_id = tree.root.expect("scanned tree has a root");
        let canvas = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));
        let search_state = SearchState::new(tree.len());
        let mut out = Vec::new();
        let mut scratch = LayoutScratch::default();
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| {
                layout_treemap(
                    black_box(&mut tree),
                    TreemapLayoutParams {
                        root: root_id,
                        canvas_rect: canvas,
                        max_depth: 4,
                        search_state: &search_state,
                        filter_to_search: false,
                        out: &mut out,
                        scratch: &mut scratch,
                    },
                );
                black_box(&mut out).clear();
            });
        });
    }
    group.finish();
}

/// Wall-time single run, used to record the baseline file. Not a
/// statistical bench — runs once per fixture size and writes results.
fn baseline_measurement(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_tree_baseline");
    for &size in FIXTURE_SIZES {
        let dest = fixture_path(size);
        generate_tree_if_missing(size, &dest);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter_custom(|iters| {
                let options = ScanOptions::default();
                let mut total = std::time::Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    let tree = scan_path_to_tree(dest.clone(), options.clone())
                        .expect("scan should succeed");
                    total += start.elapsed();
                    let _root = tree.root;
                }
                total
            });
        });
    }
    group.finish();
}

criterion_group!(
    name = large_tree;
    config = Criterion::default().sample_size(10);
    targets = baseline_measurement, scan_bench, layout_bench
);
criterion_main!(large_tree);
