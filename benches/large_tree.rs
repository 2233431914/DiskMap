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
//! To regenerate fixtures, delete `target/bench-fixtures/disk-map-tree-*`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use disk_map::scanner::{scan_path_to_tree, ScanOptions};
use disk_map::tree::TreeStore;
use disk_map::treemap::{
    layout_treemap, Camera, LayoutScratch, SearchState, TreemapLayoutParams,
};
use egui::Rect;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const FIXTURE_SIZES: &[usize] = &[1_000, 10_000, 100_000];

/// Build a synthetic tree of `target_count` entries at `dest`. Returns
/// the canonical root path. Idempotent: skips generation if `dest` exists
/// and contains any file. Distribution: roughly 90% files, 10% dirs,
/// balanced branching factor of 10. Capped at depth 8 to keep paths
/// under the 255-byte filesystem limit even at 100k entries.
fn generate_tree_if_missing(target_count: usize, dest: &Path) {
    if dest.exists()
        && fs::read_dir(dest)
            .map(|mut it| it.next().is_some())
            .unwrap_or(false)
    {
        return;
    }
    fs::create_dir_all(dest).expect("create fixture root");
    let mut count = 1; // the root itself
    // BFS frontier with depth tracking so we can stop recursing deep
    let mut frontier: Vec<(PathBuf, usize)> = vec![(dest.to_path_buf(), 0)];
    let max_depth = 8usize;
    while count < target_count && !frontier.is_empty() {
        let (dir, depth) = frontier.remove(0);
        // 9 files + 1 subdir per directory, but don't make subdirs past max_depth
        for i in 0..9 {
            if count >= target_count {
                break;
            }
            let name = format!("f{}.txt", i);
            let p = dir.join(&name);
            fs::write(&p, b"x").expect("write fixture file");
            count += 1;
        }
        if count < target_count && depth + 1 < max_depth {
            let name = format!("d{}", frontier.len());
            let p = dir.join(&name);
            fs::create_dir(&p).expect("mkdir fixture subdir");
            frontier.push((p, depth + 1));
            count += 1;
        }
    }
}

fn fixture_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is set by cargo at compile time for benches.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("target")
        .join("bench-fixtures")
}

fn root_path() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    fixture_dir().join(format!("disk-map-bench-fixture-{pid}-{nanos}-{n}"))
}

fn scan_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_tree_scan");
    for &size in FIXTURE_SIZES {
        let dest = root_path().join(format!("tree-{size}"));
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
        let dest = root_path().join(format!("tree-{size}"));
        generate_tree_if_missing(size, &dest);
        // Pre-scan once to get a tree to layout
        let mut tree = scan_path_to_tree(dest.clone(), ScanOptions::default())
            .expect("pre-scan should succeed");
        let root_id = tree.root.expect("scanned tree has a root");
        let canvas = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));
        let camera = Camera::default();
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
                        camera,
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
        let dest = root_path().join(format!("tree-{size}"));
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
