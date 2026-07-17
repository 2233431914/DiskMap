use crate::format::format_bytes;
use crate::tree::{node_index, NodeId, TreeStore};
use egui::{pos2, Rect, Vec2};
use fixedbitset::FixedBitSet;
use smallvec::SmallVec;

const MAX_VISUAL_NODES: usize = 12_000;
const MIN_DRAW_SIDE: f32 = 2.0;
const INNER_PADDING: f32 = 1.0;
const STRIP_ASPECT_WARNING: f32 = 10.0;
const SMALL_FILE_VISUAL_THRESHOLD_BYTES: u64 = 16 * 1024;
const MIN_SMALL_FILE_AGGREGATE_COUNT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelMode {
    Full,
    Compact,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualKind {
    Node(NodeId),
    SmallFiles {
        parent_id: NodeId,
        count: u32,
        size: u64,
    },
}

#[derive(Debug, Clone)]
pub struct VisualNode {
    pub kind: VisualKind,
    pub rect: Rect,
    pub depth: usize,
    pub is_dir: bool,
    pub size: u64,
    pub label_mode: LabelMode,
    pub matched: bool,
    pub ancestor_of_match: bool,
    pub hidden_by_search: bool,
    pub label_text: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct SearchState {
    query_lower: String,
    lowercase_scratch: String,
    matched_bits: FixedBitSet,
    matched_ids: Vec<NodeId>,
    matched_descendant_counts: Vec<u32>,
}

#[derive(Debug, Clone, Copy)]
enum LayoutItemKind {
    Node(NodeId),
    SmallFiles {
        parent_id: NodeId,
        count: u32,
        size: u64,
    },
}

#[derive(Debug, Clone, Copy)]
struct LayoutItem {
    kind: LayoutItemKind,
    area: f32,
}

#[derive(Debug, Default, Clone)]
struct LayoutDebugStats {
    row_count: usize,
    single_item_rows: usize,
    rect_count: usize,
    strip_like_rects: usize,
}

#[derive(Debug, Default)]
pub struct LayoutScratch {
    items: Vec<LayoutItem>,
    row: SmallVec<[LayoutItem; 16]>,
}

pub struct TreemapLayoutParams<'a, 'b> {
    pub root: NodeId,
    pub canvas_rect: Rect,
    pub max_depth: usize,
    pub search_state: &'a SearchState,
    pub filter_to_search: bool,
    pub out: &'b mut Vec<VisualNode>,
    pub scratch: &'b mut LayoutScratch,
}

struct LayoutContext<'a> {
    tree: &'a mut TreeStore,
    root_id: NodeId,
    max_depth: usize,
    search_state: &'a SearchState,
    filter_to_search: bool,
    out: &'a mut Vec<VisualNode>,
    scratch: &'a mut LayoutScratch,
}

impl SearchState {
    /// Construct an empty `SearchState` sized to the given tree length.
    /// Use `clear` to reset when the tree size changes mid-session.
    pub fn new(tree_len: usize) -> Self {
        let mut state = Self::default();
        state.matched_bits.grow(tree_len);
        state.matched_descendant_counts.resize(tree_len, 0);
        state
    }

    pub fn clear(&mut self, tree_len: usize) {
        self.query_lower.clear();
        self.lowercase_scratch.clear();
        self.matched_bits.clear();
        self.matched_bits.grow(tree_len);
        self.matched_ids.clear();
        self.matched_descendant_counts.clear();
        self.matched_descendant_counts.resize(tree_len, 0);
    }

    pub fn query(&self) -> &str {
        &self.query_lower
    }

    pub fn rebuild(&mut self, tree: &mut TreeStore, root_id: Option<NodeId>, query: &str) {
        self.query_lower = query.trim().to_lowercase();
        self.matched_bits.clear();
        self.matched_bits.grow(tree.len());
        self.matched_ids.clear();
        self.matched_descendant_counts.clear();
        self.matched_descendant_counts.resize(tree.len(), 0);

        let Some(root_id) = root_id else {
            return;
        };
        if self.query_lower.is_empty() {
            return;
        }

        let mut stack = vec![root_id];
        while let Some(node_id) = stack.pop() {
            self.ingest_node_if_matches(tree, node_id);
            tree.ensure_sorted_children(node_id);
            stack.extend(tree.sorted_children(node_id).iter().rev().copied());
        }
    }

    pub fn ingest_new_nodes(&mut self, tree: &mut TreeStore, node_ids: &[NodeId]) -> usize {
        if self.query_lower.is_empty() {
            return 0;
        }

        if self.matched_bits.len() < tree.len() {
            self.matched_bits.grow(tree.len());
        }
        if self.matched_descendant_counts.len() < tree.len() {
            self.matched_descendant_counts.resize(tree.len(), 0);
        }

        let mut updates = 0usize;
        for &node_id in node_ids {
            if self.ingest_node_if_matches(tree, node_id) {
                updates += 1;
            }
        }
        updates
    }

    pub fn is_match(&self, node_id: NodeId) -> bool {
        self.matched_bits.contains(node_index(node_id))
    }

    pub fn is_ancestor_of_match(&self, node_id: NodeId) -> bool {
        self.matched_descendant_counts
            .get(node_index(node_id))
            .copied()
            .unwrap_or_default()
            > 0
            && !self.is_match(node_id)
    }

    pub fn is_hidden(&self, node_id: NodeId) -> bool {
        !self.query_lower.is_empty()
            && !self.is_match(node_id)
            && self
                .matched_descendant_counts
                .get(node_index(node_id))
                .copied()
                .unwrap_or_default()
                == 0
    }

    pub fn match_count(&self) -> usize {
        self.matched_ids.len()
    }

    pub fn matches(&self) -> &[NodeId] {
        &self.matched_ids
    }

    pub fn has_query(&self) -> bool {
        !self.query_lower.is_empty()
    }

    pub fn visible_in_filter(&self, node_id: NodeId) -> bool {
        !self.has_query() || self.is_match(node_id) || self.is_ancestor_of_match(node_id)
    }

    fn ingest_node_if_matches(&mut self, tree: &mut TreeStore, node_id: NodeId) -> bool {
        let current_index = node_index(node_id);
        if self.query_lower.is_empty() || current_index >= tree.len() {
            return false;
        }

        if !tree.node_name_matches_query(node_id, &self.query_lower, &mut self.lowercase_scratch)
            || self.matched_bits.contains(current_index)
        {
            return false;
        }
        self.matched_bits.insert(current_index);
        self.matched_ids.push(node_id);

        let mut current = tree.node(node_id).parent;
        while let Some(ancestor_id) = current {
            let ancestor_index = node_index(ancestor_id);
            if ancestor_index >= self.matched_descendant_counts.len() {
                self.matched_descendant_counts.resize(tree.len(), 0);
            }
            self.matched_descendant_counts[ancestor_index] += 1;
            current = tree.node(ancestor_id).parent;
        }
        true
    }
}

pub fn layout_treemap(tree: &mut TreeStore, params: TreemapLayoutParams<'_, '_>) {
    params.out.clear();
    let layout_rect = params.canvas_rect.shrink(8.0);
    let mut context = LayoutContext {
        tree,
        root_id: params.root,
        max_depth: params.max_depth,
        search_state: params.search_state,
        filter_to_search: params.filter_to_search && params.search_state.has_query(),
        out: params.out,
        scratch: params.scratch,
    };

    layout_node_squarified(&mut context, params.root, layout_rect, 0);
}

fn layout_node_squarified(
    context: &mut LayoutContext<'_>,
    parent_id: NodeId,
    rect: Rect,
    depth: usize,
) {
    if context.out.len() >= MAX_VISUAL_NODES || depth >= context.max_depth {
        return;
    }
    if rect.width() < MIN_DRAW_SIDE || rect.height() < MIN_DRAW_SIDE {
        return;
    }

    context.tree.ensure_sorted_children(parent_id);
    let children = context.tree.sorted_children(parent_id);
    if children.is_empty() {
        return;
    }

    let parent_area = rect.width() * rect.height();
    if parent_area <= 0.0 {
        return;
    }

    let total_child_size: u64 = children
        .iter()
        .filter(|child_id| {
            !context.filter_to_search || context.search_state.visible_in_filter(**child_id)
        })
        .map(|child_id| context.tree.node(*child_id).size)
        .filter(|size| *size > 0)
        .sum();
    if total_child_size == 0 {
        return;
    }

    let mut items = std::mem::take(&mut context.scratch.items);
    items.clear();
    if items.capacity() < children.len() {
        items.reserve(children.len() - items.capacity());
    }
    let aggregate_small_files = !context.search_state.has_query();
    let mut small_file_size = 0_u64;
    let mut small_file_count = 0_usize;
    let mut small_file_ids = SmallVec::<[NodeId; 8]>::new();

    for &node_id in children {
        if context.filter_to_search && !context.search_state.visible_in_filter(node_id) {
            continue;
        }
        let node = context.tree.node(node_id);
        let size = node.size;
        if size > 0 {
            if aggregate_small_files
                && matches!(node.kind, crate::tree::NodeKind::File)
                && size <= SMALL_FILE_VISUAL_THRESHOLD_BYTES
            {
                small_file_size = small_file_size.saturating_add(size);
                small_file_count += 1;
                if small_file_ids.len() < MIN_SMALL_FILE_AGGREGATE_COUNT {
                    small_file_ids.push(node_id);
                }
                continue;
            }

            items.push(layout_item_for_node(
                node_id,
                size,
                total_child_size,
                parent_area,
            ));
        }
    }

    if small_file_count >= MIN_SMALL_FILE_AGGREGATE_COUNT {
        items.push(LayoutItem {
            kind: LayoutItemKind::SmallFiles {
                parent_id,
                count: u32::try_from(small_file_count).unwrap_or(u32::MAX),
                size: small_file_size,
            },
            area: (small_file_size as f32 / total_child_size as f32) * parent_area,
        });
    } else {
        for node_id in small_file_ids {
            let size = context.tree.node(node_id).size;
            items.push(layout_item_for_node(
                node_id,
                size,
                total_child_size,
                parent_area,
            ));
        }
    }
    if items.is_empty() {
        context.scratch.items = items;
        return;
    }

    let debug_stats = squarify_items(context, &items, rect, depth);
    context.scratch.items = items;

    emit_debug_warnings(depth, &debug_stats);
}

fn layout_item_for_node(
    node_id: NodeId,
    size: u64,
    total_child_size: u64,
    parent_area: f32,
) -> LayoutItem {
    LayoutItem {
        kind: LayoutItemKind::Node(node_id),
        area: (size as f32 / total_child_size as f32) * parent_area,
    }
}

fn squarify_items(
    context: &mut LayoutContext<'_>,
    items: &[LayoutItem],
    rect: Rect,
    depth: usize,
) -> LayoutDebugStats {
    let mut stats = LayoutDebugStats::default();
    let mut remaining = rect;
    context.scratch.row.clear();
    let mut index = 0usize;

    while index < items.len() && remaining.width() > 0.0 && remaining.height() > 0.0 {
        let short_side = remaining.width().min(remaining.height());
        if short_side <= 0.0 {
            break;
        }

        let next_item = items[index];
        if context.scratch.row.is_empty() {
            context.scratch.row.push(next_item);
            index += 1;
            continue;
        }

        let current_score =
            worst_aspect_ratio(context.scratch.row.iter().map(|item| item.area), short_side);
        let candidate_score = worst_aspect_ratio(
            context
                .scratch
                .row
                .iter()
                .map(|item| item.area)
                .chain(std::iter::once(next_item.area)),
            short_side,
        );

        if candidate_score <= current_score {
            context.scratch.row.push(next_item);
            index += 1;
        } else {
            let mut row_items = std::mem::take(&mut context.scratch.row);
            remaining = layout_row(context, &row_items, remaining, depth, &mut stats);
            row_items.clear();
            context.scratch.row = row_items;
        }
    }

    if !context.scratch.row.is_empty() && remaining.width() > 0.0 && remaining.height() > 0.0 {
        let mut row_items = std::mem::take(&mut context.scratch.row);
        let _ = layout_row(context, &row_items, remaining, depth, &mut stats);
        row_items.clear();
        context.scratch.row = row_items;
    }

    stats
}

fn worst_aspect_ratio<I>(areas: I, side: f32) -> f32
where
    I: IntoIterator<Item = f32>,
{
    if side <= 0.0 {
        return f32::INFINITY;
    }

    let mut sum = 0.0_f32;
    let mut max_area = 0.0_f32;
    let mut min_area = f32::INFINITY;

    for area in areas {
        if area <= 0.0 {
            return f32::INFINITY;
        }
        sum += area;
        max_area = max_area.max(area);
        min_area = min_area.min(area);
    }

    if sum <= 0.0 || min_area <= 0.0 || !min_area.is_finite() {
        return f32::INFINITY;
    }

    let side2 = side * side;
    ((side2 * max_area) / (sum * sum)).max((sum * sum) / (side2 * min_area))
}

fn layout_row(
    context: &mut LayoutContext<'_>,
    row: &[LayoutItem],
    remaining_rect: Rect,
    depth: usize,
    debug_stats: &mut LayoutDebugStats,
) -> Rect {
    let sum_area: f32 = row.iter().map(|item| item.area).sum();
    if sum_area <= 0.0 || remaining_rect.width() <= 0.0 || remaining_rect.height() <= 0.0 {
        return remaining_rect;
    }

    debug_stats.row_count += 1;
    if row.len() == 1 {
        debug_stats.single_item_rows += 1;
    }

    if remaining_rect.width() >= remaining_rect.height() {
        layout_vertical_column(context, row, remaining_rect, sum_area, depth, debug_stats)
    } else {
        layout_horizontal_row(context, row, remaining_rect, sum_area, depth, debug_stats)
    }
}

fn layout_horizontal_row(
    context: &mut LayoutContext<'_>,
    row: &[LayoutItem],
    remaining_rect: Rect,
    sum_area: f32,
    depth: usize,
    debug_stats: &mut LayoutDebugStats,
) -> Rect {
    let row_height =
        (sum_area / remaining_rect.width().max(f32::EPSILON)).min(remaining_rect.height());
    if row_height <= 0.0 {
        return remaining_rect;
    }

    let row_rect = Rect::from_min_max(
        remaining_rect.min,
        pos2(remaining_rect.right(), remaining_rect.top() + row_height),
    );

    let mut x = row_rect.left();
    for (index, item) in row.iter().copied().enumerate() {
        let width = if index + 1 == row.len() {
            row_rect.right() - x
        } else {
            (item.area / row_height.max(f32::EPSILON)).min(row_rect.right() - x)
        };
        let item_rect = Rect::from_min_max(
            pos2(x, row_rect.top()),
            pos2((x + width).min(row_rect.right()), row_rect.bottom()),
        );
        x = item_rect.right();
        emit_visual_and_recurse(context, item.kind, item_rect, depth, debug_stats);
    }

    Rect::from_min_max(
        pos2(remaining_rect.left(), row_rect.bottom()),
        remaining_rect.max,
    )
}

fn layout_vertical_column(
    context: &mut LayoutContext<'_>,
    row: &[LayoutItem],
    remaining_rect: Rect,
    sum_area: f32,
    depth: usize,
    debug_stats: &mut LayoutDebugStats,
) -> Rect {
    let column_width =
        (sum_area / remaining_rect.height().max(f32::EPSILON)).min(remaining_rect.width());
    if column_width <= 0.0 {
        return remaining_rect;
    }

    let column_rect = Rect::from_min_max(
        remaining_rect.min,
        pos2(
            remaining_rect.left() + column_width,
            remaining_rect.bottom(),
        ),
    );

    let mut y = column_rect.top();
    for (index, item) in row.iter().copied().enumerate() {
        let height = if index + 1 == row.len() {
            column_rect.bottom() - y
        } else {
            (item.area / column_width.max(f32::EPSILON)).min(column_rect.bottom() - y)
        };
        let item_rect = Rect::from_min_max(
            pos2(column_rect.left(), y),
            pos2(column_rect.right(), (y + height).min(column_rect.bottom())),
        );
        y = item_rect.bottom();
        emit_visual_and_recurse(context, item.kind, item_rect, depth, debug_stats);
    }

    Rect::from_min_max(
        pos2(column_rect.right(), remaining_rect.top()),
        remaining_rect.max,
    )
}

fn emit_visual_and_recurse(
    context: &mut LayoutContext<'_>,
    kind: LayoutItemKind,
    raw_rect: Rect,
    depth: usize,
    debug_stats: &mut LayoutDebugStats,
) {
    if context.out.len() >= MAX_VISUAL_NODES
        || raw_rect.width() < MIN_DRAW_SIDE
        || raw_rect.height() < MIN_DRAW_SIDE
    {
        return;
    }

    debug_stats.rect_count += 1;
    let aspect = rect_aspect_ratio(raw_rect);
    if aspect > STRIP_ASPECT_WARNING {
        debug_stats.strip_like_rects += 1;
    }

    let draw_rect = inset_rect(raw_rect, INNER_PADDING);
    if draw_rect.width() < MIN_DRAW_SIDE || draw_rect.height() < MIN_DRAW_SIDE {
        return;
    }

    match kind {
        LayoutItemKind::Node(node_id) => context.out.push(make_visual_node(
            context.tree,
            node_id,
            draw_rect,
            depth,
            context.root_id,
            context.search_state,
        )),
        LayoutItemKind::SmallFiles {
            parent_id,
            count,
            size,
        } => context.out.push(make_small_files_visual(
            parent_id, count, size, draw_rect, depth,
        )),
    }

    let LayoutItemKind::Node(node_id) = kind else {
        return;
    };

    if depth + 1 >= context.max_depth {
        return;
    }

    let node = context.tree.node(node_id);
    if node.children.is_empty() {
        return;
    }

    let inner_rect = inset_rect(raw_rect, INNER_PADDING * 2.0);
    if inner_rect.width() < MIN_DRAW_SIDE || inner_rect.height() < MIN_DRAW_SIDE {
        return;
    }

    layout_node_squarified(context, node_id, inner_rect, depth + 1);
}

fn make_small_files_visual(
    parent_id: NodeId,
    count: u32,
    size: u64,
    rect: Rect,
    depth: usize,
) -> VisualNode {
    let label_mode = label_mode_for_rect(rect);
    VisualNode {
        kind: VisualKind::SmallFiles {
            parent_id,
            count,
            size,
        },
        rect,
        depth,
        is_dir: false,
        size,
        label_mode,
        matched: false,
        ancestor_of_match: false,
        hidden_by_search: false,
        label_text: match label_mode {
            LabelMode::Hidden => None,
            LabelMode::Full | LabelMode::Compact => {
                Some(format!("Other Files ({count})\n{}", format_bytes(size)))
            }
        },
    }
}

fn inset_rect(rect: Rect, padding: f32) -> Rect {
    rect.shrink2(Vec2::new(
        padding.min(rect.width() * 0.5),
        padding.min(rect.height() * 0.5),
    ))
}

pub fn rect_aspect_ratio(rect: Rect) -> f32 {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return f32::INFINITY;
    }
    (rect.width() / rect.height()).max(rect.height() / rect.width())
}

