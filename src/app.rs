use crate::format::format_bytes;
use crate::platform::{move_to_trash, open_path, reveal_in_finder};
use crate::scanner::{
    self, CacheMode, PerfStats, ProgressSnapshot, ScanBatch, ScanHandle, ScanMessage, ScanOptions,
};
use crate::tree::{NodeId, NodeKind, TreeStore};
use crate::treemap::{layout_treemap, Camera, SearchState, VisualKind, VisualNode};

use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::time::{Duration, Instant};

const SEARCH_REFRESH_INTERVAL: Duration = Duration::from_millis(150);
const LAYOUT_REFRESH_INTERVAL: Duration = Duration::from_millis(33);
const CONTEXT_MENU_MIN_WIDTH: f32 = 240.0;
const CONTEXT_MENU_MAX_TITLE_CHARS: usize = 36;

pub struct DiskLensApp {
    path_input: String,
    search_input: String,
    tx: Sender<ScanMessage>,
    rx: Receiver<ScanMessage>,
    tree: TreeStore,
    focused_root: Option<NodeId>,
    selected_id: Option<NodeId>,
    hovered_id: Option<NodeId>,
    context_menu_target_id: Option<NodeId>,
    hovered_visual_kind: Option<VisualKind>,
    camera: Camera,
    max_depth: usize,
    scanning: bool,
    status: String,
    progress_summary: Option<ProgressSummary>,
    search_state: SearchState,
    active_scan_id: u64,
    scan_counter: u64,
    scan_handle: Option<ScanHandle>,
    back_history: Vec<NodeId>,
    forward_history: Vec<NodeId>,
    cached_visuals: Vec<VisualNode>,
    last_canvas_rect: Option<Rect>,
    layout_dirty: bool,
    search_dirty: bool,
    search_last_refresh: Instant,
    last_layout_refresh: Instant,
    breadcrumb_cache: String,
    pending_repaint: bool,
    perf_stats: PerfStats,
}

#[derive(Debug, Clone)]
struct ProgressSummary {
    files_scanned: u64,
    dirs_scanned: u64,
    bytes_seen: u64,
    current_path: String,
}

impl Default for DiskLensApp {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            path_input: dirs_home_fallback(),
            search_input: String::new(),
            tx,
            rx,
            tree: TreeStore::new(),
            focused_root: None,
            selected_id: None,
            hovered_id: None,
            context_menu_target_id: None,
            hovered_visual_kind: None,
            camera: Camera::default(),
            max_depth: 1,
            scanning: false,
            status: "Ready".to_string(),
            progress_summary: None,
            search_state: SearchState::default(),
            active_scan_id: 0,
            scan_counter: 0,
            scan_handle: None,
            back_history: Vec::new(),
            forward_history: Vec::new(),
            cached_visuals: Vec::new(),
            last_canvas_rect: None,
            layout_dirty: true,
            search_dirty: false,
            search_last_refresh: Instant::now(),
            last_layout_refresh: Instant::now(),
            breadcrumb_cache: String::new(),
            pending_repaint: false,
            perf_stats: PerfStats::default(),
        }
    }
}

impl eframe::App for DiskLensApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_keyboard(ctx);
        self.handle_scan_messages();
        self.maybe_refresh_search(ctx);
        self.maybe_request_deferred_repaint(ctx);
        self.drive_background_updates(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            self.show_toolbar(ui);
        });

        egui::Panel::right("details_panel")
            .default_size(280.0)
            .show_inside(ui, |ui| self.show_details_panel(ui));

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.show_treemap(ui);
        });
    }
}

