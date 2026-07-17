use criterion::{criterion_group, criterion_main, Criterion};
use disk_map::tree::{NodeId, NodeKind, TreeStore};
use disk_map::treemap::{layout_treemap, LayoutScratch, SearchState, TreemapLayoutParams};
use egui::Rect;
use std::hint::black_box;

fn build_tree(node_count: usize) -> (TreeStore, NodeId) {
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

fn search_rebuild_bench(c: &mut Criterion) {
    let (tree, root) = build_tree(20_000);
    c.bench_function("search_rebuild_bench", |b| {
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
                        max_depth: 2,
                        search_state: &search_state,
                        filter_to_search: false,
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

criterion_group!(benches, search_rebuild_bench, treemap_layout_bench);
criterion_main!(benches);
