use criterion::{criterion_group, criterion_main, Criterion};
use disk_map::scanner::{
    CacheMode, DiscoveredNode, PerfStats, ProgressSnapshot, ScanBatch, ScanOptions,
};
use disk_map::tree::{NodeKind, NodeRecord, TreeStore};
use disk_map::treemap::{layout_treemap, Camera, LayoutScratch, SearchState, TreemapLayoutParams};
use egui::Rect;
use rustc_hash::FxHashMap;
use std::hint::black_box;
use std::time::Duration;

fn build_tree(node_count: usize) -> (TreeStore, usize) {
    let mut tree = TreeStore::new();
    let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
    tree.set_root_path("/".into());
    let mut total = 0u64;

    for index in 0..node_count {
        let size = (node_count - index) as u64 + 1;
        tree.add_node(Some(root), format!("file-{index}"), NodeKind::File, size);
        total += size;
    }

    tree.apply_direct_size_delta(root, total);
    tree.repair_sorted_children(&[root]);
    (tree, root)
}

fn scan_batch_aggregation_bench(c: &mut Criterion) {
    c.bench_function("scan_batch_aggregation_bench", |b| {
        b.iter(|| {
            let mut batch = ScanBatch::default();
            let mut size_map = FxHashMap::<usize, u64>::default();
            for index in 0..4096usize {
                batch.discovered_nodes.push(DiscoveredNode {
                    node_id: index + 1,
                    parent_id: 0,
                    node: NodeRecord {
                        name: format!("node-{index}"),
                        kind: NodeKind::File,
                        size: 1,
                        scanned: true,
                        error: None,
                    },
                });
                *size_map.entry(index % 16).or_insert(0) += 1;
            }
            batch.size_deltas = size_map.into_iter().collect();
            batch.progress = Some(ProgressSnapshot {
                files_scanned: 4096,
                dirs_scanned: 16,
                bytes_seen: 4096,
                current_path: "/tmp".into(),
            });

            let stats = PerfStats {
                messages_sent: 2,
                batches_sent: 1,
                entries_seen: 4096,
                nodes_discovered: 4096,
                files_scanned: 4096,
                dirs_scanned: 16,
                size_delta_merges: 4080,
                ancestor_size_delta_total_ms: 0.0,
                parent_stack_hits: 4096,
                parent_lookup_fallbacks: 0,
                progress_snapshots_sent: 1,
                prefetched_files: 4096,
                metadata_fallback_files: 0,
                metadata_total_ms: 0.0,
                mtime_total_ms: 0.0,
                size_measure_total_ms: 0.0,
                batch_flush_total_ms: 0.0,
                scan_elapsed_ms: 0.0,
                layout_recompute_count: 0,
                layout_total_ms: 0.0,
                search_rebuild_count: 0,
                search_incremental_updates: 0,
                db_cache_hits: 0,
                db_cache_misses: 0,
                db_flush_count: 0,
            };

            black_box((
                batch,
                stats,
                ScanOptions {
                    batch_flush_interval: Duration::from_millis(33),
                    max_pending_nodes: 2048,
                    max_pending_size_deltas: 4096,
                    cache_mode: CacheMode::Disabled,
                    exclude_patterns: Vec::new(),
                    include_hidden: true,
                    follow_symlinks: false,
                    stay_on_filesystem: false,
                },
            ));
        })
    });
}

fn parent_lookup_hot_path_bench(c: &mut Criterion) {
    c.bench_function("parent_lookup_hot_path_bench", |b| {
        b.iter(|| {
            let mut stack = vec![0usize];
            let mut fallbacks = 0u64;
            for depth in (1..128usize).cycle().take(50_000) {
                if stack.len() > depth {
                    stack.truncate(depth);
                }
                if stack.get(depth.saturating_sub(1)).is_none() {
                    fallbacks += 1;
                    stack.resize(depth, 0);
                }
                if stack.len() <= depth {
                    stack.resize(depth + 1, 0);
                }
                stack[depth] = depth;
            }
            black_box((stack, fallbacks));
        })
    });
}

fn search_incremental_bench(c: &mut Criterion) {
    let (tree, root) = build_tree(20_000);
    c.bench_function("search_incremental_bench", |b| {
        b.iter(|| {
            let mut state = SearchState::default();
            let mut tree = tree.clone();
            state.rebuild(&mut tree, Some(root), "file-199");
            black_box(state);
        })
    });
}

fn treemap_layout_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("treemap_layout_bench");
    for count in [1_000usize, 10_000, 50_000] {
        let (tree, root) = build_tree(count);
        let search_state = SearchState::default();
        let canvas = Rect::from_min_max((0.0, 0.0).into(), (1400.0, 900.0).into());
        group.bench_with_input(format!("nodes_{count}"), &count, |b, _| {
            let mut tree = tree.clone();
            let mut visuals = Vec::new();
            let mut scratch = LayoutScratch::default();
            b.iter(|| {
                layout_treemap(
                    &mut tree,
                    TreemapLayoutParams {
                        root,
                        canvas_rect: canvas,
                        camera: Camera::default(),
                        max_depth: 2,
                        search_state: &search_state,
                        out: &mut visuals,
                        scratch: &mut scratch,
                    },
                );
                black_box(visuals.len())
            })
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    scan_batch_aggregation_bench,
    parent_lookup_hot_path_bench,
    search_incremental_bench,
    treemap_layout_bench
);
criterion_main!(benches);