impl DiskLensApp {
    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Path:");
            let path_edit = ui.text_edit_singleline(&mut self.path_input);
            if path_edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.start_scan();
            }

            if ui.button("Scan").clicked() {
                self.start_scan();
            }

            if ui
                .add_enabled(!self.back_history.is_empty(), egui::Button::new("Back"))
                .clicked()
            {
                self.navigate_back();
            }

            if ui
                .add_enabled(!self.forward_history.is_empty(), egui::Button::new("Forward"))
                .clicked()
            {
                self.navigate_forward();
            }

            if ui
                .add_enabled(self.parent_of_focused_root().is_some(), egui::Button::new("Up"))
                .clicked()
            {
                if let Some(parent) = self.parent_of_focused_root() {
                    self.enter_root(parent, true);
                }
            }

            if ui.button("Reset View").clicked() {
                self.reset_camera();
            }

            ui.separator();
            ui.label("Search:");
            if ui.text_edit_singleline(&mut self.search_input).changed() {
                self.mark_search_dirty();
            }
            if !self.search_input.is_empty() && ui.button("Clear").clicked() {
                self.search_input.clear();
                self.search_state.clear(self.tree.len());
                self.search_dirty = false;
                self.layout_dirty = true;
            }

            ui.separator();
            ui.label("Depth:");
            if ui.add(egui::Slider::new(&mut self.max_depth, 1..=10).text("")).changed() {
                self.layout_dirty = true;
                self.last_layout_refresh = Instant::now()
                    .checked_sub(LAYOUT_REFRESH_INTERVAL)
                    .unwrap_or_else(Instant::now);
            }

            ui.separator();
            ui.label(&self.status);
        });
    }

    fn show_details_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Details");
        ui.separator();

        let subject_id = self.selected_id.or(self.focused_root);
        if let Some(node_id) = subject_id {
            let node = self.tree.node(node_id);
            let node_path = self.tree.node_path(node_id);
            let root_size = self
                .focused_root
                .map(|root_id| self.tree.node(root_id).size.max(1))
                .unwrap_or(1);
            let matched = self.search_state.is_match(node_id);

            ui.label(format!("Name: {}", node.name));
            ui.label(format!("Size: {}", format_bytes(node.size)));
            let parent_share = if let Some(parent_id) = node.parent {
                let parent_size = self.tree.node(parent_id).size.max(1);
                (node.size as f32 / parent_size as f32) * 100.0
            } else {
                100.0
            };
            ui.label(format!("Share of root: {:.1}%", (node.size as f32 / root_size as f32) * 100.0));
            ui.label(format!("Share of parent: {:.1}%", parent_share));
            ui.label(format!("Type: {}", describe_node_kind(node.kind, !node.children.is_empty())));
            ui.label(format!("Scanned: {}", if node.scanned { "yes" } else { "in progress" }));
            if let Some(error) = &node.error {
                ui.label(format!("Error: {error}"));
            }
            ui.label(format!("Path: {}", node_path.display()));
            if !self.search_input.is_empty() {
                ui.label(format!("Search match: {}", if matched { "yes" } else { "no" }));
                ui.label(format!("Matches in view: {}", self.search_state.match_count()));
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Open").clicked() {
                    open_path(&node_path);
                }
                if ui.button("Reveal").clicked() {
                    reveal_in_finder(&node_path);
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Copy Path").clicked() {
                    ui.ctx().copy_text(node_path.display().to_string());
                }
                if ui.button("Trash").clicked() {
                    let _ = move_to_trash(&node_path);
                }
            });

            if let Some(parent) = node.parent {
                ui.separator();
                let parent_name = self.tree.node(parent).name.clone();
                if ui.button(format!("Parent: {parent_name}")).clicked() {
                    self.selected_id = Some(parent);
                }
            }

            if !node.children.is_empty() {
                ui.separator();
                ui.label("Largest Children");
                self.tree.ensure_sorted_children(node_id);
                let child_ids: Vec<NodeId> = self
                    .tree
                    .sorted_children(node_id)
                    .iter()
                    .take(12)
                    .copied()
                    .collect();
                for child_id in child_ids {
                    let child = self.tree.node(child_id);
                    let label = format!("{}  {}", child.name, format_bytes(child.size));
                    let response = ui.selectable_label(self.selected_id == Some(child_id), label);
                    if response.clicked() {
                        self.selected_id = Some(child_id);
                    }
                    if response.double_clicked() && !child.children.is_empty() {
                        self.enter_root(child_id, true);
                    }
                }
            }
        } else {
            ui.label("Run a scan to populate the treemap.");
        }

        if let Some(progress) = &self.progress_summary {
            ui.separator();
            ui.heading("Scan");
            ui.label(format!("Files: {}", progress.files_scanned));
            ui.label(format!("Dirs: {}", progress.dirs_scanned));
            ui.label(format!("Seen: {}", format_bytes(progress.bytes_seen)));
            ui.label(format!("Current: {}", progress.current_path));
        }
    }

    fn show_treemap(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(available, Sense::click_and_drag());
        let painter = ui.painter_at(available);
        painter.rect_filled(available, 0.0, Color32::from_rgb(18, 18, 18));

        let Some(root_id) = self.focused_root else {
            painter.text(
                available.center(),
                egui::Align2::CENTER_CENTER,
                "Input path and click Scan",
                egui::TextStyle::Heading.resolve(ui.style()),
                Color32::LIGHT_GRAY,
            );
            return;
        };

        let canvas_changed = self.last_canvas_rect != Some(available);
        if canvas_changed {
            self.last_canvas_rect = Some(available);
            self.layout_dirty = true;
            self.last_layout_refresh = Instant::now()
                .checked_sub(LAYOUT_REFRESH_INTERVAL)
                .unwrap_or_else(Instant::now);
        }

        let should_refresh_layout = self.layout_dirty
            && (!self.scanning || self.last_layout_refresh.elapsed() >= LAYOUT_REFRESH_INTERVAL);

        if should_refresh_layout {
            let layout_start = Instant::now();
            self.cached_visuals = layout_treemap(
                &mut self.tree,
                root_id,
                available,
                self.camera,
                self.max_depth,
                &self.search_state,
            );
            self.layout_dirty = false;
            self.last_layout_refresh = Instant::now();
            self.perf_stats.layout_recompute_count += 1;
            self.perf_stats.layout_total_ms += layout_start.elapsed().as_secs_f64() * 1000.0;
        }

        self.hovered_visual_kind =
            find_hovered_visual(&self.cached_visuals, response.hover_pos()).map(|visual| visual.kind);
        self.hovered_id = self.hovered_visual_kind.map(|kind| match kind {
            VisualKind::Node(node_id) => node_id,
        });

        if response.secondary_clicked() {
            self.context_menu_target_id = self.hovered_id;
        }

        if response.dragged() && self.search_input.is_empty() {
            let drag_delta = response.drag_delta();
            if drag_delta != Vec2::ZERO {
                self.camera.pan += drag_delta;
                self.layout_dirty = true;
                self.last_layout_refresh = Instant::now()
                    .checked_sub(LAYOUT_REFRESH_INTERVAL)
                    .unwrap_or_else(Instant::now);
            }
        }

        let zoom_delta = ui.ctx().input(|input| input.zoom_delta());
        if (zoom_delta - 1.0).abs() > f32::EPSILON {
            if let Some(pointer) = response.hover_pos() {
                self.camera.zoom_around(pointer, zoom_delta);
                self.layout_dirty = true;
                self.last_layout_refresh = Instant::now()
                    .checked_sub(LAYOUT_REFRESH_INTERVAL)
                    .unwrap_or_else(Instant::now);
            }
        }

        for visual in &self.cached_visuals {
            self.paint_visual(ui, &painter, visual);
        }
        self.paint_breadcrumb_overlay(ui, &painter, available);

        if response.double_clicked() {
            if let Some(node_id) = self.hovered_id {
                if !self.tree.node(node_id).children.is_empty() {
                    self.enter_root(node_id, true);
                } else {
                    self.selected_id = Some(node_id);
                }
            } else {
                self.reset_camera();
            }
        } else if response.clicked() {
            if let Some(node_id) = self.hovered_id {
                self.selected_id = Some(node_id);
            } else {
                self.selected_id = None;
            }
        }

        response.context_menu(|ui| {
            if let Some(node_id) = self.context_menu_target_id {
                let node = self.tree.node(node_id);
                let node_path = self.tree.node_path(node_id);
                ui.set_min_width(CONTEXT_MENU_MIN_WIDTH);
                ui.vertical(|ui| {
                    ui.label(RichText::new(truncate_middle(&node.name, CONTEXT_MENU_MAX_TITLE_CHARS)).strong());
                    ui.label(RichText::new(format_bytes(node.size)).small().weak());
                    ui.separator();
                    if ui.button("Open").clicked() {
                        open_path(&node_path);
                        ui.close();
                    }
                    if ui.button("Reveal in Finder").clicked() {
                        reveal_in_finder(&node_path);
                        ui.close();
                    }
                    if ui.button("Copy Path").clicked() {
                        ui.ctx().copy_text(node_path.display().to_string());
                        ui.close();
                    }
                    if ui.button("Move to Trash").clicked() {
                        let _ = move_to_trash(&node_path);
                        ui.close();
                    }
                });
            }
        });

        let context_menu_open = response.context_menu_opened();

        if !context_menu_open {
            self.context_menu_target_id = None;
        }

        if !context_menu_open {
            if let Some(node_id) = self.hovered_id {
                if let Some(pos) = response.hover_pos() {
                    self.show_hover_tooltip(ui, node_id, pos);
                }
            }
        }
    }

    fn show_hover_tooltip(&self, ui: &egui::Ui, node_id: NodeId, pos: Pos2) {
        let node = self.tree.node(node_id);
        let node_path = self.tree.node_path(node_id);
        egui::Area::new(egui::Id::new("hover_tooltip"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos + egui::vec2(16.0, 16.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::default()
                    .fill(Color32::from_rgb(40, 40, 40))
                    .show(ui, |ui| {
                        ui.label(&node.name);
                        ui.label(format_bytes(node.size));
                        ui.label(node_path.display().to_string());
                        if let Some(error) = &node.error {
                            ui.label(error);
                        }
                        ui.label(if node.scanned { "Scanned" } else { "Scanning..." });
                    });
            });
    }

    fn paint_breadcrumb_overlay(&self, ui: &egui::Ui, painter: &egui::Painter, available: Rect) {
        if self.breadcrumb_cache.is_empty() {
            return;
        }

        let overlay_rect = Rect::from_min_max(
            available.left_top() + egui::vec2(12.0, 12.0),
            available.left_top() + egui::vec2((available.width() * 0.6).min(680.0), 40.0),
        );
        painter.rect_filled(
            overlay_rect,
            6.0,
            Color32::from_rgba_unmultiplied(0, 0, 0, 160),
        );
        painter.text(
            overlay_rect.left_center() + egui::vec2(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &self.breadcrumb_cache,
            egui::TextStyle::Body.resolve(ui.style()),
            Color32::WHITE,
        );
    }

    fn paint_visual(&self, ui: &egui::Ui, painter: &egui::Painter, visual: &VisualNode) {
        let is_hovered = matches!(visual.kind, VisualKind::Node(node_id) if self.hovered_id == Some(node_id));
        let is_selected = matches!(visual.kind, VisualKind::Node(node_id) if self.selected_id == Some(node_id));
        let fill = fill_color_for_visual(visual, is_hovered, is_selected);
        let stroke = stroke_for_visual(visual, is_hovered, is_selected);

        painter.rect_filled(visual.rect, 2.0, fill);
        painter.rect_stroke(visual.rect, 2.0, stroke, egui::StrokeKind::Inside);

        if let Some(label_text) = &visual.label_text {
            painter.text(
                visual.rect.center(),
                egui::Align2::CENTER_CENTER,
                label_text,
                egui::TextStyle::Small.resolve(ui.style()),
                Color32::WHITE,
            );
        }
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
            if let Some(selected_id) = self.selected_id {
                if !self.tree.node(selected_id).children.is_empty() {
                    self.enter_root(selected_id, true);
                }
            }
        }

        if ctx.input(|input| input.key_pressed(egui::Key::Backspace)) {
            self.navigate_back();
        }

        if ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::ArrowLeft)) {
            self.navigate_back();
        }

        if ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::ArrowRight)) {
            self.navigate_forward();
        }

        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            if self.selected_id.is_some() {
                self.selected_id = None;
            } else if !self.search_input.is_empty() {
                self.search_input.clear();
                self.search_state.clear(self.tree.len());
                self.layout_dirty = true;
            }
        }
    }

    fn handle_scan_messages(&mut self) {
        let mut saw_batch = false;

        while let Ok(message) = self.rx.try_recv() {
            let message_scan_id = scan_id_for_message(&message);
            if message_scan_id != self.active_scan_id {
                continue;
            }

            match message {
                ScanMessage::Started { path, root_node, .. } => {
                    self.scanning = true;
                    self.status = format!("Scanning {}", path.display());
                    self.tree.clear();
                    self.tree.push_node(None, root_node);
                    self.focused_root = self.tree.root;
                    self.layout_dirty = true;
                    self.mark_search_dirty();
                    self.rebuild_breadcrumb_cache();
                }
                ScanMessage::Batch { batch, .. } => {
                    self.apply_scan_batch(batch);
                    saw_batch = true;
                }
                ScanMessage::Finished {
                    total_bytes,
                    perf_stats,
                    ..
                } => {
                    self.scanning = false;
                    self.merge_scan_perf_stats(perf_stats);
                    self.prune_invalid_selection();
                    self.refresh_search_matches();
                    self.layout_dirty = true;
                    self.status = format!("Finished: {}", format_bytes(total_bytes));
                    self.pending_repaint = true;
                    eprintln!("{}", format_perf_stats(&self.perf_stats));
                }
                ScanMessage::Cancelled { perf_stats, .. } => {
                    self.scanning = false;
                    self.merge_scan_perf_stats(perf_stats);
                    self.status = "Scan cancelled".to_string();
                    self.pending_repaint = true;
                    eprintln!("{}", format_perf_stats(&self.perf_stats));
                }
                ScanMessage::Error {
                    message,
                    perf_stats,
                    ..
                } => {
                    self.scanning = false;
                    self.merge_scan_perf_stats(perf_stats);
                    self.status = format!("Error: {message}");
                    self.pending_repaint = true;
                    eprintln!("{}", format_perf_stats(&self.perf_stats));
                }
            }
        }

        if saw_batch {
            self.pending_repaint = true;
        }
    }

    fn apply_scan_batch(&mut self, batch: ScanBatch) {
        let mut dirty_nodes = Vec::with_capacity(
            batch.discovered_nodes.len().saturating_mul(2)
                + batch.size_deltas.len()
                + batch.scanned_nodes.len(),
        );
        let mut discovered_node_ids = Vec::with_capacity(batch.discovered_nodes.len());

        for discovered in batch.discovered_nodes {
            let node_id = discovered.node_id;
            let parent_id = discovered.parent_id;
            self.tree.insert_node(node_id, Some(parent_id), discovered.node);
            discovered_node_ids.push(node_id);
            dirty_nodes.push(parent_id);
            dirty_nodes.push(node_id);
        }

        for (node_id, delta) in batch.size_deltas {
            if node_id < self.tree.len() {
                self.tree.apply_direct_size_delta(node_id, delta);
                dirty_nodes.push(node_id);
            }
        }

        for node_id in batch.scanned_nodes {
            if node_id < self.tree.len() {
                self.tree.mark_scanned(node_id);
                dirty_nodes.push(node_id);
            }
        }

        dirty_nodes.sort_unstable();
        dirty_nodes.dedup();
        let touched_visible_subtree = dirty_nodes
            .iter()
            .copied()
            .any(|node_id| self.batch_touches_visible_subtree(node_id));
        if let Some(progress) = batch.progress {
            self.apply_progress(progress);
        }

        if !discovered_node_ids.is_empty() && !self.search_input.trim().is_empty() {
            self.perf_stats.search_incremental_updates +=
                self.search_state.ingest_new_nodes(&mut self.tree, &discovered_node_ids) as u64;
        }

        if touched_visible_subtree {
            self.layout_dirty = true;
        }
    }

    fn apply_progress(&mut self, progress: ProgressSnapshot) {
        self.progress_summary = Some(ProgressSummary {
            files_scanned: progress.files_scanned,
            dirs_scanned: progress.dirs_scanned,
            bytes_seen: progress.bytes_seen,
            current_path: progress.current_path.display().to_string(),
        });
        self.status = format!(
            "Scanning {} files, {} dirs, {}",
            progress.files_scanned,
            progress.dirs_scanned,
            format_bytes(progress.bytes_seen)
        );
    }

    fn merge_scan_perf_stats(&mut self, perf_stats: PerfStats) {
        self.perf_stats.messages_sent = perf_stats.messages_sent;
        self.perf_stats.batches_sent = perf_stats.batches_sent;
        self.perf_stats.entries_seen = perf_stats.entries_seen;
        self.perf_stats.nodes_discovered = perf_stats.nodes_discovered;
        self.perf_stats.size_delta_merges = perf_stats.size_delta_merges;
        self.perf_stats.parent_stack_hits = perf_stats.parent_stack_hits;
        self.perf_stats.parent_lookup_fallbacks = perf_stats.parent_lookup_fallbacks;
        self.perf_stats.progress_snapshots_sent = perf_stats.progress_snapshots_sent;
        self.perf_stats.metadata_total_ms = perf_stats.metadata_total_ms;
        self.perf_stats.size_measure_total_ms = perf_stats.size_measure_total_ms;
        self.perf_stats.batch_flush_total_ms = perf_stats.batch_flush_total_ms;
        self.perf_stats.scan_elapsed_ms = perf_stats.scan_elapsed_ms;
        self.perf_stats.db_cache_hits = perf_stats.db_cache_hits;
        self.perf_stats.db_cache_misses = perf_stats.db_cache_misses;
        self.perf_stats.db_flush_count = perf_stats.db_flush_count;
    }

    fn start_scan(&mut self) {
        if let Some(handle) = &self.scan_handle {
            handle.cancel();
        }

        self.scan_counter += 1;
        self.active_scan_id = self.scan_counter;
        self.scan_handle = Some(scanner::start_scan(
            std::path::PathBuf::from(self.path_input.trim()),
            self.active_scan_id,
            ScanOptions {
                cache_mode: CacheMode::Disabled,
                ..Default::default()
            },
            self.tx.clone(),
        ));

        self.tree.clear();
        self.focused_root = None;
        self.selected_id = None;
        self.hovered_id = None;
        self.context_menu_target_id = None;
        self.hovered_visual_kind = None;
        self.search_state.clear(0);
        self.progress_summary = None;
        self.back_history.clear();
        self.forward_history.clear();
        self.cached_visuals.clear();
        self.reset_camera();
        self.layout_dirty = true;
        self.search_dirty = false;
        self.scanning = true;
        self.status = format!("Scanning {}", self.path_input.trim());
        self.breadcrumb_cache.clear();
        self.pending_repaint = true;
        self.perf_stats = PerfStats::default();
    }

    fn enter_root(&mut self, node_id: NodeId, push_history: bool) {
        if self.focused_root == Some(node_id) {
            self.reset_camera();
            return;
        }
        if push_history {
            if let Some(current) = self.focused_root {
                self.back_history.push(current);
            }
            self.forward_history.clear();
        }
        self.focused_root = Some(node_id);
        self.selected_id = Some(node_id);
        self.reset_camera();
        self.refresh_search_matches();
        self.layout_dirty = true;
        self.rebuild_breadcrumb_cache();
    }

    fn navigate_back(&mut self) {
        let Some(previous) = self.back_history.pop() else {
            return;
        };
        if let Some(current) = self.focused_root {
            self.forward_history.push(current);
        }
        self.focused_root = Some(previous);
        self.selected_id = Some(previous);
        self.reset_camera();
        self.refresh_search_matches();
        self.layout_dirty = true;
        self.rebuild_breadcrumb_cache();
    }

    fn navigate_forward(&mut self) {
        let Some(next) = self.forward_history.pop() else {
            return;
        };
        if let Some(current) = self.focused_root {
            self.back_history.push(current);
        }
        self.focused_root = Some(next);
        self.selected_id = Some(next);
        self.reset_camera();
        self.refresh_search_matches();
        self.layout_dirty = true;
        self.rebuild_breadcrumb_cache();
    }

    fn reset_camera(&mut self) {
        self.camera = Camera::default();
        self.layout_dirty = true;
        self.last_layout_refresh = Instant::now()
            .checked_sub(LAYOUT_REFRESH_INTERVAL)
            .unwrap_or_else(Instant::now);
    }

    fn refresh_search_matches(&mut self) {
        self.search_dirty = false;
        self.search_last_refresh = Instant::now();
        self.search_state
            .rebuild(&mut self.tree, self.focused_root, self.search_input.trim());
        self.perf_stats.search_rebuild_count += 1;
    }

    fn maybe_refresh_search(&mut self, ctx: &egui::Context) {
        if !self.search_dirty {
            return;
        }

        if self.search_last_refresh.elapsed() >= SEARCH_REFRESH_INTERVAL || !self.scanning {
            self.refresh_search_matches();
            self.layout_dirty = true;
            self.pending_repaint = true;
            ctx.request_repaint();
        }
    }

    fn mark_search_dirty(&mut self) {
        self.search_dirty = true;
        self.search_last_refresh = self
            .search_last_refresh
            .checked_sub(SEARCH_REFRESH_INTERVAL)
            .unwrap_or_else(Instant::now);
    }

    fn prune_invalid_selection(&mut self) {
        if let Some(selected_id) = self.selected_id {
            if selected_id >= self.tree.len() {
                self.selected_id = None;
            }
        }
        if let Some(root_id) = self.focused_root {
            if root_id >= self.tree.len() {
                self.focused_root = self.tree.root;
                self.rebuild_breadcrumb_cache();
            }
        }
    }

    fn parent_of_focused_root(&self) -> Option<NodeId> {
        self.focused_root.and_then(|node_id| self.tree.node(node_id).parent)
    }

    fn batch_touches_visible_subtree(&self, node_id: NodeId) -> bool {
        if let Some(root_id) = self.focused_root {
            self.tree.is_descendant_or_same(node_id, root_id)
                || self.tree.is_descendant_or_same(root_id, node_id)
        } else {
            true
        }
    }

    fn rebuild_breadcrumb_cache(&mut self) {
        let Some(root_id) = self.focused_root else {
            self.breadcrumb_cache.clear();
            return;
        };

        self.breadcrumb_cache = self
            .tree
            .ancestors(root_id)
            .into_iter()
            .map(|id| self.tree.node(id).name.clone())
            .collect::<Vec<_>>()
            .join(" / ");
    }

    fn maybe_request_deferred_repaint(&mut self, ctx: &egui::Context) {
        if self.pending_repaint {
            ctx.request_repaint();
            self.pending_repaint = false;
        }
    }

    fn drive_background_updates(&self, ctx: &egui::Context) {
        if self.scanning {
            // Keep the UI alive while scan batches arrive, even when there is no user input.
            ctx.request_repaint_after(LAYOUT_REFRESH_INTERVAL);
        } else if self.search_dirty {
            ctx.request_repaint_after(SEARCH_REFRESH_INTERVAL);
        }
    }

    #[cfg(test)]
    fn apply_scan_message_for_test(&mut self, message: ScanMessage) {
        if scan_id_for_message(&message) != self.active_scan_id {
            return;
        }

        match message {
            ScanMessage::Started { root_node, .. } => {
                self.tree.clear();
                self.tree.push_node(None, root_node);
                self.focused_root = self.tree.root;
                self.rebuild_breadcrumb_cache();
            }
            ScanMessage::Batch { batch, .. } => self.apply_scan_batch(batch),
            ScanMessage::Finished { .. } | ScanMessage::Cancelled { .. } | ScanMessage::Error { .. } => {}
        }
    }
}