#[cfg(debug_assertions)]
fn emit_debug_warnings(depth: usize, stats: &LayoutDebugStats) {
    if stats.row_count > 1 && stats.single_item_rows == stats.row_count {
        eprintln!(
            "warning: squarify grouping failed at depth {depth}: every row had exactly one item"
        );
    }
    if stats.rect_count > 0 && (stats.strip_like_rects as f32 / stats.rect_count as f32) > 0.6 {
        eprintln!(
            "warning: strip-heavy squarify output at depth {depth}: {}/{} rects exceed aspect {}",
            stats.strip_like_rects, stats.rect_count, STRIP_ASPECT_WARNING
        );
    }
}

#[cfg(not(debug_assertions))]
fn emit_debug_warnings(_depth: usize, _stats: &LayoutDebugStats) {}

fn make_visual_node(
    tree: &TreeStore,
    node_id: NodeId,
    rect: Rect,
    depth: usize,
    root_id: NodeId,
    search_state: &SearchState,
) -> VisualNode {
    let node = tree.node(node_id);
    let matched = search_state.is_match(node_id);
    let ancestor_of_match = search_state.is_ancestor_of_match(node_id);
    let hidden_by_search = search_state.is_hidden(node_id);
    let label_mode = label_mode_for_rect(rect);

    let label_text = match label_mode {
        LabelMode::Full => Some(if node_id == root_id {
            format!("{} (root)\n{}", node.name, format_bytes(node.size))
        } else {
            format!("{}\n{}", node.name, format_bytes(node.size))
        }),
        LabelMode::Compact => Some(if node_id == root_id {
            format!("{} (root)\n{}", node.name, format_bytes(node.size))
        } else {
            format!("{}\n{}", node.name, format_bytes(node.size))
        }),
        LabelMode::Hidden => None,
    };

    VisualNode {
        kind: VisualKind::Node(node_id),
        rect,
        depth,
        is_dir: !node.children.is_empty(),
        size: node.size,
        label_mode,
        matched,
        ancestor_of_match,
        hidden_by_search,
        label_text,
    }
}

