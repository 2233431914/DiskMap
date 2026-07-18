//! Large-tree benchmark suite with fixed on-disk fixtures.
//!
//! Generates balanced directory trees (1k, 10k, 100k entries) plus flat-100k
//! and deep-10k scan fixtures under `target/bench-fixtures/`, then measures
//! `scan_path_to_tree` and `layout_treemap`. Recorded baselines live in
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
use crossbeam_channel::unbounded;
use disk_map::scanner::{scan_path_to_tree, start_scan, PerfStats, ScanMessage, ScanOptions};
use disk_map::tree::TreeStore;
use disk_map::treemap::{layout_treemap, LayoutScratch, SearchState, TreemapLayoutParams};
use egui::Rect;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const FIXTURE_SIZES: &[usize] = &[1_000, 10_000, 100_000];
const FIXTURE_VERSION: u32 = 2;
const FLAT_FIXTURE_SIZE: usize = 100_000;
const DEEP_FIXTURE_SIZE: usize = 10_000;
const DEEP_FIXTURE_LEVELS: usize = 32;

struct ScanProbe {
    elapsed: Duration,
    first_batch: Option<Duration>,
    perf_stats: PerfStats,
}

#[derive(Clone, Copy)]
enum FixtureShape {
    Balanced,
    Flat,
    Deep,
}

impl FixtureShape {
    fn marker_name(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Flat => "flat",
            Self::Deep => "deep-32",
        }
    }
}

/// Build a synthetic tree with exactly `target_count` entries at `dest`.
/// A sibling completion marker makes fixture reuse deterministic.
fn generate_tree_if_missing(target_count: usize, dest: &Path, shape: FixtureShape) {
    let marker = dest.with_extension("complete");
    let expected_marker = format!(
        "disk-map-fixture-v{FIXTURE_VERSION}:{}:{target_count}",
        shape.marker_name()
    );
    if dest.is_dir()
        && fs::read_to_string(&marker)
            .ok()
            .is_some_and(|value| value.trim() == expected_marker)
    {
        return;
    }
    if dest.exists() {
        fs::remove_dir_all(dest).expect("remove stale benchmark fixture");
    }
    fs::create_dir_all(dest).expect("create fixture root");

    match shape {
        FixtureShape::Balanced => generate_balanced_tree(target_count, dest),
        FixtureShape::Flat => generate_flat_tree(target_count, dest),
        FixtureShape::Deep => generate_deep_tree(target_count, dest),
    }

    fs::write(marker, expected_marker).expect("write fixture marker");
}

fn generate_balanced_tree(target_count: usize, dest: &Path) {
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
}

fn generate_flat_tree(target_count: usize, dest: &Path) {
    for index in 0..target_count.saturating_sub(1) {
        fs::write(dest.join(format!("f{index:06}.txt")), b"x").expect("write flat fixture file");
    }
}

fn generate_deep_tree(target_count: usize, dest: &Path) {
    let mut directories = Vec::with_capacity(DEEP_FIXTURE_LEVELS);
    let mut current = dest.to_path_buf();
    for level in 0..DEEP_FIXTURE_LEVELS {
        current = current.join(format!("level-{level:02}"));
        fs::create_dir(&current).expect("mkdir deep fixture directory");
        directories.push(current.clone());
    }

    let directory_entries = 1 + directories.len();
    for index in 0..target_count.saturating_sub(directory_entries) {
        let directory = &directories[index % directories.len()];
        fs::write(directory.join(format!("f{index:06}.txt")), b"x")
            .expect("write deep fixture file");
    }
}

fn balanced_fixture_path(target_count: usize) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/bench-fixtures")
        .join(format!("disk-map-tree-{target_count}"))
}

fn shaped_fixture_path(shape: &str, target_count: usize) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/bench-fixtures")
        .join(format!("disk-map-{shape}-{target_count}"))
}

fn probe_scan(path: &Path, options: ScanOptions) -> ScanProbe {
    let (tx, rx) = unbounded();
    let started_at = Instant::now();
    let _handle = start_scan(path.to_path_buf(), 1, options, tx);
    let mut first_batch = None;

    loop {
        match rx.recv().expect("probe scan channel should stay open") {
            ScanMessage::Started { .. } => {}
            ScanMessage::Batch { .. } => {
                first_batch.get_or_insert_with(|| started_at.elapsed());
            }
            ScanMessage::Finished { perf_stats, .. } => {
                return ScanProbe {
                    elapsed: started_at.elapsed(),
                    first_batch,
                    perf_stats,
                };
            }
            ScanMessage::Cancelled { .. } => panic!("probe scan was cancelled"),
            ScanMessage::Error { message, .. } => panic!("probe scan failed: {message}"),
        }
    }
}