fn scan_id_for_message(message: &ScanMessage) -> u64 {
    match message {
        ScanMessage::Started { scan_id, .. }
        | ScanMessage::Batch { scan_id, .. }
        | ScanMessage::Finished { scan_id, .. }
        | ScanMessage::Cancelled { scan_id, .. }
        | ScanMessage::Error { scan_id, .. } => *scan_id,
    }
}

fn find_hovered_visual(visuals: &[VisualNode], pos: Option<Pos2>) -> Option<&VisualNode> {
    let pos = pos?;
    visuals
        .iter()
        .rev()
        .find(|visual| visual.rect.contains(pos) && visual.rect.width() >= 2.0 && visual.rect.height() >= 2.0)
}

fn fill_color_for_visual(visual: &VisualNode, hovered: bool, selected: bool) -> Color32 {
    let mut color = if visual.is_dir {
        match visual.depth % 5 {
            0 => Color32::from_rgb(45, 101, 168),
            1 => Color32::from_rgb(56, 128, 91),
            2 => Color32::from_rgb(145, 112, 56),
            3 => Color32::from_rgb(120, 79, 137),
            _ => Color32::from_rgb(67, 132, 140),
        }
    } else {
        Color32::from_rgb(112, 112, 118)
    };

    if visual.hidden_by_search {
        color = color.gamma_multiply(0.35);
    } else if visual.ancestor_of_match {
        color = color.gamma_multiply(0.8);
    } else if visual.matched {
        color = color.gamma_multiply(1.2);
    }

    if hovered {
        color = color.gamma_multiply(1.15);
    }
    if selected {
        color = color.gamma_multiply(1.2);
    }

    color
}