fn label_mode_for_rect(rect: Rect) -> LabelMode {
    if rect.width() > 120.0 && rect.height() > 72.0 {
        LabelMode::Full
    } else if rect.width() > 72.0 && rect.height() > 36.0 {
        LabelMode::Compact
    } else {
        LabelMode::Hidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{NodeKind, TreeStore};

    fn sample_tree(sizes: &[u64]) -> (TreeStore, NodeId) {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/".into());
        let mut total = 0_u64;
        for (index, size) in sizes.iter().copied().enumerate() {
            let child = tree.add_node(Some(root), format!("child-{index}"), NodeKind::File, size);
            tree.node_mut(child).scanned = true;
            total += size;
        }
        tree.apply_direct_size_delta(root, total);
        tree.repair_sorted_children(&[root]);
        (tree, root)
    }

    #[test]
    fn layout_should_keep_rects_inside_canvas() {
        let (mut tree, root) = sample_tree(&[60, 40]);
        tree.repair_sorted_children(&[root]);
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 800.0));
        let mut visuals = Vec::new();
        let mut scratch = LayoutScratch::default();
        layout_treemap(
            &mut tree,
            TreemapLayoutParams {
                root,
                canvas_rect: canvas,
                max_depth: 1,
                search_state: &SearchState::default(),
                filter_to_search: false,
                out: &mut visuals,
                scratch: &mut scratch,
            },
        );

        assert!(!visuals.is_empty());
        assert!(visuals
            .iter()
            .all(|visual| canvas.contains(visual.rect.min) && canvas.contains(visual.rect.max)));
    }

    #[test]
    fn search_state_marks_ancestors_without_recursive_queries() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/".into());
        let dir = tree.add_node(Some(root), "dir".into(), NodeKind::Dir, 0);
        let file = tree.add_node(Some(dir), "match".into(), NodeKind::File, 1);
        let mut search_state = SearchState::default();

        search_state.rebuild(&mut tree, Some(root), "match");

        assert!(search_state.is_match(file));
        assert!(search_state.is_ancestor_of_match(root));
        assert!(search_state.is_ancestor_of_match(dir));
        assert!(!search_state.is_hidden(dir));
        assert_eq!(search_state.matches(), &[file]);
        assert_eq!(search_state.match_count(), search_state.matches().len());
    }

    #[test]
    fn search_state_clear_removes_ordered_matches() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/".into());
        let file = tree.add_node(Some(root), "match".into(), NodeKind::File, 1);
        let mut search_state = SearchState::default();
        search_state.rebuild(&mut tree, Some(root), "match");

        search_state.clear(tree.len());

        assert!(search_state.matches().is_empty());
        assert_eq!(search_state.match_count(), 0);
        assert!(!search_state.is_match(file));
    }

    #[test]
    fn search_state_matches_follow_sorted_depth_first_order() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/".into());
        let small_file = tree.add_node(Some(root), "match-small".into(), NodeKind::File, 1);
        let dir = tree.add_node(Some(root), "match-dir".into(), NodeKind::Dir, 10);
        let nested = tree.add_node(Some(dir), "match-nested".into(), NodeKind::File, 1);
        let large_file = tree.add_node(Some(root), "match-large".into(), NodeKind::File, 20);
        tree.repair_sorted_children(&[root, dir]);

        let mut search_state = SearchState::default();
        search_state.rebuild(&mut tree, Some(root), "match");

        assert_eq!(
            search_state.matches(),
            &[dir, nested, large_file, small_file]
        );
    }

    #[test]
    fn filtered_layout_only_emits_matches_and_ancestors() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/".into());
        let dir = tree.add_node(Some(root), "dir".into(), NodeKind::Dir, 0);
        let matching = tree.add_node(Some(dir), "target-file".into(), NodeKind::File, 10);
        let hidden = tree.add_node(Some(root), "other-file".into(), NodeKind::File, 50);
        tree.apply_direct_size_delta(dir, 10);
        tree.apply_direct_size_delta(root, 60);
        tree.repair_sorted_children(&[root, dir]);

        let mut search_state = SearchState::default();
        search_state.rebuild(&mut tree, Some(root), "target");
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 700.0));
        let mut visuals = Vec::new();
        let mut scratch = LayoutScratch::default();

        layout_treemap(
            &mut tree,
            TreemapLayoutParams {
                root,
                canvas_rect: canvas,
                max_depth: 2,
                search_state: &search_state,
                filter_to_search: true,
                out: &mut visuals,
                scratch: &mut scratch,
            },
        );

        let emitted_ids: Vec<NodeId> = visuals
            .iter()
            .filter_map(|visual| match visual.kind {
                VisualKind::Node(node_id) => Some(node_id),
                VisualKind::SmallFiles { .. } => None,
            })
            .collect();
        assert!(emitted_ids.contains(&dir));
        assert!(emitted_ids.contains(&matching));
        assert!(!emitted_ids.contains(&hidden));
    }

    #[test]
    fn layout_handles_zero_sizes_and_tiny_rectangles() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/".into());
        tree.add_node(Some(root), "zero".into(), NodeKind::File, 0);
        tree.add_node(Some(root), "also-zero".into(), NodeKind::Dir, 0);
        tree.repair_sorted_children(&[root]);

        let tiny = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let mut visuals = Vec::new();
        let mut scratch = LayoutScratch::default();
        layout_treemap(
            &mut tree,
            TreemapLayoutParams {
                root,
                canvas_rect: tiny,
                max_depth: 2,
                search_state: &SearchState::default(),
                filter_to_search: false,
                out: &mut visuals,
                scratch: &mut scratch,
            },
        );

        assert!(visuals.is_empty());
    }

    #[test]
    fn layout_preserves_area_ratio_for_sized_children() {
        let (mut tree, root) = sample_tree(&[90, 60, 30, 20]);
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 700.0));
        let mut visuals = Vec::new();
        let mut scratch = LayoutScratch::default();
        layout_treemap(
            &mut tree,
            TreemapLayoutParams {
                root,
                canvas_rect: canvas,
                max_depth: 1,
                search_state: &SearchState::default(),
                filter_to_search: false,
                out: &mut visuals,
                scratch: &mut scratch,
            },
        );

        let first = visuals.iter().find(|visual| {
            matches!(visual.kind, VisualKind::Node(node_id) if tree.node(node_id).name == "child-0")
        }).unwrap();
        let second = visuals.iter().find(|visual| {
            matches!(visual.kind, VisualKind::Node(node_id) if tree.node(node_id).name == "child-1")
        }).unwrap();
        let area_ratio = first.rect.area() / second.rect.area().max(f32::EPSILON);

        assert!((area_ratio - 1.5).abs() < 0.2);
    }

    #[test]
    fn layout_avoids_collapsing_every_small_item_into_tiny_strips() {
        let (mut tree, root) = sample_tree(&[100, 80, 60, 30, 20, 10]);
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(900.0, 600.0));
        let mut visuals = Vec::new();
        let mut scratch = LayoutScratch::default();
        layout_treemap(
            &mut tree,
            TreemapLayoutParams {
                root,
                canvas_rect: canvas,
                max_depth: 1,
                search_state: &SearchState::default(),
                filter_to_search: false,
                out: &mut visuals,
                scratch: &mut scratch,
            },
        );

        assert!(visuals
            .iter()
            .all(|visual| visual.rect.width() >= 2.0 && visual.rect.height() >= 2.0));
    }

    #[test]
    fn layout_aggregates_many_small_files_without_changing_tree_data() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/root".into());
        for index in 0..8 {
            tree.add_node(
                Some(root),
                format!("small-{index}.txt"),
                NodeKind::File,
                100,
            );
        }
        tree.apply_direct_size_delta(root, 800);
        tree.repair_sorted_children(&[root]);
        let original_len = tree.len();
        let mut visuals = Vec::new();
        let mut scratch = LayoutScratch::default();

        layout_treemap(
            &mut tree,
            TreemapLayoutParams {
                root,
                canvas_rect: Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 500.0)),
                max_depth: 1,
                search_state: &SearchState::default(),
                filter_to_search: false,
                out: &mut visuals,
                scratch: &mut scratch,
            },
        );

        assert_eq!(tree.len(), original_len);
        assert!(visuals.iter().any(|visual| matches!(
            visual.kind,
            VisualKind::SmallFiles {
                parent_id: id,
                count: 8,
                size: 800
            } if id == root
        )));
    }

    #[test]
    fn squarified_algorithm_forms_multi_item_groups_and_avoids_full_width_strips() {
        let sizes = [1405_u64, 475, 339, 127, 99, 64, 51, 32, 7, 6, 5, 4].map(|size| size * 1024);
        let (mut tree, root) = sample_tree(&sizes);
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 600.0));
        let mut visuals = Vec::new();
        let mut scratch = LayoutScratch::default();
        layout_treemap(
            &mut tree,
            TreemapLayoutParams {
                root,
                canvas_rect: canvas,
                max_depth: 1,
                search_state: &SearchState::default(),
                filter_to_search: false,
                out: &mut visuals,
                scratch: &mut scratch,
            },
        );

        let total_area: f32 = visuals.iter().map(|visual| visual.rect.area()).sum();
        assert!((total_area - (1000.0 * 600.0)).abs() < 50_000.0);
        assert!(visuals
            .iter()
            .all(|visual| visual.rect.width() >= 0.0 && visual.rect.height() >= 0.0));

        let full_width_like = visuals
            .iter()
            .filter(|visual| (visual.rect.width() - (canvas.width() - 18.0)).abs() < 4.0)
            .count();
        assert!(full_width_like < visuals.len());

        let good_aspect_count = visuals
            .iter()
            .filter(|visual| rect_aspect_ratio(visual.rect) < 10.0)
            .count();
        assert!(good_aspect_count * 2 >= visuals.len());
    }

    #[test]
    fn search_state_incrementally_tracks_new_matches() {
        let mut tree = TreeStore::new();
        let root = tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        tree.set_root_path("/".into());
        let child = tree.add_node(Some(root), "alpha".into(), NodeKind::File, 1);
        let mut search_state = SearchState::default();

        search_state.rebuild(&mut tree, Some(root), "match");
        assert_eq!(search_state.match_count(), 0);

        let matching = tree.add_node(Some(root), "match-file".into(), NodeKind::File, 1);
        tree.repair_sorted_children(&[root]);
        let updates = search_state.ingest_new_nodes(&mut tree, &[child, matching]);

        assert_eq!(updates, 1);
        assert_eq!(search_state.matches(), &[matching]);
        assert!(search_state.is_match(matching));
        assert!(search_state.is_ancestor_of_match(root));

        let duplicate_updates = search_state.ingest_new_nodes(&mut tree, &[matching]);
        assert_eq!(duplicate_updates, 0);
        assert_eq!(search_state.matches(), &[matching]);
    }
}