fn maybe_print_probe(label: &str, path: &Path, options: ScanOptions) {
    let enabled = std::env::var("DISKMAP_SCAN_PROBE")
        .ok()
        .is_some_and(|value| value == "all" || value == label);
    if !enabled {
        return;
    }

    let probe = probe_scan(path, options);
    eprintln!(
        "scan_probe label={label} elapsed_ms={:.3} first_batch_ms={:.3} prefetched_files={} metadata_fallback_files={}",
        probe.elapsed.as_secs_f64() * 1000.0,
        probe
            .first_batch
            .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0),
        probe.perf_stats.prefetched_files,
        probe.perf_stats.metadata_fallback_files,
    );
}

fn realistic_exclude_options() -> ScanOptions {
    ScanOptions {
        exclude_patterns: vec![
            ".git".into(),
            "node_modules".into(),
            "build".into(),
            "*.tmp".into(),
            "Library/Caches".into(),
        ],
        ..ScanOptions::default()
    }
}

fn scan_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_tree_scan");
    for &size in FIXTURE_SIZES {
        let dest = balanced_fixture_path(size);
        generate_tree_if_missing(size, &dest, FixtureShape::Balanced);
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

fn targeted_scan_bench(c: &mut Criterion) {
    let balanced = balanced_fixture_path(100_000);
    generate_tree_if_missing(100_000, &balanced, FixtureShape::Balanced);
    maybe_print_probe("balanced_default_100k", &balanced, ScanOptions::default());
    c.bench_function("balanced_default_100k", |b| {
        b.iter(|| {
            black_box(
                scan_path_to_tree(black_box(balanced.clone()), ScanOptions::default())
                    .expect("balanced scan should succeed"),
            )
        })
    });

    #[cfg(unix)]
    c.bench_function("balanced_stay_on_filesystem_100k", |b| {
        b.iter(|| {
            let options = ScanOptions {
                stay_on_filesystem: true,
                ..ScanOptions::default()
            };
            black_box(
                scan_path_to_tree(black_box(balanced.clone()), options)
                    .expect("balanced stay-on-filesystem scan should succeed"),
            )
        })
    });

    c.bench_function("balanced_exclude_miss_100k", |b| {
        b.iter(|| {
            let options = ScanOptions {
                exclude_patterns: vec!["__diskmap_no_match__".into()],
                ..ScanOptions::default()
            };
            black_box(
                scan_path_to_tree(black_box(balanced.clone()), options)
                    .expect("balanced exclude-miss scan should succeed"),
            )
        })
    });

    maybe_print_probe(
        "balanced_exclude_rules_100k",
        &balanced,
        realistic_exclude_options(),
    );
    c.bench_function("balanced_exclude_rules_100k", |b| {
        b.iter(|| {
            black_box(
                scan_path_to_tree(black_box(balanced.clone()), realistic_exclude_options())
                    .expect("balanced realistic exclude scan should succeed"),
            )
        })
    });

    let flat = shaped_fixture_path("flat", FLAT_FIXTURE_SIZE);
    generate_tree_if_missing(FLAT_FIXTURE_SIZE, &flat, FixtureShape::Flat);
    maybe_print_probe("flat_default_100k", &flat, ScanOptions::default());
    c.bench_function("flat_default_100k", |b| {
        b.iter(|| {
            black_box(
                scan_path_to_tree(black_box(flat.clone()), ScanOptions::default())
                    .expect("flat scan should succeed"),
            )
        })
    });

    let deep = shaped_fixture_path("deep", DEEP_FIXTURE_SIZE);
    generate_tree_if_missing(DEEP_FIXTURE_SIZE, &deep, FixtureShape::Deep);
    maybe_print_probe("deep_default_10k", &deep, ScanOptions::default());
    c.bench_function("deep_default_10k", |b| {
        b.iter(|| {
            black_box(
                scan_path_to_tree(black_box(deep.clone()), ScanOptions::default())
                    .expect("deep scan should succeed"),
            )
        })
    });
}

fn layout_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_tree_layout");
    for &size in FIXTURE_SIZES {
        let dest = balanced_fixture_path(size);
        generate_tree_if_missing(size, &dest, FixtureShape::Balanced);
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
        let dest = balanced_fixture_path(size);
        generate_tree_if_missing(size, &dest, FixtureShape::Balanced);
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
    targets = baseline_measurement, scan_bench, targeted_scan_bench, layout_bench
);
criterion_main!(large_tree);