fn stroke_for_visual(visual: &VisualNode, hovered: bool, selected: bool) -> Stroke {
    if selected {
        Stroke::new(2.5, Color32::WHITE)
    } else if hovered {
        Stroke::new(2.0, Color32::from_rgb(255, 233, 171))
    } else if visual.matched {
        Stroke::new(1.8, Color32::from_rgb(255, 214, 82))
    } else if visual.is_dir {
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 42))
    } else {
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 24))
    }
}

fn describe_node_kind(kind: NodeKind, has_children: bool) -> &'static str {
    match kind {
        NodeKind::Dir if has_children => "Directory",
        NodeKind::Dir => "Empty directory",
        NodeKind::File => "File",
        NodeKind::Symlink => "Symlink",
        NodeKind::Error => "Error entry",
    }
}

fn dirs_home_fallback() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}

fn truncate_middle(input: &str, max_chars: usize) -> String {
    let char_count = input.chars().count();
    if char_count <= max_chars {
        return input.to_string();
    }

    let left_len = max_chars.saturating_sub(1) / 2;
    let right_len = max_chars.saturating_sub(left_len + 1);
    let left = input.chars().take(left_len).collect::<String>();
    let right = input
        .chars()
        .rev()
        .take(right_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{left}…{right}")
}

fn format_perf_stats(stats: &PerfStats) -> String {
    format!(
        "perf: messages={} batches={} entries={} nodes={} size_merges={} parent_stack_hits={} parent_fallbacks={} progress_snapshots={} scan_ms={:.2} metadata_ms={:.2} size_ms={:.2} flush_ms={:.2} layouts={} layout_ms={:.2} search_rebuilds={} search_incremental={} db_hits={} db_misses={} db_flushes={}",
        stats.messages_sent,
        stats.batches_sent,
        stats.entries_seen,
        stats.nodes_discovered,
        stats.size_delta_merges,
        stats.parent_stack_hits,
        stats.parent_lookup_fallbacks,
        stats.progress_snapshots_sent,
        stats.scan_elapsed_ms,
        stats.metadata_total_ms,
        stats.size_measure_total_ms,
        stats.batch_flush_total_ms,
        stats.layout_recompute_count,
        stats.layout_total_ms,
        stats.search_rebuild_count,
        stats.search_incremental_updates,
        stats.db_cache_hits,
        stats.db_cache_misses,
        stats.db_flush_count
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{DiscoveredNode, ScanBatch};
    use crate::tree::NodeRecord;

    fn root_started(scan_id: u64) -> ScanMessage {
        ScanMessage::Started {
            scan_id,
            path: "/root".into(),
            root_node: TreeStore::root_record("/root".into(), "root".into()),
        }
    }

    #[test]
    fn incremental_messages_build_tree_correctly() {
        let mut app = DiskLensApp {
            active_scan_id: 1,
            ..Default::default()
        };
        app.apply_scan_message_for_test(root_started(1));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![DiscoveredNode {
                    node_id: 1,
                    parent_id: 0,
                    node: NodeRecord {
                        name: "child".into(),
                        path: "/root/child".into(),
                        kind: NodeKind::File,
                        size: 5,
                        scanned: false,
                        error: None,
                    },
                }],
                size_deltas: vec![(0, 5)],
                scanned_nodes: vec![1],
                progress: None,
            },
        });

        assert_eq!(app.tree.len(), 2);
        assert_eq!(app.tree.node(0).size, 5);
        assert!(app.tree.node(1).scanned);
    }

    #[test]
    fn stale_scan_messages_are_ignored() {
        let mut app = DiskLensApp {
            active_scan_id: 2,
            ..Default::default()
        };
        app.apply_scan_message_for_test(root_started(1));
        assert!(app.tree.root.is_none());
    }

    #[test]
    fn cancel_like_new_scan_keeps_old_events_out() {
        let mut app = DiskLensApp {
            active_scan_id: 2,
            ..Default::default()
        };
        app.apply_scan_message_for_test(root_started(2));
        app.active_scan_id = 3;
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 2,
            batch: ScanBatch {
                discovered_nodes: vec![DiscoveredNode {
                    node_id: 1,
                    parent_id: 0,
                    node: NodeRecord {
                        name: "old".into(),
                        path: "/root/old".into(),
                        kind: NodeKind::File,
                        size: 1,
                        scanned: true,
                        error: None,
                    },
                }],
                size_deltas: vec![],
                scanned_nodes: vec![],
                progress: None,
            },
        });

        assert_eq!(app.tree.len(), 1);
    }

    #[test]
    fn search_rebuild_marks_matches_in_current_root() {
        let mut app = DiskLensApp {
            active_scan_id: 1,
            ..Default::default()
        };
        app.apply_scan_message_for_test(root_started(1));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![DiscoveredNode {
                    node_id: 1,
                    parent_id: 0,
                    node: NodeRecord {
                        name: "match-me".into(),
                        path: "/root/match-me".into(),
                        kind: NodeKind::File,
                        size: 1,
                        scanned: true,
                        error: None,
                    },
                }],
                size_deltas: vec![],
                scanned_nodes: vec![],
                progress: None,
            },
        });
        app.search_input = "match".into();
        app.refresh_search_matches();

        assert_eq!(app.search_state.match_count(), 1);
        assert!(app.search_state.is_match(1));
    }

    #[test]
    fn truncate_middle_should_keep_prefix_and_suffix() {
        let truncated = truncate_middle("abcdefghijklmnopqrstuvwxyz", 10);
        assert_eq!(truncated, "abcd…vwxyz");
    }
}
