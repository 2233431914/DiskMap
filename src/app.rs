use crate::cleanup::{
    protected_path_reason, CleanupCandidate, CleanupQueue, ProtectedPathReason, QueueAddResult,
};
use crate::duplicates::{find_duplicate_candidates, DuplicateCandidate, DuplicateReport};
use crate::export::{export_focused_report, export_subtree, ExportFormat, FocusedReportMetadata};
use crate::format::format_bytes;
use crate::insights::{
    analyze_insights, AgeBucketSummary, FileTypeSummary, InsightReport, OldLargeFile,
    INSIGHT_REPORT_LIMIT,
};
use crate::platform::{move_to_trash, open_path, reveal_in_finder};
use crate::scanner::{
    parse_exclude_patterns, scan_path_to_tree, size_basis_detail, size_basis_label, CacheMode,
    PerfStats, ProgressSnapshot, ScanBatch, ScanMessage, ScanOptions,
};
use crate::snapshot::{
    capture_snapshot, compare_snapshots, ScanSnapshot, SnapshotChange, SnapshotDiff,
};
use crate::tree::{NodeId, NodeKind, TreeStore};
use crate::treemap::{
    layout_treemap, Camera, LayoutScratch, TreemapLayoutParams, VisualKind, VisualNode,
};
use crate::watcher::{WatchPoll, WatchSession};

mod navigation;
mod scan_session;
mod search_nav;

use navigation::{NavigationOutcome, NavigationState};
use scan_session::ScanSession;
use search_nav::{SearchController, SearchDirection, SEARCH_REFRESH_INTERVAL};

use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui;
use egui::{
    Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Shadow, Stroke, Theme, Vec2,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LAYOUT_REFRESH_INTERVAL: Duration = Duration::from_millis(33);
const CONTEXT_MENU_MIN_WIDTH: f32 = 240.0;
const CONTEXT_MENU_MAX_TITLE_CHARS: usize = 36;
const STORAGE_PATH_INPUT: &str = "disk_map.path_input";
const STORAGE_EXCLUDE_INPUT: &str = "disk_map.exclude_input";
const STORAGE_INCLUDE_HIDDEN: &str = "disk_map.include_hidden";
const STORAGE_FOLLOW_SYMLINKS: &str = "disk_map.follow_symlinks";
const STORAGE_STAY_ON_FILESYSTEM: &str = "disk_map.stay_on_filesystem";
const STORAGE_REALTIME_WATCH: &str = "disk_map.realtime_watch";
const STORAGE_SQLITE_CACHE: &str = "disk_map.sqlite_cache";
const STORAGE_SEARCH_FILTER: &str = "disk_map.search_filter";
const STORAGE_COLOR_BY_EXTENSION: &str = "disk_map.color_by_extension";
const STORAGE_RECENT_ROOTS: &str = "disk_map.recent_roots";
const STORAGE_PINNED_ROOTS: &str = "disk_map.pinned_roots";
const STORAGE_MAX_DEPTH: &str = "disk_map.max_depth";
const STORAGE_THEME: &str = "disk_map.theme";
const MAX_RECENT_ROOTS: usize = 10;
const MAX_PINNED_ROOTS: usize = 12;
const SNAPSHOT_DIFF_LIMIT: usize = 5;
const DUPLICATE_REPORT_LIMIT: usize = 8;

#[derive(Clone, Copy)]
struct Palette {
    surface: Color32,
    panel: Color32,
    panel_elevated: Color32,
    text: Color32,
    text_muted: Color32,
    text_faint: Color32,
    accent: Color32,
    accent_soft: Color32,
    danger: Color32,
    stroke_subtle: Color32,
    stroke_strong: Color32,
    dir_palette: [Color32; 5],
    file_neutral: Color32,
    shadow_color: Color32,
}

struct StateMessage {
    title: &'static str,
    detail: String,
}

#[derive(Debug)]
struct IncrementalScanResult {
    target_id: NodeId,
    path: PathBuf,
    result: anyhow::Result<TreeStore>,
}

const DARK_PALETTE: Palette = Palette {
    surface: Color32::from_rgb(0x0F, 0x0F, 0x12),
    panel: Color32::from_rgb(0x18, 0x18, 0x1B),
    panel_elevated: Color32::from_rgb(0x23, 0x23, 0x28),
    text: Color32::from_rgb(0xEC, 0xEC, 0xF1),
    text_muted: Color32::from_rgb(0xA1, 0xA1, 0xAA),
    text_faint: Color32::from_rgb(0x71, 0x71, 0x7A),
    accent: Color32::from_rgb(0x81, 0x8C, 0xF8),
    accent_soft: Color32::from_rgba_premultiplied(0x33, 0x38, 0x60, 60),
    danger: Color32::from_rgb(0xF8, 0x71, 0x71),
    stroke_subtle: Color32::from_rgba_premultiplied(14, 14, 14, 14),
    stroke_strong: Color32::from_rgba_premultiplied(32, 32, 32, 32),
    dir_palette: [
        Color32::from_rgb(0x3B, 0x3F, 0x5C),
        Color32::from_rgb(0x3D, 0x5A, 0x4F),
        Color32::from_rgb(0x5A, 0x4A, 0x3A),
        Color32::from_rgb(0x4A, 0x3A, 0x5A),
        Color32::from_rgb(0x3A, 0x4D, 0x5A),
    ],
    file_neutral: Color32::from_rgb(0x48, 0x48, 0x4A),
    shadow_color: Color32::from_black_alpha(160),
};

const LIGHT_PALETTE: Palette = Palette {
    surface: Color32::from_rgb(0xF8, 0xF8, 0xFB),
    panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    panel_elevated: Color32::from_rgb(0xF1, 0xF1, 0xF4),
    text: Color32::from_rgb(0x18, 0x18, 0x1B),
    text_muted: Color32::from_rgb(0x52, 0x52, 0x5B),
    text_faint: Color32::from_rgb(0xA1, 0xA1, 0xAA),
    accent: Color32::from_rgb(0x63, 0x66, 0xF1),
    accent_soft: Color32::from_rgba_premultiplied(0x14, 0x15, 0x33, 38),
    danger: Color32::from_rgb(0xDC, 0x26, 0x26),
    stroke_subtle: Color32::from_rgba_premultiplied(0, 0, 0, 12),
    stroke_strong: Color32::from_rgba_premultiplied(0, 0, 0, 26),
    dir_palette: [
        Color32::from_rgb(0xC8, 0xCC, 0xE0),
        Color32::from_rgb(0xCC, 0xE0, 0xD4),
        Color32::from_rgb(0xE0, 0xD0, 0xBC),
        Color32::from_rgb(0xD8, 0xC8, 0xE0),
        Color32::from_rgb(0xC8, 0xD4, 0xE0),
    ],
    file_neutral: Color32::from_rgb(0xD2, 0xD2, 0xD7),
    shadow_color: Color32::from_black_alpha(48),
};

fn palette_for(theme: Theme) -> &'static Palette {
    match theme {
        Theme::Dark => &DARK_PALETTE,
        Theme::Light => &LIGHT_PALETTE,
    }
}

fn palette(ctx: &egui::Context) -> &'static Palette {
    palette_for(ctx.theme())
}

fn pick_label_color(bg: Color32) -> Color32 {
    let luma = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if luma < 140.0 {
        Color32::from_rgb(245, 245, 250)
    } else {
        Color32::from_rgb(20, 20, 24)
    }
}

pub fn configure_theme(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Light, build_visuals(&LIGHT_PALETTE, false));
    ctx.set_visuals_of(Theme::Dark, build_visuals(&DARK_PALETTE, true));

    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.menu_margin = Margin::same(10);
    style.spacing.window_margin = Margin::same(14);
    style.spacing.indent = 16.0;
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(18.0));
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::proportional(11.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(12.0));
    ctx.set_global_style(style);
}

fn build_visuals(p: &Palette, dark: bool) -> egui::Visuals {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = p.panel;
    visuals.window_fill = p.panel;
    visuals.extreme_bg_color = p.surface;
    visuals.faint_bg_color = p.panel_elevated;
    visuals.override_text_color = Some(p.text);

    let radius = CornerRadius::same(6);
    let widgets = &mut visuals.widgets;

    widgets.noninteractive.bg_fill = p.panel;
    widgets.noninteractive.weak_bg_fill = p.panel;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke_subtle);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    widgets.noninteractive.corner_radius = radius;

    widgets.inactive.bg_fill = p.panel_elevated;
    widgets.inactive.weak_bg_fill = p.panel_elevated;
    widgets.inactive.bg_stroke = Stroke::new(1.0, p.stroke_subtle);
    widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    widgets.inactive.corner_radius = radius;

    widgets.hovered.bg_fill = p.panel_elevated;
    widgets.hovered.weak_bg_fill = p.panel_elevated;
    widgets.hovered.bg_stroke = Stroke::new(1.0, p.stroke_strong);
    widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    widgets.hovered.corner_radius = radius;

    widgets.active.bg_fill = p.accent_soft;
    widgets.active.weak_bg_fill = p.accent_soft;
    widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    widgets.active.corner_radius = radius;

    widgets.open.bg_fill = p.panel_elevated;
    widgets.open.weak_bg_fill = p.panel_elevated;
    widgets.open.bg_stroke = Stroke::new(1.0, p.stroke_strong);
    widgets.open.fg_stroke = Stroke::new(1.0, p.text);
    widgets.open.corner_radius = radius;

    visuals.selection.bg_fill = p.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0, p.accent);
    visuals.hyperlink_color = p.accent;

    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(10);

    visuals.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 24,
        spread: 0,
        color: p.shadow_color,
    };
    visuals.window_shadow = Shadow {
        offset: [0, 2],
        blur: 12,
        spread: 0,
        color: p.shadow_color,
    };

    visuals
}

pub struct DiskMapApp {
    path_input: String,
    exclude_input: String,
    include_hidden: bool,
    follow_symlinks: bool,
    stay_on_filesystem: bool,
    realtime_watch_enabled: bool,
    sqlite_cache_enabled: bool,
    watcher: Option<WatchSession>,
    initial_scan_pending: bool,
    tx: Sender<ScanMessage>,
    rx: Receiver<ScanMessage>,
    incremental_tx: Sender<IncrementalScanResult>,
    incremental_rx: Receiver<IncrementalScanResult>,
    incremental_scan_active: bool,
    tree: TreeStore,
    navigation: NavigationState,
    search: SearchController,
    search_filter_enabled: bool,
    color_by_extension: bool,
    recent_roots: Vec<String>,
    pinned_roots: Vec<String>,
    last_snapshot: Option<ScanSnapshot>,
    snapshot_diff: Option<SnapshotDiff>,
    duplicate_report: Option<DuplicateReport>,
    insight_report: Option<InsightReport>,
    scan: ScanSession,
    hovered_id: Option<NodeId>,
    context_menu_target_id: Option<NodeId>,
    trash_confirm_target_id: Option<NodeId>,
    cleanup_queue: CleanupQueue,
    destructive_actions_enabled: bool,
    hovered_visual_kind: Option<VisualKind>,
    camera: Camera,
    max_depth: usize,
    theme_preference: Option<Theme>,
    status: String,
    cached_visuals: Vec<VisualNode>,
    layout_scratch: LayoutScratch,
    last_canvas_rect: Option<Rect>,
    layout_dirty: bool,
    last_layout_refresh: Instant,
    pending_repaint: bool,
}

impl Default for DiskMapApp {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        let (incremental_tx, incremental_rx) = unbounded();
        Self {
            path_input: dirs_home_fallback(),
            exclude_input: String::new(),
            include_hidden: ScanOptions::default().include_hidden,
            follow_symlinks: ScanOptions::default().follow_symlinks,
            stay_on_filesystem: ScanOptions::default().stay_on_filesystem,
            realtime_watch_enabled: false,
            sqlite_cache_enabled: false,
            watcher: None,
            initial_scan_pending: true,
            tx,
            rx,
            incremental_tx,
            incremental_rx,
            incremental_scan_active: false,
            tree: TreeStore::new(),
            navigation: NavigationState::default(),
            search: SearchController::default(),
            search_filter_enabled: false,
            color_by_extension: false,
            recent_roots: Vec::new(),
            pinned_roots: Vec::new(),
            last_snapshot: None,
            snapshot_diff: None,
            duplicate_report: None,
            insight_report: None,
            scan: ScanSession::default(),
            hovered_id: None,
            context_menu_target_id: None,
            trash_confirm_target_id: None,
            cleanup_queue: CleanupQueue::default(),
            destructive_actions_enabled: false,
            hovered_visual_kind: None,
            camera: Camera::default(),
            max_depth: 1,
            theme_preference: None,
            status: "Ready".to_string(),
            cached_visuals: Vec::new(),
            layout_scratch: LayoutScratch::default(),
            last_canvas_rect: None,
            layout_dirty: true,
            last_layout_refresh: Instant::now(),
            pending_repaint: false,
        }
    }
}

impl DiskMapApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::default();
        if let Some(storage) = cc.storage {
            app.restore_preferences(storage);
        }
        if let Some(theme) = app.theme_preference {
            apply_theme_preference(&cc.egui_ctx, theme);
        } else {
            app.theme_preference = Some(cc.egui_ctx.theme());
        }
        app
    }

    fn restore_preferences(&mut self, storage: &dyn eframe::Storage) {
        if let Some(path_input) = storage.get_string(STORAGE_PATH_INPUT) {
            if !path_input.trim().is_empty() {
                self.path_input = path_input;
            }
        }

        if let Some(exclude_input) = storage.get_string(STORAGE_EXCLUDE_INPUT) {
            self.exclude_input = exclude_input;
        }

        if let Some(include_hidden) = storage
            .get_string(STORAGE_INCLUDE_HIDDEN)
            .and_then(|value| parse_storage_bool(&value))
        {
            self.include_hidden = include_hidden;
        }

        if let Some(follow_symlinks) = storage
            .get_string(STORAGE_FOLLOW_SYMLINKS)
            .and_then(|value| parse_storage_bool(&value))
        {
            self.follow_symlinks = follow_symlinks;
        }

        if let Some(stay_on_filesystem) = storage
            .get_string(STORAGE_STAY_ON_FILESYSTEM)
            .and_then(|value| parse_storage_bool(&value))
        {
            self.stay_on_filesystem = stay_on_filesystem;
        }

        if let Some(realtime_watch_enabled) = storage
            .get_string(STORAGE_REALTIME_WATCH)
            .and_then(|value| parse_storage_bool(&value))
        {
            self.realtime_watch_enabled = realtime_watch_enabled;
        }

        if let Some(sqlite_cache_enabled) = storage
            .get_string(STORAGE_SQLITE_CACHE)
            .and_then(|value| parse_storage_bool(&value))
        {
            self.sqlite_cache_enabled = sqlite_cache_enabled;
        }

        if let Some(search_filter_enabled) = storage
            .get_string(STORAGE_SEARCH_FILTER)
            .and_then(|value| parse_storage_bool(&value))
        {
            self.search_filter_enabled = search_filter_enabled;
        }

        if let Some(color_by_extension) = storage
            .get_string(STORAGE_COLOR_BY_EXTENSION)
            .and_then(|value| parse_storage_bool(&value))
        {
            self.color_by_extension = color_by_extension;
        }

        if let Some(recent_roots) = storage.get_string(STORAGE_RECENT_ROOTS) {
            self.recent_roots = parse_stored_paths(&recent_roots, MAX_RECENT_ROOTS);
        }

        if let Some(pinned_roots) = storage.get_string(STORAGE_PINNED_ROOTS) {
            self.pinned_roots = parse_stored_paths(&pinned_roots, MAX_PINNED_ROOTS);
        }

        if let Some(depth) = storage
            .get_string(STORAGE_MAX_DEPTH)
            .and_then(|value| value.parse::<usize>().ok())
        {
            self.max_depth = depth.clamp(1, 10);
        }

        self.theme_preference = storage
            .get_string(STORAGE_THEME)
            .and_then(|value| parse_theme_preference(&value));
    }

    fn save_preferences(&self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_PATH_INPUT, self.path_input.clone());
        storage.set_string(STORAGE_EXCLUDE_INPUT, self.exclude_input.clone());
        storage.set_string(STORAGE_INCLUDE_HIDDEN, self.include_hidden.to_string());
        storage.set_string(STORAGE_FOLLOW_SYMLINKS, self.follow_symlinks.to_string());
        storage.set_string(
            STORAGE_STAY_ON_FILESYSTEM,
            self.stay_on_filesystem.to_string(),
        );
        storage.set_string(
            STORAGE_REALTIME_WATCH,
            self.realtime_watch_enabled.to_string(),
        );
        storage.set_string(STORAGE_SQLITE_CACHE, self.sqlite_cache_enabled.to_string());
        storage.set_string(
            STORAGE_SEARCH_FILTER,
            self.search_filter_enabled.to_string(),
        );
        storage.set_string(
            STORAGE_COLOR_BY_EXTENSION,
            self.color_by_extension.to_string(),
        );
        storage.set_string(STORAGE_RECENT_ROOTS, serialize_paths(&self.recent_roots));
        storage.set_string(STORAGE_PINNED_ROOTS, serialize_paths(&self.pinned_roots));
        storage.set_string(STORAGE_MAX_DEPTH, self.max_depth.to_string());
        if let Some(theme) = self.theme_preference {
            storage.set_string(STORAGE_THEME, theme_preference_name(theme).to_string());
        }
    }

    fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            exclude_patterns: parse_exclude_patterns(&self.exclude_input),
            cache_mode: if self.sqlite_cache_enabled {
                CacheMode::Enabled
            } else {
                CacheMode::Disabled
            },
            include_hidden: self.include_hidden,
            follow_symlinks: self.follow_symlinks,
            stay_on_filesystem: self.stay_on_filesystem,
            ..ScanOptions::default()
        }
    }
}

impl eframe::App for DiskMapApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.initial_scan_pending {
            self.initial_scan_pending = false;
            self.start_scan();
        }
        self.handle_keyboard(ctx);
        self.handle_scan_messages();
        self.handle_incremental_scan_results();
        self.handle_watch_events();
        self.maybe_refresh_search(ctx);
        self.maybe_request_deferred_repaint(ctx);
        self.drive_background_updates(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            self.show_toolbar(ui);
        });

        egui::Panel::right("details_panel")
            .resizable(true)
            .default_size(280.0)
            .min_size(260.0)
            .max_size(340.0)
            .show_inside(ui, |ui| self.show_details_panel(ui));

        egui::Panel::bottom("status_bar")
            .exact_size(28.0)
            .show_inside(ui, |ui| self.show_status_bar(ui));

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.show_treemap(ui);
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.save_preferences(storage);
    }
}

impl DiskMapApp {
    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if icon_button(ui, self.navigation.can_go_back(), ToolbarIcon::ArrowLeft)
                .on_hover_text("Back")
                .clicked()
            {
                self.navigate_back();
            }

            if icon_button(
                ui,
                self.navigation.can_go_forward(),
                ToolbarIcon::ArrowRight,
            )
            .on_hover_text("Forward")
            .clicked()
            {
                self.navigate_forward();
            }

            if icon_button(ui, self.navigation.can_go_up(&self.tree), ToolbarIcon::Up)
                .on_hover_text("Up to parent directory")
                .clicked()
            {
                if let Some(parent) = self.navigation.parent_of_focused_root(&self.tree) {
                    self.enter_root(parent, true);
                }
            }

            if icon_button(
                ui,
                self.navigation.can_return_to_scan_root(&self.tree),
                ToolbarIcon::Home,
            )
            .on_hover_text("Return to scan root")
            .clicked()
            {
                self.return_to_scan_root();
            }

            if icon_button(ui, true, ToolbarIcon::Refresh)
                .on_hover_text("Reset view")
                .clicked()
            {
                self.reset_camera();
            }

            ui.add_space(4.0);

            let path_width = ui.available_width().clamp(220.0, 420.0) - 280.0;
            let path_width = path_width.max(200.0);
            let path_edit = ui.add_sized(
                [path_width, 28.0],
                egui::TextEdit::singleline(&mut self.path_input).hint_text("/path/to/scan"),
            );
            if path_edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.start_scan();
            }

            self.show_roots_menu(ui);

            let scan_label = if self.scan.is_scanning() {
                "Cancel"
            } else {
                "Scan"
            };
            if ui
                .add_sized([72.0, 28.0], egui::Button::new(scan_label))
                .clicked()
            {
                if self.scan.is_scanning() {
                    self.cancel_scan();
                } else {
                    self.start_scan();
                }
            }

            ui.add_space(4.0);
            ui.label(
                RichText::new("RESCAN")
                    .size(10.0)
                    .color(palette(ui.ctx()).text_faint)
                    .strong(),
            );
            if ui
                .add_enabled(
                    self.can_rescan_scan_root(),
                    egui::Button::new("Root").min_size(Vec2::new(48.0, 28.0)),
                )
                .on_hover_text("Rescan the original scan root")
                .clicked()
            {
                self.rescan_scan_root();
            }
            if ui
                .add_enabled(
                    self.can_rescan_focused_subtree(),
                    egui::Button::new("View").min_size(Vec2::new(48.0, 28.0)),
                )
                .on_hover_text("Rescan the currently focused directory")
                .clicked()
            {
                self.rescan_focused_subtree();
            }

            ui.add_space(6.0);

            ui.label(
                RichText::new("EXCLUDE")
                    .size(10.0)
                    .color(palette(ui.ctx()).text_faint)
                    .strong(),
            );
            ui.add_sized(
                [150.0, 28.0],
                egui::TextEdit::singleline(&mut self.exclude_input)
                    .hint_text(".git,node_modules,target"),
            );

            ui.add_space(4.0);
            ui.checkbox(&mut self.include_hidden, "Hidden")
                .on_hover_text("Include hidden files and folders");
            ui.checkbox(&mut self.follow_symlinks, "Links")
                .on_hover_text("Follow symlinked directories during scan");
            ui.checkbox(&mut self.stay_on_filesystem, "Same FS")
                .on_hover_text("Stay on the scan root filesystem when supported");

            let before_watch = self.realtime_watch_enabled;
            ui.checkbox(&mut self.realtime_watch_enabled, "Watch")
                .on_hover_text("Watch the scan root and rescan after debounced filesystem changes");
            if self.realtime_watch_enabled != before_watch {
                self.update_watch_state();
            }
            ui.checkbox(&mut self.sqlite_cache_enabled, "SQLite")
                .on_hover_text("Experimental scan cache for faster rescans");

            ui.add_space(6.0);

            let search_response = ui.add_sized(
                [180.0, 28.0],
                egui::TextEdit::singleline(self.search.input_mut())
                    .hint_text("Search files & folders"),
            );
            if search_response.changed() {
                self.mark_search_dirty();
            }
            if search_response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
            {
                if ui.input(|input| input.modifiers.shift) {
                    self.navigate_search_match(SearchDirection::Previous);
                } else {
                    self.navigate_search_match(SearchDirection::Next);
                }
            }
            ui.add_space(2.0);
            if icon_button(
                ui,
                self.can_navigate_search_matches(),
                ToolbarIcon::ArrowLeft,
            )
            .on_hover_text("Previous search match")
            .clicked()
            {
                self.navigate_search_match(SearchDirection::Previous);
            }
            if icon_button(
                ui,
                self.can_navigate_search_matches(),
                ToolbarIcon::ArrowRight,
            )
            .on_hover_text("Next search match")
            .clicked()
            {
                self.navigate_search_match(SearchDirection::Next);
            }
            if !self.search.input().is_empty()
                && icon_button(ui, true, ToolbarIcon::Close)
                    .on_hover_text("Clear search")
                    .clicked()
            {
                self.clear_search();
            }
            if ui
                .checkbox(&mut self.search_filter_enabled, "Filter")
                .on_hover_text("Show only search matches and their ancestor folders")
                .changed()
            {
                self.mark_layout_dirty_now();
            }

            ui.add_space(6.0);
            ui.label(
                RichText::new("DEPTH")
                    .size(10.0)
                    .color(palette(ui.ctx()).text_faint)
                    .strong(),
            );
            if ui
                .add_sized(
                    [96.0, 18.0],
                    egui::Slider::new(&mut self.max_depth, 1..=10).text(""),
                )
                .changed()
            {
                self.mark_layout_dirty_now();
            }
            if ui
                .checkbox(&mut self.color_by_extension, "Ext")
                .on_hover_text("Color files by extension")
                .changed()
            {
                self.mark_layout_dirty_now();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(theme) = theme_cycle_button(ui) {
                    self.theme_preference = Some(theme);
                }
            });
        });
    }

    fn show_roots_menu(&mut self, ui: &mut egui::Ui) {
        let pin_candidate = self.current_root_candidate();
        let is_pinned = pin_candidate
            .as_deref()
            .is_some_and(|path| self.is_root_pinned(path));

        ui.menu_button("Roots", |ui| {
            ui.set_min_width(280.0);
            let can_pin = pin_candidate.is_some();
            let pin_label = if is_pinned {
                "Unpin Current"
            } else {
                "Pin Current"
            };
            if ui
                .add_enabled(can_pin, egui::Button::new(pin_label))
                .clicked()
            {
                if let Some(path) = pin_candidate.as_deref() {
                    self.toggle_pinned_root(path);
                }
                ui.close();
            }

            ui.separator();
            self.show_root_menu_group(ui, "Pinned", self.pinned_roots.clone());
            self.show_root_menu_group(ui, "Recent", self.recent_roots.clone());
        })
        .response
        .on_hover_text("Open recent and pinned scan roots");
    }

    fn show_root_menu_group(&mut self, ui: &mut egui::Ui, label: &str, roots: Vec<String>) {
        ui.label(
            RichText::new(label)
                .size(10.0)
                .strong()
                .color(palette(ui.ctx()).text_faint),
        );
        if roots.is_empty() {
            ui.label(
                RichText::new("None")
                    .small()
                    .color(palette(ui.ctx()).text_faint),
            );
            return;
        }

        for path in roots {
            if ui
                .button(truncate_middle(&path, 54))
                .on_hover_text(&path)
                .clicked()
            {
                self.start_scan_path(PathBuf::from(path));
                ui.close();
            }
        }
    }

    fn show_details_panel(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui.ctx());
        ui.add_space(4.0);
        ui.label(
            RichText::new("DETAILS")
                .size(11.0)
                .strong()
                .color(p.text_muted),
        );
        ui.add_space(2.0);
        section_divider(ui, p);
        ui.add_space(8.0);

        let subject_id = self
            .navigation
            .selected_id()
            .or(self.navigation.focused_root());
        let Some(node_id) = subject_id else {
            self.show_state_message(ui, p, &self.no_root_state_message());
            self.show_progress_section(ui, p);
            self.show_scan_issue_section(ui, p);
            self.show_search_section(ui, p);
            return;
        };

        let node_path = self.tree.node_real_path(node_id);
        let (node_name, node_size, node_kind, child_count, node_scanned, node_error, node_parent) = {
            let node = self.tree.node(node_id);
            (
                node.name.clone(),
                node.size,
                node.kind,
                node.children.len(),
                node.scanned,
                node.error.clone(),
                node.parent,
            )
        };
        let parent_size = node_parent.map(|pid| self.tree.node(pid).size);
        let parent_fraction = parent_size.and_then(|s| {
            if s > 0 {
                Some((node_size as f64 / s as f64).min(1.0) as f32)
            } else {
                None
            }
        });
        let matched = self.search.state().is_match(node_id);
        let kind_label = describe_node_kind(node_kind, child_count > 0);

        egui::Frame::new()
            .fill(p.panel_elevated)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(12))
            .stroke(Stroke::new(1.0, p.stroke_subtle))
            .show(ui, |ui| {
                ui.label(RichText::new(&node_name).strong().size(14.0).color(p.text));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format_bytes(node_size))
                            .monospace()
                            .color(p.accent),
                    );
                    if let Some(frac) = parent_fraction {
                        ui.add_space(6.0);
                        mini_bar(ui, frac, p);
                        ui.label(
                            RichText::new(format!("{:.0}% of parent", frac * 100.0))
                                .small()
                                .color(p.text_muted),
                        );
                    }
                });
                ui.label(
                    RichText::new(size_basis_label())
                        .small()
                        .color(p.text_faint),
                )
                .on_hover_text(size_basis_detail());
                ui.add_space(4.0);
                let meta = if child_count > 0 {
                    format!("{kind_label} · {} items", child_count)
                } else {
                    kind_label.to_string()
                };
                ui.label(RichText::new(meta).small().color(p.text_muted));
                if !node_scanned {
                    ui.label(
                        RichText::new("Scanning in progress…")
                            .small()
                            .color(p.accent),
                    );
                }
                if !self.search.query().is_empty() {
                    let (txt, color) = if matched {
                        ("Matches search", p.accent)
                    } else {
                        ("No search match", p.text_faint)
                    };
                    ui.label(RichText::new(txt).small().color(color));
                }
                if let Some(path) = &node_path {
                    ui.add_space(4.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(path.display().to_string())
                                .monospace()
                                .small()
                                .color(p.text_faint),
                        )
                        .wrap(),
                    );
                }
            });

        if let Some(err) = &node_error {
            ui.add_space(6.0);
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(
                    p.danger.r(),
                    p.danger.g(),
                    p.danger.b(),
                    28,
                ))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.label(RichText::new(format!("Error: {err}")).color(p.danger));
                });
        }

        ui.add_space(12.0);
        ui.label(
            RichText::new("PRIMARY")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        let path_available = node_path.is_some();
        ui.columns(2, |cols| {
            let w0 = cols[0].available_width();
            if accent_button(&mut cols[0], "Open", path_available, w0, p).clicked() {
                if let Some(path) = &node_path {
                    self.apply_platform_result("Open", open_path(path));
                }
            }
            let w1 = cols[1].available_width();
            if cols[1]
                .add_enabled(
                    path_available,
                    egui::Button::new("Reveal").min_size(Vec2::new(w1, 32.0)),
                )
                .clicked()
            {
                if let Some(path) = &node_path {
                    self.apply_platform_result("Reveal", reveal_in_finder(path));
                }
            }
        });

        ui.add_space(10.0);
        ui.label(
            RichText::new("UTILITY")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        let copy_width = ui.available_width();
        if ui
            .add_enabled(
                path_available,
                egui::Button::new("Copy Path").min_size(Vec2::new(copy_width, 28.0)),
            )
            .clicked()
        {
            if let Some(path) = &node_path {
                ui.ctx().copy_text(path.display().to_string());
            }
        }
        ui.add_space(4.0);
        if ui
            .checkbox(&mut self.destructive_actions_enabled, "Allow Trash")
            .on_hover_text("Enable cleanup review queue and two-step Move to Trash")
            .changed()
            && !self.destructive_actions_enabled
        {
            self.trash_confirm_target_id = None;
        }
        if self.destructive_actions_enabled {
            let queue_enabled = path_available;
            let label = if self.cleanup_queue.contains_node(node_id) {
                "Queued for Trash"
            } else {
                "Queue for Trash"
            };
            let queue_width = ui.available_width();
            if ui
                .add_enabled(
                    queue_enabled,
                    egui::Button::new(label).min_size(Vec2::new(queue_width, 28.0)),
                )
                .clicked()
            {
                self.queue_cleanup_candidate(node_id);
            }
        }
        ui.add_space(4.0);
        let focused_export_id = self.navigation.focused_root();
        let scan_root_export_id = self.tree.root;
        let scan_root_is_focused =
            focused_export_id.is_some() && focused_export_id == scan_root_export_id;
        ui.columns(2, |cols| {
            let can_export = focused_export_id.is_some();
            let w0 = cols[0].available_width();
            if cols[0]
                .add_enabled(
                    can_export,
                    egui::Button::new("Export View CSV").min_size(Vec2::new(w0, 28.0)),
                )
                .clicked()
            {
                self.export_focused_subtree(ExportFormat::Csv);
            }
            let w1 = cols[1].available_width();
            if cols[1]
                .add_enabled(
                    can_export,
                    egui::Button::new("Export View JSON").min_size(Vec2::new(w1, 28.0)),
                )
                .clicked()
            {
                self.export_focused_subtree(ExportFormat::Json);
            }
        });
        if !scan_root_is_focused {
            ui.add_space(4.0);
            ui.columns(2, |cols| {
                let can_export = scan_root_export_id.is_some();
                let w0 = cols[0].available_width();
                if cols[0]
                    .add_enabled(
                        can_export,
                        egui::Button::new("Export Root CSV").min_size(Vec2::new(w0, 28.0)),
                    )
                    .clicked()
                {
                    self.export_scan_root(ExportFormat::Csv);
                }
                let w1 = cols[1].available_width();
                if cols[1]
                    .add_enabled(
                        can_export,
                        egui::Button::new("Export Root JSON").min_size(Vec2::new(w1, 28.0)),
                    )
                    .clicked()
                {
                    self.export_scan_root(ExportFormat::Json);
                }
            });
        }
        ui.add_space(4.0);
        let report_width = ui.available_width();
        if ui
            .add_enabled(
                focused_export_id.is_some(),
                egui::Button::new("Export Report JSON").min_size(Vec2::new(report_width, 28.0)),
            )
            .on_hover_text("Export current view data plus metadata needed to reproduce this view")
            .clicked()
        {
            self.export_focused_report_json();
        }
        ui.add_space(4.0);
        let duplicate_width = ui.available_width();
        if ui
            .add_enabled(
                focused_export_id.is_some() && !self.scan.is_scanning(),
                egui::Button::new("Analyze Duplicates").min_size(Vec2::new(duplicate_width, 28.0)),
            )
            .on_hover_text("Read-only heuristic: same file name and same size in the current view")
            .clicked()
        {
            self.analyze_duplicate_candidates();
        }
        ui.add_space(4.0);
        let insight_width = ui.available_width();
        if ui
            .add_enabled(
                focused_export_id.is_some() && !self.scan.is_scanning(),
                egui::Button::new("Analyze Insights").min_size(Vec2::new(insight_width, 28.0)),
            )
            .on_hover_text("Read-only age buckets and extension category summary for this view")
            .clicked()
        {
            self.analyze_file_insights();
        }

        if let Some(parent) = node_parent {
            ui.add_space(10.0);
            ui.label(
                RichText::new("PARENT")
                    .size(10.0)
                    .strong()
                    .color(p.text_faint),
            );
            ui.add_space(4.0);
            let parent_name = self.tree.node(parent).name.clone();
            if ui
                .add(
                    egui::Button::new(RichText::new(format!("↑ {parent_name}")).color(p.text))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(1.0, p.stroke_subtle)),
                )
                .clicked()
            {
                self.navigation.set_selected_id(Some(parent));
            }
        }

        self.show_progress_section(ui, p);
        self.show_scan_issue_section(ui, p);
        self.show_cleanup_queue_section(ui, p);
        self.show_snapshot_diff_section(ui, p);
        self.show_duplicate_report_section(ui, p);
        self.show_insight_report_section(ui, p);
        self.show_search_section(ui, p);
    }

    fn show_progress_section(&self, ui: &mut egui::Ui, p: &Palette) {
        let Some(progress) = self.scan.progress() else {
            return;
        };
        ui.add_space(12.0);
        ui.label(
            RichText::new("SCAN")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{} files · {} dirs",
                progress.files_scanned, progress.dirs_scanned
            ))
            .small()
            .color(p.text_muted),
        );
        ui.label(
            RichText::new(format_bytes(progress.bytes_seen))
                .monospace()
                .color(p.text),
        );
        ui.label(
            RichText::new(size_basis_label())
                .small()
                .color(p.text_faint),
        )
        .on_hover_text(size_basis_detail());
        let current_path = truncate_middle(&progress.current_path.display().to_string(), 42);
        ui.add(
            egui::Label::new(
                RichText::new(current_path)
                    .small()
                    .monospace()
                    .color(p.text_faint),
            )
            .truncate(),
        );
    }

    fn show_state_message(&self, ui: &mut egui::Ui, p: &Palette, message: &StateMessage) {
        ui.label(
            RichText::new(message.title)
                .strong()
                .size(14.0)
                .color(p.text),
        );
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(&message.detail).color(p.text_muted).small()).wrap());
    }

    fn show_scan_issue_section(&self, ui: &mut egui::Ui, p: &Palette) {
        let summary = self.scan.issue_summary();
        if !summary.has_findings() {
            return;
        }

        ui.add_space(12.0);
        ui.label(
            RichText::new("SCAN ISSUES")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        for (label, count, color) in [
            ("Error entries", summary.error_entries, p.danger),
            ("Skipped paths", summary.skipped_paths, p.text_muted),
            ("Permission errors", summary.permission_errors, p.danger),
            ("Symlinks", summary.symlinks, p.text_muted),
        ] {
            if count == 0 {
                continue;
            }
            ui.label(
                RichText::new(format!("{label}: {count}"))
                    .small()
                    .color(color),
            );
        }
    }

    fn show_cleanup_queue_section(&mut self, ui: &mut egui::Ui, p: &Palette) {
        if self.cleanup_queue.is_empty() {
            return;
        }

        ui.add_space(12.0);
        ui.label(
            RichText::new("CLEANUP QUEUE")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{} · {}",
                pluralize(self.cleanup_queue.len() as u64, "candidate", "candidates"),
                format_bytes(self.cleanup_queue.total_size())
            ))
            .small()
            .color(p.text_muted),
        );

        let candidates = self.cleanup_queue.candidates().to_vec();
        for candidate in candidates {
            cleanup_candidate_row(ui, p, &candidate);
            ui.columns(2, |cols| {
                let w0 = cols[0].available_width();
                if cols[0]
                    .add_enabled(
                        self.destructive_actions_enabled,
                        egui::Button::new(
                            if self.trash_confirm_target_id == Some(candidate.node_id) {
                                "Confirm Trash"
                            } else {
                                "Trash"
                            },
                        )
                        .min_size(Vec2::new(w0, 24.0)),
                    )
                    .clicked()
                {
                    self.arm_or_confirm_queued_trash(candidate.node_id);
                }
                let w1 = cols[1].available_width();
                if cols[1]
                    .add(egui::Button::new("Remove").min_size(Vec2::new(w1, 24.0)))
                    .clicked()
                {
                    self.remove_cleanup_candidate(candidate.node_id);
                }
            });
            ui.add_space(4.0);
        }
    }

    fn show_search_section(&self, ui: &mut egui::Ui, p: &Palette) {
        let query = self.search.query();
        if query.is_empty() {
            return;
        }
        ui.add_space(12.0);
        ui.label(
            RichText::new("SEARCH")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!("Query: {query}"))
                .small()
                .color(p.text_muted),
        );
        let match_text = if self.search.is_dirty() {
            format!("{} matches · Updating…", self.search.state().match_count())
        } else if let Some(index) = self.search.active_match() {
            format!(
                "{} / {} matches · Ready",
                index + 1,
                self.search.state().match_count()
            )
        } else {
            format!("{} matches · Ready", self.search.state().match_count())
        };
        ui.label(
            RichText::new(match_text)
                .small()
                .color(if self.search.is_dirty() {
                    p.accent
                } else {
                    p.text_muted
                }),
        );
    }

    fn show_snapshot_diff_section(&self, ui: &mut egui::Ui, p: &Palette) {
        let Some(diff) = &self.snapshot_diff else {
            return;
        };

        ui.add_space(12.0);
        ui.label(
            RichText::new("SNAPSHOT DIFF")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{} total change",
                format_signed_bytes(diff.total_delta())
            ))
            .small()
            .monospace()
            .color(if diff.total_delta() >= 0 {
                p.accent
            } else {
                p.text_muted
            }),
        )
        .on_hover_text(diff.root_path.display().to_string());

        if !diff.has_changes() {
            ui.label(
                RichText::new("No path-level changes since previous scan.")
                    .small()
                    .color(p.text_muted),
            );
            return;
        }

        snapshot_change_group(ui, p, "Added", &diff.added);
        snapshot_change_group(ui, p, "Grown", &diff.grown);
        snapshot_change_group(ui, p, "Shrunk", &diff.shrunk);
        snapshot_change_group(ui, p, "Removed", &diff.removed);
    }

    fn show_duplicate_report_section(&self, ui: &mut egui::Ui, p: &Palette) {
        let Some(report) = &self.duplicate_report else {
            return;
        };

        ui.add_space(12.0);
        ui.label(
            RichText::new("DUPLICATE CANDIDATES")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{} groups · {} files · up to {} candidates",
                report.group_count,
                report.file_count,
                format_bytes(report.total_reclaimable_bytes)
            ))
            .small()
            .color(p.text_muted),
        )
        .on_hover_text(report.root_path.display().to_string());

        if report.candidates.is_empty() {
            ui.label(
                RichText::new("No same-name same-size candidates in this view.")
                    .small()
                    .color(p.text_muted),
            );
            return;
        }

        for candidate in &report.candidates {
            duplicate_candidate_row(ui, p, candidate);
        }
    }

    fn show_insight_report_section(&self, ui: &mut egui::Ui, p: &Palette) {
        let Some(report) = &self.insight_report else {
            return;
        };

        ui.add_space(12.0);
        ui.label(
            RichText::new("INSIGHTS")
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{} files · {} known mtimes · {}",
                report.file_count,
                report.known_mtime_count,
                format_bytes(report.total_size)
            ))
            .small()
            .color(p.text_muted),
        )
        .on_hover_text(report.root_path.display().to_string());

        if report.file_count == 0 {
            ui.label(
                RichText::new("No files in this view.")
                    .small()
                    .color(p.text_muted),
            );
            return;
        }

        insight_type_group(ui, p, &report.type_summaries);
        insight_age_group(ui, p, &report.age_buckets);
        insight_old_files_group(ui, p, &report.old_large_files);
    }

    fn show_status_bar(&self, ui: &mut egui::Ui) {
        let p = palette(ui.ctx());
        let full_rect = ui.max_rect();
        ui.painter().line_segment(
            [full_rect.left_top(), full_rect.right_top()],
            Stroke::new(1.0, p.stroke_subtle),
        );

        ui.horizontal_centered(|ui| {
            ui.add_space(4.0);
            let dot_color = if self.status.starts_with("Error") {
                p.danger
            } else if self.scan.is_scanning() {
                p.accent
            } else if self.status.starts_with("Cancel") {
                p.text_faint
            } else {
                Color32::from_rgb(0x4A, 0xC4, 0x7A)
            };
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, dot_color);
            ui.label(RichText::new(&self.status).size(11.5).color(p.text_muted));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                if !self.navigation.breadcrumb().is_empty() {
                    let crumb = self.navigation.breadcrumb().replace(" / ", " › ");
                    let display = truncate_middle(&crumb, 60);
                    ui.label(
                        RichText::new(display)
                            .size(11.5)
                            .monospace()
                            .color(p.text_faint),
                    );
                }

                if let Some(progress) = self.scan.progress() {
                    ui.add_space(10.0);
                    ui.label(RichText::new("│").size(11.0).color(p.text_faint));
                    ui.add_space(10.0);
                    let text = format!(
                        "{} files · {} dirs · {}",
                        progress.files_scanned,
                        progress.dirs_scanned,
                        format_bytes(progress.bytes_seen)
                    );
                    ui.label(RichText::new(text).size(11.5).color(p.text_muted));
                    let current_path =
                        truncate_middle(&progress.current_path.display().to_string(), 44);
                    ui.add_space(10.0);
                    ui.label(RichText::new("│").size(11.0).color(p.text_faint));
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(current_path)
                            .size(11.5)
                            .monospace()
                            .color(p.text_faint),
                    );
                }
            });
        });
    }

    fn no_root_state_message(&self) -> StateMessage {
        if self.scan.is_scanning() {
            return StateMessage {
                title: "Starting scan",
                detail: format!("Waiting for scan results from {}.", self.path_input.trim()),
            };
        }

        if self.status.starts_with("Error") {
            return StateMessage {
                title: "Unable to scan path",
                detail: self
                    .status
                    .strip_prefix("Error: ")
                    .unwrap_or(&self.status)
                    .to_string(),
            };
        }

        if self.status.starts_with("Scan cancelled") {
            return StateMessage {
                title: "Scan cancelled",
                detail: "Start another scan to populate the treemap.".to_string(),
            };
        }

        StateMessage {
            title: "No scan loaded",
            detail: "Choose a path and start a scan to populate the treemap.".to_string(),
        }
    }

    fn empty_root_state_message(&self, root_id: NodeId) -> Option<StateMessage> {
        let root = self.tree.node(root_id);
        if self.scan.is_scanning() || !root.children.is_empty() {
            return None;
        }

        Some(StateMessage {
            title: "Empty folder",
            detail: format!("{} has no visible files or child folders.", root.name),
        })
    }

    fn paint_state_message(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        style: &egui::Style,
        palette: &Palette,
        message: &StateMessage,
    ) {
        painter.text(
            rect.center() - Vec2::new(0.0, 12.0),
            egui::Align2::CENTER_CENTER,
            message.title,
            egui::TextStyle::Heading.resolve(style),
            palette.text_faint,
        );
        painter.text(
            rect.center() + Vec2::new(0.0, 14.0),
            egui::Align2::CENTER_CENTER,
            truncate_middle(&message.detail, 72),
            egui::TextStyle::Small.resolve(style),
            palette.text_muted,
        );
    }

    fn show_treemap(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui.ctx());
        let available = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(available, Sense::click_and_drag());
        let painter = ui.painter_at(available);
        painter.rect_filled(available, 0.0, p.surface);

        let Some(root_id) = self.navigation.focused_root() else {
            self.paint_state_message(
                &painter,
                available,
                ui.style(),
                p,
                &self.no_root_state_message(),
            );
            return;
        };

        if let Some(message) = self.empty_root_state_message(root_id) {
            self.paint_state_message(&painter, available, ui.style(), p, &message);
            return;
        }

        let canvas_changed = self.last_canvas_rect != Some(available);
        if canvas_changed {
            self.last_canvas_rect = Some(available);
            self.layout_dirty = true;
            self.last_layout_refresh = Instant::now()
                .checked_sub(LAYOUT_REFRESH_INTERVAL)
                .unwrap_or_else(Instant::now);
        }

        let should_refresh_layout = self.layout_dirty
            && (!self.scan.is_scanning()
                || self.last_layout_refresh.elapsed() >= LAYOUT_REFRESH_INTERVAL);

        if should_refresh_layout {
            let layout_start = Instant::now();
            layout_treemap(
                &mut self.tree,
                TreemapLayoutParams {
                    root: root_id,
                    canvas_rect: available,
                    camera: self.camera,
                    max_depth: self.max_depth,
                    search_state: self.search.state(),
                    filter_to_search: self.search_filter_enabled,
                    out: &mut self.cached_visuals,
                    scratch: &mut self.layout_scratch,
                },
            );
            self.layout_dirty = false;
            self.last_layout_refresh = Instant::now();
            self.scan.record_layout_recompute(layout_start.elapsed());
        }

        self.hovered_visual_kind = find_hovered_visual(&self.cached_visuals, response.hover_pos())
            .map(|visual| visual.kind);
        self.hovered_id = self.hovered_visual_kind.map(|kind| match kind {
            VisualKind::Node(node_id) => node_id,
        });

        if response.secondary_clicked() {
            self.context_menu_target_id = self.hovered_id;
        }

        if response.dragged() && self.search.input().is_empty() {
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
        if response.double_clicked() {
            if let Some(node_id) = self.hovered_id {
                if !self.tree.node(node_id).children.is_empty() {
                    self.enter_root(node_id, true);
                } else {
                    self.navigation.set_selected_id(Some(node_id));
                }
            } else {
                self.reset_camera();
            }
        } else if response.clicked() {
            if let Some(node_id) = self.hovered_id {
                self.navigation.set_selected_id(Some(node_id));
            } else {
                self.navigation.set_selected_id(None);
            }
        }

        response.context_menu(|ui| {
            if let Some(node_id) = self.context_menu_target_id {
                let p = palette(ui.ctx());
                let node_path = self.tree.node_real_path(node_id);
                let node = self.tree.node(node_id);
                let node_name = node.name.clone();
                let node_size = node.size;
                ui.set_min_width(CONTEXT_MENU_MIN_WIDTH);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(truncate_middle(&node_name, CONTEXT_MENU_MAX_TITLE_CHARS))
                            .strong()
                            .color(p.text),
                    );
                    ui.label(
                        RichText::new(format_bytes(node_size))
                            .small()
                            .monospace()
                            .color(p.text_muted),
                    );
                    ui.separator();
                    if ui
                        .add_enabled(node_path.is_some(), egui::Button::new("Open"))
                        .clicked()
                    {
                        if let Some(path) = &node_path {
                            self.apply_platform_result("Open", open_path(path));
                        }
                        ui.close();
                    }
                    if ui
                        .add_enabled(node_path.is_some(), egui::Button::new("Reveal in Finder"))
                        .clicked()
                    {
                        if let Some(path) = &node_path {
                            self.apply_platform_result("Reveal", reveal_in_finder(path));
                        }
                        ui.close();
                    }
                    if ui
                        .add_enabled(node_path.is_some(), egui::Button::new("Copy Path"))
                        .clicked()
                    {
                        if let Some(path) = &node_path {
                            ui.ctx().copy_text(path.display().to_string());
                        }
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

    fn show_hover_tooltip(&mut self, ui: &egui::Ui, node_id: NodeId, pos: Pos2) {
        let p = palette(ui.ctx());
        let node_path = self.tree.node_real_path(node_id);
        let node = self.tree.node(node_id);
        egui::Area::new(egui::Id::new("hover_tooltip"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos + egui::vec2(16.0, 16.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::default()
                    .fill(p.panel_elevated)
                    .stroke(Stroke::new(1.0, p.stroke_subtle))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(10))
                    .shadow(ui.visuals().popup_shadow)
                    .show(ui, |ui| {
                        ui.set_max_width(260.0);
                        ui.spacing_mut().item_spacing.y = 3.0;

                        ui.horizontal(|ui| {
                            let dot_color = if node.scanned { p.text_faint } else { p.accent };
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                            ui.painter().circle_filled(rect.center(), 3.0, dot_color);
                            ui.add(
                                egui::Label::new(RichText::new(&node.name).strong().color(p.text))
                                    .truncate(),
                            );
                        });

                        ui.label(
                            RichText::new(format_bytes(node.size))
                                .monospace()
                                .color(p.accent),
                        );

                        if let Some(path) = &node_path {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(path.display().to_string())
                                        .small()
                                        .monospace()
                                        .color(p.text_muted),
                                )
                                .truncate(),
                            );
                        } else {
                            ui.label(
                                RichText::new("Aggregated small files")
                                    .small()
                                    .color(p.text_faint),
                            );
                        }

                        if let Some(error) = &node.error {
                            ui.add(
                                egui::Label::new(RichText::new(error).small().color(p.danger))
                                    .wrap(),
                            );
                        }
                    });
            });
    }

    fn paint_visual(&self, ui: &egui::Ui, painter: &egui::Painter, visual: &VisualNode) {
        let palette = palette(ui.ctx());
        let is_hovered =
            matches!(visual.kind, VisualKind::Node(node_id) if self.hovered_id == Some(node_id));
        let is_selected = matches!(visual.kind, VisualKind::Node(node_id) if self.navigation.selected_id() == Some(node_id));
        let extension_color = self.extension_color_for_visual(visual);
        let fill = fill_color_for_visual(visual, is_hovered, is_selected, palette, extension_color);
        let stroke = stroke_for_visual(visual, is_hovered, is_selected, palette);

        painter.rect_filled(visual.rect, 3.0, fill);
        painter.rect_stroke(visual.rect, 3.0, stroke, egui::StrokeKind::Inside);

        if is_selected {
            painter.rect_stroke(
                visual.rect.expand(2.0),
                4.0,
                Stroke::new(3.0, palette.accent_soft),
                egui::StrokeKind::Outside,
            );
        }

        if let Some(label_text) = &visual.label_text {
            painter.text(
                visual.rect.center(),
                egui::Align2::CENTER_CENTER,
                label_text,
                egui::TextStyle::Small.resolve(ui.style()),
                pick_label_color(fill),
            );
        }
    }

    fn extension_color_for_visual(&self, visual: &VisualNode) -> Option<Color32> {
        if !self.color_by_extension || visual.is_dir {
            return None;
        }
        let VisualKind::Node(node_id) = visual.kind;
        let node = self.tree.node(node_id);
        if !matches!(
            node.kind,
            NodeKind::File | NodeKind::Symlink | NodeKind::Error
        ) {
            return None;
        }
        Some(color_for_extension(&node.name))
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if !ctx.egui_wants_keyboard_input()
            && ctx.input(|input| input.key_pressed(egui::Key::Enter))
        {
            self.enter_selected_directory();
        }

        if !ctx.egui_wants_keyboard_input()
            && ctx.input(|input| input.key_pressed(egui::Key::Backspace))
        {
            self.navigate_back();
        }

        if ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::ArrowLeft)) {
            self.navigate_back();
        }

        if ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::ArrowRight)) {
            self.navigate_forward();
        }

        if !ctx.egui_wants_keyboard_input()
            && ctx.input(|input| input.key_pressed(egui::Key::CloseBracket))
        {
            self.increase_depth();
        }

        if !ctx.egui_wants_keyboard_input()
            && ctx.input(|input| input.key_pressed(egui::Key::OpenBracket))
        {
            self.decrease_depth();
        }

        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            if self.navigation.selected_id().is_some() {
                self.navigation.set_selected_id(None);
            } else if !self.search.input().is_empty() {
                self.clear_search();
            }
        }
    }

    fn handle_scan_messages(&mut self) {
        let mut saw_batch = false;

        while let Ok(message) = self.rx.try_recv() {
            if !self.scan.accepts(&message) {
                continue;
            }

            match message {
                ScanMessage::Started {
                    path, root_node, ..
                } => {
                    self.scan.mark_started();
                    self.status = format!("Scanning {}", path.display());
                    self.record_recent_root(&path);
                    self.tree.clear();
                    self.tree.push_node(None, root_node);
                    self.tree.set_root_path(path);
                    self.navigation.set_scan_root(self.tree.root);
                    self.layout_dirty = true;
                    self.mark_search_dirty();
                    self.navigation.rebuild_breadcrumb_cache(&self.tree);
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
                    self.scan.mark_finished(perf_stats);
                    self.prune_invalid_selection();
                    self.refresh_search_matches();
                    self.layout_dirty = true;
                    self.status = self.finished_status(total_bytes);
                    self.update_snapshot_comparison();
                    self.update_watch_state();
                    self.pending_repaint = true;
                    eprintln!("{}", format_perf_stats(self.scan.perf_stats()));
                }
                ScanMessage::Cancelled { perf_stats, .. } => {
                    self.scan.mark_cancelled(perf_stats);
                    self.status = "Scan cancelled".to_string();
                    self.pending_repaint = true;
                    eprintln!("{}", format_perf_stats(self.scan.perf_stats()));
                }
                ScanMessage::Error {
                    message,
                    perf_stats,
                    ..
                } => {
                    self.scan.mark_error(perf_stats);
                    self.status = format!("Error: {message}");
                    self.pending_repaint = true;
                    eprintln!("{}", format_perf_stats(self.scan.perf_stats()));
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
            self.scan.observe_node(&discovered.node);
            self.tree
                .insert_node(node_id, Some(parent_id), discovered.node);
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
        let visible_dirty_nodes: Vec<NodeId> = dirty_nodes
            .iter()
            .copied()
            .filter(|&node_id| self.batch_touches_visible_subtree(node_id))
            .collect();
        if !visible_dirty_nodes.is_empty() {
            self.tree.repair_sorted_children(&visible_dirty_nodes);
        }
        let touched_visible_subtree = dirty_nodes
            .iter()
            .copied()
            .any(|node_id| self.batch_touches_visible_subtree(node_id));
        if let Some(progress) = batch.progress {
            self.apply_progress(progress);
        }

        if !discovered_node_ids.is_empty() && !self.search.query().is_empty() {
            let updates =
                self.search
                    .ingest_new_nodes(&mut self.tree, &discovered_node_ids) as u64;
            self.scan.record_search_incremental_updates(updates);
        }

        if touched_visible_subtree {
            self.layout_dirty = true;
        }
    }

    fn apply_progress(&mut self, progress: ProgressSnapshot) {
        self.scan.apply_progress(progress);
        self.status = "Scanning...".to_string();
    }

    fn current_root_candidate(&mut self) -> Option<String> {
        self.scan_root_rescan_path()
            .or_else(|| normalized_path_candidate(&self.path_input).map(PathBuf::from))
            .map(|path| path.display().to_string())
    }

    fn is_root_pinned(&self, path: &str) -> bool {
        self.pinned_roots.iter().any(|existing| existing == path)
    }

    fn toggle_pinned_root(&mut self, path: &str) {
        if let Some(index) = self
            .pinned_roots
            .iter()
            .position(|existing| existing == path)
        {
            self.pinned_roots.remove(index);
            self.status = format!("Unpinned {}", truncate_middle(path, 48));
        } else {
            push_unique_front(&mut self.pinned_roots, path.to_string(), MAX_PINNED_ROOTS);
            self.status = format!("Pinned {}", truncate_middle(path, 48));
        }
        self.pending_repaint = true;
    }

    fn record_recent_root(&mut self, path: &Path) {
        push_unique_front(
            &mut self.recent_roots,
            path.display().to_string(),
            MAX_RECENT_ROOTS,
        );
    }

    fn update_snapshot_comparison(&mut self) {
        let Some(root_id) = self.tree.root else {
            self.snapshot_diff = None;
            return;
        };
        let Some(current_snapshot) = capture_snapshot(&mut self.tree, root_id) else {
            self.snapshot_diff = None;
            return;
        };

        self.snapshot_diff = self
            .last_snapshot
            .as_ref()
            .filter(|previous| previous.root_path == current_snapshot.root_path)
            .map(|previous| compare_snapshots(previous, &current_snapshot, SNAPSHOT_DIFF_LIMIT));
        self.last_snapshot = Some(current_snapshot);
    }

    fn finished_status(&self, total_bytes: u64) -> String {
        let issue_count = self.scan.issue_summary().issue_count();
        if issue_count == 0 {
            return format!("Finished: {}", format_bytes(total_bytes));
        }

        format!(
            "Finished: {} · {}",
            format_bytes(total_bytes),
            pluralize(issue_count, "issue", "issues")
        )
    }

    fn clear_search(&mut self) {
        self.search.clear(self.tree.len());
        self.layout_dirty = true;
    }

    fn apply_platform_result(&mut self, action: &str, result: anyhow::Result<()>) {
        if let Err(error) = result {
            self.status = format!("{action} failed: {error}");
            self.pending_repaint = true;
        }
    }

    fn queue_cleanup_candidate(&mut self, node_id: NodeId) {
        if !self.destructive_actions_enabled {
            self.status = "Move to Trash is disabled".to_string();
            self.pending_repaint = true;
            return;
        }

        let Some(path) = self.tree.node_real_path(node_id) else {
            self.status = "Cleanup queue unavailable for virtual nodes".to_string();
            self.pending_repaint = true;
            return;
        };

        if let Some(reason) = protected_path_reason(&path) {
            self.status = protected_path_status(reason, &path);
            self.pending_repaint = true;
            return;
        }

        let node = self.tree.node(node_id);
        let candidate = CleanupCandidate {
            node_id,
            name: node.name.clone(),
            path: path.clone(),
            size: node.size,
            item_count: self.cleanup_item_count(node_id),
            kind: node.kind,
        };
        self.status = match self.cleanup_queue.add(candidate) {
            QueueAddResult::Added => format!("Queued cleanup candidate: {}", path.display()),
            QueueAddResult::AlreadyQueued => {
                format!("Cleanup candidate already queued: {}", path.display())
            }
        };
        self.pending_repaint = true;
    }

    fn arm_or_confirm_queued_trash(&mut self, node_id: NodeId) {
        if !self.destructive_actions_enabled {
            self.status = "Move to Trash is disabled".to_string();
            self.pending_repaint = true;
            return;
        }

        let Some(candidate) = self.cleanup_queue.get(node_id).cloned() else {
            self.status = "Move to Trash unavailable: candidate is not queued".to_string();
            self.pending_repaint = true;
            return;
        };

        if let Some(reason) = protected_path_reason(&candidate.path) {
            self.trash_confirm_target_id = None;
            self.status = protected_path_status(reason, &candidate.path);
            self.pending_repaint = true;
            return;
        }

        if self.trash_confirm_target_id != Some(node_id) {
            self.trash_confirm_target_id = Some(node_id);
            self.status = format!(
                "Confirm Trash: {} · {} · {}",
                candidate.path.display(),
                format_bytes(candidate.size),
                pluralize(candidate.item_count as u64, "item", "items")
            );
            self.pending_repaint = true;
            return;
        }

        self.trash_confirm_target_id = None;
        match move_to_trash(&candidate.path) {
            Ok(()) => {
                self.cleanup_queue.remove(node_id);
                self.status = format!("Moved to Trash: {}", candidate.path.display());
            }
            Err(error) => {
                self.status = format!("Move to Trash failed: {error}");
            }
        }
        self.pending_repaint = true;
    }

    fn remove_cleanup_candidate(&mut self, node_id: NodeId) {
        if let Some(candidate) = self.cleanup_queue.remove(node_id) {
            if self.trash_confirm_target_id == Some(node_id) {
                self.trash_confirm_target_id = None;
            }
            self.status = format!("Removed cleanup candidate: {}", candidate.path.display());
            self.pending_repaint = true;
        }
    }

    fn cleanup_item_count(&self, node_id: NodeId) -> usize {
        if node_id >= self.tree.len() {
            return 0;
        }

        let mut count = 1usize;
        let mut stack = self.tree.node(node_id).children.clone();
        while let Some(id) = stack.pop() {
            if id >= self.tree.len() {
                continue;
            }
            count += 1;
            stack.extend(self.tree.node(id).children.iter().copied());
        }
        count
    }

    fn handle_watch_events(&mut self) {
        let Some(watcher) = &mut self.watcher else {
            return;
        };

        match watcher.poll(Instant::now()) {
            WatchPoll::Noop => {}
            WatchPoll::Pending => {
                self.pending_repaint = true;
            }
            WatchPoll::Ready(change) => {
                let change_count = change.paths.len();
                if self.scan.is_scanning() || self.incremental_scan_active {
                    self.status = format!(
                        "Watch noticed {} while scan is running",
                        pluralize(change_count as u64, "change", "changes")
                    );
                    self.pending_repaint = true;
                    return;
                }
                if let Some((target_id, path)) = self.incremental_rescan_target(&change.paths) {
                    self.start_incremental_scan(target_id, path, change_count);
                } else if let Some(path) = self.scan_root_rescan_path() {
                    self.status = "Watch fallback rescan after unresolved change".to_string();
                    self.start_scan_path(path);
                }
            }
            WatchPoll::Error(error) => {
                self.status = format!("Watch failed: {error}");
                self.watcher = None;
                self.pending_repaint = true;
            }
        }
    }

    fn handle_incremental_scan_results(&mut self) {
        while let Ok(result) = self.incremental_rx.try_recv() {
            self.incremental_scan_active = false;
            match result.result {
                Ok(source_tree) => {
                    let Some(new_ids) = self
                        .tree
                        .replace_children_from(result.target_id, &source_tree)
                    else {
                        self.status = format!(
                            "Incremental rescan skipped: target changed {}",
                            result.path.display()
                        );
                        self.pending_repaint = true;
                        continue;
                    };
                    let mut dirty_nodes = new_ids;
                    dirty_nodes.push(result.target_id);
                    dirty_nodes.extend(self.tree.ancestors(result.target_id));
                    dirty_nodes.sort_unstable();
                    dirty_nodes.dedup();
                    self.tree.repair_sorted_children(&dirty_nodes);
                    self.navigation.prune_invalid(&self.tree);
                    self.navigation.rebuild_breadcrumb_cache(&self.tree);
                    self.refresh_search_matches();
                    self.layout_dirty = true;
                    self.status = format!("Updated {}", result.path.display());
                    self.pending_repaint = true;
                }
                Err(error) => {
                    self.status = format!(
                        "Incremental rescan failed for {}: {error}",
                        result.path.display()
                    );
                    self.pending_repaint = true;
                }
            }
        }
    }

    fn start_incremental_scan(&mut self, target_id: NodeId, path: PathBuf, change_count: usize) {
        self.incremental_scan_active = true;
        self.status = format!(
            "Updating {} after {}",
            path.display(),
            pluralize(change_count as u64, "change", "changes")
        );
        let options = self.scan_options();
        let tx = self.incremental_tx.clone();
        std::thread::spawn(move || {
            let result = scan_path_to_tree(path.clone(), options);
            let _ = tx.send(IncrementalScanResult {
                target_id,
                path,
                result,
            });
        });
        self.pending_repaint = true;
    }

    fn incremental_rescan_target(
        &mut self,
        changed_paths: &[PathBuf],
    ) -> Option<(NodeId, PathBuf)> {
        let mut best: Option<(NodeId, PathBuf, usize)> = None;
        for changed_path in changed_paths {
            let Some((node_id, node_path)) = self.closest_known_directory(changed_path) else {
                continue;
            };
            let depth = node_path.components().count();
            if best
                .as_ref()
                .is_none_or(|(_, _, best_depth)| depth > *best_depth)
            {
                best = Some((node_id, node_path, depth));
            }
        }
        best.map(|(node_id, path, _)| (node_id, path))
    }

    fn closest_known_directory(&mut self, changed_path: &Path) -> Option<(NodeId, PathBuf)> {
        let root = self.tree.root?;
        let mut best: Option<(NodeId, PathBuf, usize)> = None;
        for node_id in 0..self.tree.len() {
            if !matches!(self.tree.node(node_id).kind, NodeKind::Dir) {
                continue;
            }
            if !self.tree.is_descendant_or_same(node_id, root) {
                continue;
            }
            let path = self.tree.node_real_path(node_id)?;
            if !changed_path.starts_with(&path) {
                continue;
            }
            let depth = path.components().count();
            if best
                .as_ref()
                .is_none_or(|(_, _, best_depth)| depth > *best_depth)
            {
                best = Some((node_id, path, depth));
            }
        }
        best.map(|(node_id, path, _)| (node_id, path))
    }

    fn start_scan(&mut self) {
        let path = std::path::PathBuf::from(self.path_input.trim());
        self.start_scan_path(path);
    }

    fn start_scan_path(&mut self, path: std::path::PathBuf) {
        self.stop_watching();
        self.incremental_scan_active = false;
        self.scan
            .start(path.clone(), self.scan_options(), self.tx.clone());

        self.tree.clear();
        self.navigation.clear_for_new_scan();
        self.hovered_id = None;
        self.context_menu_target_id = None;
        self.trash_confirm_target_id = None;
        self.cleanup_queue.clear();
        self.hovered_visual_kind = None;
        self.snapshot_diff = None;
        self.duplicate_report = None;
        self.insight_report = None;
        self.search.clear(0);
        self.cached_visuals.clear();
        self.reset_camera();
        self.layout_dirty = true;
        self.path_input = path.display().to_string();
        self.status = format!("Scanning {}", path.display());
        self.pending_repaint = true;
    }

    fn update_watch_state(&mut self) {
        if !self.realtime_watch_enabled {
            self.stop_watching();
            return;
        }

        let Some(root_path) = self.scan_root_rescan_path() else {
            self.stop_watching();
            return;
        };

        if self
            .watcher
            .as_ref()
            .is_some_and(|watcher| watcher.root_path() == &root_path)
        {
            return;
        }

        match WatchSession::start(root_path.clone()) {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.status = format!("Watching {}", root_path.display());
                self.pending_repaint = true;
            }
            Err(error) => {
                self.watcher = None;
                self.status = format!("Watch failed: {error}");
                self.pending_repaint = true;
            }
        }
    }

    fn stop_watching(&mut self) {
        self.watcher = None;
    }

    fn can_rescan_scan_root(&mut self) -> bool {
        !self.scan.is_scanning() && self.scan_root_rescan_path().is_some()
    }

    fn can_rescan_focused_subtree(&mut self) -> bool {
        !self.scan.is_scanning() && self.focused_subtree_rescan_path().is_some()
    }

    fn rescan_scan_root(&mut self) {
        let Some(path) = self.scan_root_rescan_path() else {
            self.status = "Rescan unavailable: no scan root".to_string();
            self.pending_repaint = true;
            return;
        };
        self.start_scan_path(path);
    }

    fn rescan_focused_subtree(&mut self) {
        let Some(path) = self.focused_subtree_rescan_path() else {
            self.status = "Rescan unavailable: no focused directory".to_string();
            self.pending_repaint = true;
            return;
        };
        self.start_scan_path(path);
    }

    fn scan_root_rescan_path(&mut self) -> Option<std::path::PathBuf> {
        self.tree
            .root
            .and_then(|root_id| self.tree.node_real_path(root_id))
    }

    fn focused_subtree_rescan_path(&mut self) -> Option<std::path::PathBuf> {
        self.navigation
            .focused_root()
            .filter(|&root_id| {
                root_id < self.tree.len() && matches!(self.tree.node(root_id).kind, NodeKind::Dir)
            })
            .and_then(|root_id| self.tree.node_real_path(root_id))
    }

    fn export_focused_subtree(&mut self, format: ExportFormat) {
        let Some(root_id) = self.navigation.focused_root() else {
            self.status = "Export unavailable: no focused directory".to_string();
            self.pending_repaint = true;
            return;
        };

        match self.write_focused_export(root_id, format) {
            Ok(path) => {
                self.status = format!("Exported {} to {}", format.label(), path.display());
            }
            Err(error) => {
                self.status = format!("Export failed: {error}");
            }
        }
        self.pending_repaint = true;
    }

    fn export_scan_root(&mut self, format: ExportFormat) {
        let Some(root_id) = self.tree.root else {
            self.status = "Export unavailable: no scan root".to_string();
            self.pending_repaint = true;
            return;
        };

        match self.write_focused_export(root_id, format) {
            Ok(path) => {
                self.status = format!("Exported {} to {}", format.label(), path.display());
            }
            Err(error) => {
                self.status = format!("Export failed: {error}");
            }
        }
        self.pending_repaint = true;
    }

    fn export_focused_report_json(&mut self) {
        let Some(root_id) = self.navigation.focused_root() else {
            self.status = "Report export unavailable: no focused directory".to_string();
            self.pending_repaint = true;
            return;
        };

        match self.write_focused_report(root_id) {
            Ok(path) => {
                self.status = format!("Exported report to {}", path.display());
            }
            Err(error) => {
                self.status = format!("Report export failed: {error}");
            }
        }
        self.pending_repaint = true;
    }

    fn analyze_duplicate_candidates(&mut self) {
        let Some(root_id) = self.navigation.focused_root() else {
            self.duplicate_report = None;
            self.status = "Duplicate analysis unavailable: no focused directory".to_string();
            self.pending_repaint = true;
            return;
        };

        match find_duplicate_candidates(&mut self.tree, root_id, DUPLICATE_REPORT_LIMIT) {
            Some(report) => {
                let status = if report.group_count == 0 {
                    "Duplicate analysis found no candidates".to_string()
                } else {
                    format!(
                        "Duplicate analysis found {}",
                        pluralize(
                            report.group_count as u64,
                            "candidate group",
                            "candidate groups"
                        )
                    )
                };
                self.duplicate_report = Some(report);
                self.status = status;
            }
            None => {
                self.duplicate_report = None;
                self.status = "Duplicate analysis unavailable for this view".to_string();
            }
        }
        self.pending_repaint = true;
    }

    fn analyze_file_insights(&mut self) {
        let Some(root_id) = self.navigation.focused_root() else {
            self.insight_report = None;
            self.status = "Insights unavailable: no focused directory".to_string();
            self.pending_repaint = true;
            return;
        };

        match analyze_insights(
            &mut self.tree,
            root_id,
            current_unix_secs(),
            INSIGHT_REPORT_LIMIT,
        ) {
            Some(report) => {
                self.status = format!(
                    "Insights analyzed {}",
                    pluralize(report.file_count as u64, "file", "files")
                );
                self.insight_report = Some(report);
            }
            None => {
                self.insight_report = None;
                self.status = "Insights unavailable for this view".to_string();
            }
        }
        self.pending_repaint = true;
    }

    fn write_focused_export(
        &mut self,
        root_id: NodeId,
        format: ExportFormat,
    ) -> anyhow::Result<PathBuf> {
        if root_id >= self.tree.len() {
            anyhow::bail!("focused directory is no longer available");
        }

        let content = export_subtree(&mut self.tree, root_id, format);
        let output_path = default_export_path(format);
        std::fs::write(&output_path, content)?;
        Ok(output_path)
    }

    fn write_focused_report(&mut self, root_id: NodeId) -> anyhow::Result<PathBuf> {
        if root_id >= self.tree.len() {
            anyhow::bail!("focused directory is no longer available");
        }

        let metadata = self.focused_report_metadata(root_id)?;
        let content = export_focused_report(&mut self.tree, root_id, &metadata);
        let output_path = default_report_path();
        std::fs::write(&output_path, content)?;
        Ok(output_path)
    }

    fn focused_report_metadata(
        &mut self,
        root_id: NodeId,
    ) -> anyhow::Result<FocusedReportMetadata> {
        let scan_root_id = self
            .tree
            .root
            .ok_or_else(|| anyhow::anyhow!("scan root is no longer available"))?;
        let scan_root_path = self
            .tree
            .node_real_path(scan_root_id)
            .ok_or_else(|| anyhow::anyhow!("scan root has no real path"))?;
        let focused_path = self
            .tree
            .node_real_path(root_id)
            .ok_or_else(|| anyhow::anyhow!("focused node has no real path"))?;

        Ok(FocusedReportMetadata {
            generated_at_unix_secs: current_unix_secs(),
            scan_root_path: scan_root_path.display().to_string(),
            focused_path: focused_path.display().to_string(),
            size_basis: size_basis_label(),
            max_depth: self.max_depth,
            search_query: self.search.query().to_string(),
            search_filter_enabled: self.search_filter_enabled,
            color_mode: if self.color_by_extension {
                "extension"
            } else {
                "directory-depth"
            },
            include_hidden: self.include_hidden,
            follow_symlinks: self.follow_symlinks,
            stay_on_filesystem: self.stay_on_filesystem,
            sqlite_cache_enabled: self.sqlite_cache_enabled,
            realtime_watch_enabled: self.realtime_watch_enabled,
            exclude_patterns: parse_exclude_patterns(&self.exclude_input),
        })
    }

    fn cancel_scan(&mut self) {
        if self.scan.cancel() {
            self.status = "Cancelling scan...".to_string();
            self.pending_repaint = true;
        }
    }

    fn enter_root(&mut self, node_id: NodeId, push_history: bool) {
        let outcome = self.navigation.enter_root(node_id, push_history);
        self.apply_navigation_outcome(outcome);
    }

    fn return_to_scan_root(&mut self) {
        let outcome = self.navigation.return_to_scan_root(&self.tree);
        self.apply_navigation_outcome(outcome);
    }

    fn navigate_back(&mut self) {
        let outcome = self.navigation.navigate_back();
        self.apply_navigation_outcome(outcome);
    }

    fn navigate_forward(&mut self) {
        let outcome = self.navigation.navigate_forward();
        self.apply_navigation_outcome(outcome);
    }

    fn reset_camera(&mut self) {
        self.camera = Camera::default();
        self.mark_layout_dirty_now();
    }

    fn mark_layout_dirty_now(&mut self) {
        self.layout_dirty = true;
        self.last_layout_refresh = Instant::now()
            .checked_sub(LAYOUT_REFRESH_INTERVAL)
            .unwrap_or_else(Instant::now);
    }

    fn apply_navigation_outcome(&mut self, outcome: NavigationOutcome) {
        match outcome {
            NavigationOutcome::Noop => {}
            NavigationOutcome::ResetCameraOnly => self.reset_camera(),
            NavigationOutcome::FocusChanged { refresh_search } => {
                self.reset_camera();
                if refresh_search {
                    self.refresh_search_matches();
                }
                self.layout_dirty = true;
                self.navigation.rebuild_breadcrumb_cache(&self.tree);
            }
        }
    }

    fn refresh_search_matches(&mut self) {
        self.search
            .refresh(&mut self.tree, self.navigation.focused_root());
        self.scan.record_search_rebuild();
    }

    fn maybe_refresh_search(&mut self, ctx: &egui::Context) {
        if self.search.maybe_refresh_due(self.scan.is_scanning()) {
            self.refresh_search_matches();
            self.layout_dirty = true;
            self.pending_repaint = true;
            ctx.request_repaint();
        }
    }

    fn mark_search_dirty(&mut self) {
        self.search.mark_dirty();
    }

    fn can_navigate_search_matches(&self) -> bool {
        self.search.can_navigate()
    }

    fn navigate_search_match(&mut self, direction: SearchDirection) {
        if self.search.query().is_empty() {
            return;
        }
        if self.search.is_dirty() {
            self.refresh_search_matches();
        }

        if let Some(node_id) = self.search.next_match(direction, &self.tree) {
            let outcome = self.navigation.focus_search_match(&self.tree, node_id);
            self.apply_navigation_outcome(outcome);
        }
    }

    fn enter_selected_directory(&mut self) -> bool {
        let Some(selected_id) = self.navigation.selected_id() else {
            return false;
        };
        if selected_id >= self.tree.len() || self.tree.node(selected_id).children.is_empty() {
            return false;
        }
        self.enter_root(selected_id, true);
        true
    }

    fn increase_depth(&mut self) -> bool {
        if self.max_depth >= 10 {
            return false;
        }
        self.max_depth += 1;
        self.mark_layout_dirty_now();
        true
    }

    fn decrease_depth(&mut self) -> bool {
        if self.max_depth <= 1 {
            return false;
        }
        self.max_depth -= 1;
        self.mark_layout_dirty_now();
        true
    }

    fn prune_invalid_selection(&mut self) {
        self.navigation.prune_invalid(&self.tree);
    }

    fn batch_touches_visible_subtree(&self, node_id: NodeId) -> bool {
        if let Some(root_id) = self.navigation.focused_root() {
            self.tree.is_descendant_or_same(node_id, root_id)
                || self.tree.is_descendant_or_same(root_id, node_id)
        } else {
            true
        }
    }

    fn maybe_request_deferred_repaint(&mut self, ctx: &egui::Context) {
        if self.pending_repaint {
            ctx.request_repaint();
            self.pending_repaint = false;
        }
    }

    fn drive_background_updates(&self, ctx: &egui::Context) {
        if self.scan.is_scanning() {
            // Keep the UI alive while scan batches arrive, even when there is no user input.
            ctx.request_repaint_after(LAYOUT_REFRESH_INTERVAL);
        } else if self.watcher.is_some() {
            ctx.request_repaint_after(LAYOUT_REFRESH_INTERVAL);
        } else if self.search.is_dirty() {
            ctx.request_repaint_after(SEARCH_REFRESH_INTERVAL);
        }
    }

    #[cfg(test)]
    fn apply_scan_message_for_test(&mut self, message: ScanMessage) {
        if !self.scan.accepts(&message) {
            return;
        }

        match message {
            ScanMessage::Started {
                path, root_node, ..
            } => {
                self.tree.clear();
                self.tree.push_node(None, root_node);
                self.tree.set_root_path(path.clone());
                self.record_recent_root(&path);
                self.navigation.set_scan_root(self.tree.root);
                self.navigation.rebuild_breadcrumb_cache(&self.tree);
            }
            ScanMessage::Batch { batch, .. } => self.apply_scan_batch(batch),
            ScanMessage::Finished { .. } => self.update_snapshot_comparison(),
            ScanMessage::Cancelled { .. } | ScanMessage::Error { .. } => {}
        }
    }

    #[cfg(test)]
    fn set_active_scan_id_for_test(&mut self, scan_id: u64) {
        self.scan.set_active_id_for_test(scan_id);
    }
}

fn find_hovered_visual(visuals: &[VisualNode], pos: Option<Pos2>) -> Option<&VisualNode> {
    let pos = pos?;
    visuals.iter().rev().find(|visual| {
        visual.rect.contains(pos) && visual.rect.width() >= 2.0 && visual.rect.height() >= 2.0
    })
}

fn fill_color_for_visual(
    visual: &VisualNode,
    hovered: bool,
    selected: bool,
    palette: &Palette,
    extension_color: Option<Color32>,
) -> Color32 {
    let mut color = if visual.is_dir {
        palette.dir_palette[visual.depth % palette.dir_palette.len()]
    } else {
        extension_color.unwrap_or(palette.file_neutral)
    };

    if visual.hidden_by_search {
        color = color.gamma_multiply(0.35);
    } else if visual.ancestor_of_match {
        color = color.gamma_multiply(0.85);
    } else if visual.matched {
        color = color.gamma_multiply(1.10);
    }

    if hovered {
        color = color.gamma_multiply(1.10);
    }
    if selected {
        color = color.gamma_multiply(1.12);
    }

    color
}

fn color_for_extension(name: &str) -> Color32 {
    const COLORS: [Color32; 10] = [
        Color32::from_rgb(0x6B, 0xA6, 0xD6),
        Color32::from_rgb(0x5B, 0xB8, 0x8A),
        Color32::from_rgb(0xD9, 0xA6, 0x4A),
        Color32::from_rgb(0xD7, 0x6A, 0x6A),
        Color32::from_rgb(0x9B, 0x7C, 0xD8),
        Color32::from_rgb(0x4D, 0xB6, 0xAC),
        Color32::from_rgb(0xC7, 0x78, 0xB7),
        Color32::from_rgb(0x8D, 0xA3, 0x4F),
        Color32::from_rgb(0x6F, 0x8F, 0xD8),
        Color32::from_rgb(0xB8, 0x86, 0x5B),
    ];
    let ext = file_extension_key(name);
    let mut hash = 0usize;
    for byte in ext.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as usize);
    }
    COLORS[hash % COLORS.len()]
}

fn file_extension_key(name: &str) -> String {
    let trimmed = name.trim();
    let Some((_, ext)) = trimmed.rsplit_once('.') else {
        return String::new();
    };
    ext.to_ascii_lowercase()
}

fn stroke_for_visual(
    visual: &VisualNode,
    hovered: bool,
    selected: bool,
    palette: &Palette,
) -> Stroke {
    if selected {
        Stroke::new(1.5, palette.accent)
    } else if hovered {
        Stroke::new(1.5, palette.stroke_strong)
    } else if visual.matched {
        Stroke::new(1.5, palette.accent)
    } else if visual.is_dir {
        Stroke::new(1.0, palette.stroke_subtle)
    } else {
        Stroke::NONE
    }
}

fn describe_node_kind(kind: NodeKind, has_children: bool) -> &'static str {
    match kind {
        NodeKind::Dir if has_children => "Directory",
        NodeKind::Dir => "Empty directory",
        NodeKind::File => "File",
        NodeKind::Symlink => "Symlink",
        NodeKind::Error => "Error entry",
        NodeKind::Aggregate => "Aggregated files",
    }
}

#[derive(Debug, Clone, Copy)]
enum ToolbarIcon {
    ArrowLeft,
    ArrowRight,
    Up,
    Home,
    Refresh,
    Close,
    ThemeLight,
    ThemeDark,
}

fn icon_button(ui: &mut egui::Ui, enabled: bool, icon: ToolbarIcon) -> egui::Response {
    let desired_size = Vec2::new(28.0, 28.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(desired_size, sense);
    let visuals = ui.style().interact(&response);
    let fill = if enabled {
        visuals.bg_fill
    } else {
        ui.visuals().widgets.inactive.weak_bg_fill
    };
    let stroke = if enabled {
        visuals.bg_stroke
    } else {
        ui.visuals().widgets.inactive.bg_stroke
    };
    ui.painter().rect(
        rect,
        visuals.corner_radius,
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );

    let icon_color = if enabled {
        visuals.fg_stroke.color
    } else {
        ui.visuals().weak_text_color()
    };
    paint_toolbar_icon(ui.painter(), rect, icon, icon_color, fill);
    response
}

fn paint_toolbar_icon(
    painter: &egui::Painter,
    rect: Rect,
    icon: ToolbarIcon,
    color: Color32,
    button_fill: Color32,
) {
    let c = rect.center();
    let stroke = Stroke::new(1.4, color);
    match icon {
        ToolbarIcon::ArrowLeft => arrow_geom(painter, c, false, stroke),
        ToolbarIcon::ArrowRight => arrow_geom(painter, c, true, stroke),
        ToolbarIcon::Up => {
            let tail = Pos2::new(c.x, c.y + 5.5);
            let tip = Pos2::new(c.x, c.y - 5.5);
            painter.line_segment([tail, tip], stroke);
            painter.line_segment([tip, Pos2::new(tip.x - 4.0, tip.y + 4.0)], stroke);
            painter.line_segment([tip, Pos2::new(tip.x + 4.0, tip.y + 4.0)], stroke);
        }
        ToolbarIcon::Home => {
            let roof_left = Pos2::new(c.x - 6.0, c.y - 0.5);
            let roof_top = Pos2::new(c.x, c.y - 6.0);
            let roof_right = Pos2::new(c.x + 6.0, c.y - 0.5);
            painter.line_segment([roof_left, roof_top], stroke);
            painter.line_segment([roof_top, roof_right], stroke);
            let base_min = Pos2::new(c.x - 4.5, c.y - 0.5);
            let base_max = Pos2::new(c.x + 4.5, c.y + 6.0);
            painter.line_segment([base_min, Pos2::new(base_min.x, base_max.y)], stroke);
            painter.line_segment([Pos2::new(base_min.x, base_max.y), base_max], stroke);
            painter.line_segment([base_max, Pos2::new(base_max.x, base_min.y)], stroke);
        }
        ToolbarIcon::Refresh => {
            let r = 5.5;
            let start_angle = -0.35 * std::f32::consts::PI;
            let end_angle = 1.4 * std::f32::consts::PI;
            let steps = 18;
            let mut prev: Option<Pos2> = None;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let theta = start_angle + (end_angle - start_angle) * t;
                let p = Pos2::new(c.x + r * theta.cos(), c.y + r * theta.sin());
                if let Some(p0) = prev {
                    painter.line_segment([p0, p], stroke);
                }
                prev = Some(p);
            }
            let end = Pos2::new(c.x + r * end_angle.cos(), c.y + r * end_angle.sin());
            painter.line_segment([end, Pos2::new(end.x - 2.5, end.y - 3.5)], stroke);
            painter.line_segment([end, Pos2::new(end.x + 3.5, end.y - 1.5)], stroke);
        }
        ToolbarIcon::Close => {
            painter.line_segment(
                [
                    Pos2::new(c.x - 4.5, c.y - 4.5),
                    Pos2::new(c.x + 4.5, c.y + 4.5),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(c.x - 4.5, c.y + 4.5),
                    Pos2::new(c.x + 4.5, c.y - 4.5),
                ],
                stroke,
            );
        }
        ToolbarIcon::ThemeLight => {
            painter.circle_filled(c, 3.0, color);
            for i in 0..8 {
                let theta = i as f32 * std::f32::consts::PI / 4.0;
                let inner = Pos2::new(c.x + 4.5 * theta.cos(), c.y + 4.5 * theta.sin());
                let outer = Pos2::new(c.x + 6.5 * theta.cos(), c.y + 6.5 * theta.sin());
                painter.line_segment([inner, outer], stroke);
            }
        }
        ToolbarIcon::ThemeDark => {
            let r = 6.0;
            painter.circle_filled(c, r, color);
            painter.circle_filled(Pos2::new(c.x + 2.6, c.y - 1.4), r - 0.5, button_fill);
        }
    }
}

fn arrow_geom(painter: &egui::Painter, center: Pos2, point_right: bool, stroke: Stroke) {
    let shaft_half = 5.5;
    let head = 4.0;
    let (start, end, tip_a, tip_b) = if point_right {
        let tail = Pos2::new(center.x - shaft_half, center.y);
        let tip = Pos2::new(center.x + shaft_half, center.y);
        (
            tail,
            tip,
            Pos2::new(tip.x - head, tip.y - head),
            Pos2::new(tip.x - head, tip.y + head),
        )
    } else {
        let tip = Pos2::new(center.x - shaft_half, center.y);
        let tail = Pos2::new(center.x + shaft_half, center.y);
        (
            tail,
            tip,
            Pos2::new(tip.x + head, tip.y - head),
            Pos2::new(tip.x + head, tip.y + head),
        )
    };
    painter.line_segment([start, end], stroke);
    painter.line_segment([end, tip_a], stroke);
    painter.line_segment([end, tip_b], stroke);
}

fn apply_theme_preference(ctx: &egui::Context, theme: Theme) {
    let (preference, system_theme) = match theme {
        Theme::Dark => (egui::ThemePreference::Dark, egui::SystemTheme::Dark),
        Theme::Light => (egui::ThemePreference::Light, egui::SystemTheme::Light),
    };
    ctx.set_theme(preference);
    ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(system_theme));
}

fn parse_theme_preference(value: &str) -> Option<Theme> {
    match value {
        "dark" => Some(Theme::Dark),
        "light" => Some(Theme::Light),
        _ => None,
    }
}

fn parse_storage_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_stored_paths(value: &str, limit: usize) -> Vec<String> {
    let mut paths = Vec::new();
    for line in value.lines() {
        let Some(path) = normalized_path_candidate(line) else {
            continue;
        };
        if paths.iter().any(|existing| existing == &path) {
            continue;
        }
        paths.push(path);
        if paths.len() >= limit {
            break;
        }
    }
    paths
}

fn serialize_paths(paths: &[String]) -> String {
    paths.join("\n")
}

fn normalized_path_candidate(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.contains('\n') || trimmed.contains('\r') {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn push_unique_front(paths: &mut Vec<String>, path: String, limit: usize) {
    if path.is_empty() || limit == 0 {
        return;
    }
    paths.retain(|existing| existing != &path);
    paths.insert(0, path);
    paths.truncate(limit);
}

fn theme_preference_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => "dark",
        Theme::Light => "light",
    }
}

fn theme_cycle_button(ui: &mut egui::Ui) -> Option<Theme> {
    let current = ui.ctx().theme();
    let (icon, tooltip, next_theme) = match current {
        Theme::Dark => (
            ToolbarIcon::ThemeLight,
            "Switch to light mode",
            Theme::Light,
        ),
        Theme::Light => (ToolbarIcon::ThemeDark, "Switch to dark mode", Theme::Dark),
    };
    let response = icon_button(ui, true, icon).on_hover_text(tooltip);
    if response.clicked() {
        apply_theme_preference(ui.ctx(), next_theme);
        Some(next_theme)
    } else {
        None
    }
}

fn section_divider(ui: &mut egui::Ui, palette: &Palette) {
    let (_, rect) = ui.allocate_space(Vec2::new(ui.available_width(), 1.0));
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0, palette.stroke_subtle),
    );
}

fn mini_bar(ui: &mut egui::Ui, fraction: f32, palette: &Palette) {
    let width = 56.0;
    let height = 5.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();
    let radius = CornerRadius::same(3);
    painter.rect_filled(rect, radius, palette.stroke_subtle);
    let frac = fraction.clamp(0.0, 1.0);
    let mut filled = rect;
    filled.set_width(rect.width() * frac);
    if filled.width() > 0.5 {
        painter.rect_filled(filled, radius, palette.accent);
    }
}

fn accent_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    width: f32,
    palette: &Palette,
) -> egui::Response {
    let text_color = if enabled {
        Color32::WHITE
    } else {
        palette.text_faint
    };
    let fill = if enabled {
        palette.accent
    } else {
        palette.panel_elevated
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).color(text_color).strong())
            .fill(fill)
            .stroke(Stroke::NONE)
            .min_size(Vec2::new(width, 32.0)),
    )
}

fn snapshot_change_group(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    changes: &[SnapshotChange],
) {
    if changes.is_empty() {
        return;
    }

    ui.add_space(4.0);
    ui.label(
        RichText::new(format!("{label}: {}", changes.len()))
            .small()
            .strong()
            .color(palette.text_muted),
    );
    for change in changes {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format_signed_bytes(change.delta))
                    .small()
                    .monospace()
                    .color(if change.delta >= 0 {
                        palette.accent
                    } else {
                        palette.text_muted
                    }),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(truncate_middle(&change.path, 34))
                        .small()
                        .color(palette.text_faint),
                )
                .truncate(),
            )
            .on_hover_text(&change.path);
        });
    }
}

fn duplicate_candidate_row(ui: &mut egui::Ui, palette: &Palette, candidate: &DuplicateCandidate) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!(
            "{} · {} files · {} each",
            truncate_middle(&candidate.name, 28),
            candidate.paths.len(),
            format_bytes(candidate.size)
        ))
        .small()
        .strong()
        .color(palette.text_muted),
    );
    ui.label(
        RichText::new(format!(
            "Potential reclaim: {}",
            format_bytes(candidate.reclaimable_bytes)
        ))
        .small()
        .monospace()
        .color(palette.accent),
    );
    for path in candidate.paths.iter().take(3) {
        ui.add(
            egui::Label::new(
                RichText::new(truncate_middle(path, 38))
                    .small()
                    .color(palette.text_faint),
            )
            .truncate(),
        )
        .on_hover_text(path);
    }
}

fn cleanup_candidate_row(ui: &mut egui::Ui, palette: &Palette, candidate: &CleanupCandidate) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(format!(
            "{} · {} · {}",
            truncate_middle(&candidate.name, 28),
            describe_node_kind(candidate.kind, candidate.item_count > 1),
            pluralize(candidate.item_count as u64, "item", "items")
        ))
        .small()
        .strong()
        .color(palette.text_muted),
    );
    ui.label(
        RichText::new(format_bytes(candidate.size))
            .small()
            .monospace()
            .color(palette.accent),
    );
    ui.add(
        egui::Label::new(
            RichText::new(truncate_middle(&candidate.path.display().to_string(), 38))
                .small()
                .color(palette.text_faint),
        )
        .truncate(),
    )
    .on_hover_text(candidate.path.display().to_string());
}

fn protected_path_status(reason: ProtectedPathReason, path: &Path) -> String {
    format!(
        "Protected path blocked: {} ({})",
        path.display(),
        reason.label()
    )
}

fn insight_type_group(ui: &mut egui::Ui, palette: &Palette, summaries: &[FileTypeSummary]) {
    if summaries.is_empty() {
        return;
    }

    ui.add_space(6.0);
    ui.label(
        RichText::new("By type")
            .small()
            .strong()
            .color(palette.text_muted),
    );
    for summary in summaries.iter().take(INSIGHT_REPORT_LIMIT) {
        let ext = if summary.extension == "(none)" {
            "no ext".to_string()
        } else {
            format!(".{}", summary.extension)
        };
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format_bytes(summary.total_size))
                    .small()
                    .monospace()
                    .color(palette.accent),
            );
            ui.label(
                RichText::new(format!(
                    "{} {ext} · {}",
                    summary.category,
                    pluralize(summary.file_count as u64, "file", "files")
                ))
                .small()
                .color(palette.text_faint),
            );
        });
    }
}

fn insight_age_group(ui: &mut egui::Ui, palette: &Palette, summaries: &[AgeBucketSummary]) {
    if summaries.iter().all(|summary| summary.file_count == 0) {
        return;
    }

    ui.add_space(6.0);
    ui.label(
        RichText::new("By modified age")
            .small()
            .strong()
            .color(palette.text_muted),
    );
    for summary in summaries {
        if summary.file_count == 0 {
            continue;
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format_bytes(summary.total_size))
                    .small()
                    .monospace()
                    .color(if summary.bucket.label() == "unknown" {
                        palette.text_faint
                    } else {
                        palette.accent
                    }),
            );
            ui.label(
                RichText::new(format!(
                    "{} · {}",
                    summary.bucket.label(),
                    pluralize(summary.file_count as u64, "file", "files")
                ))
                .small()
                .color(palette.text_faint),
            );
        });
    }
}

fn insight_old_files_group(ui: &mut egui::Ui, palette: &Palette, files: &[OldLargeFile]) {
    if files.is_empty() {
        return;
    }

    ui.add_space(6.0);
    ui.label(
        RichText::new("Old large files")
            .small()
            .strong()
            .color(palette.text_muted),
    );
    for file in files.iter().take(INSIGHT_REPORT_LIMIT) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format_bytes(file.size))
                    .small()
                    .monospace()
                    .color(palette.accent),
            );
            ui.label(
                RichText::new(format!("{}d · {}", file.age_days, file.category))
                    .small()
                    .color(palette.text_muted),
            );
        });
        ui.add(
            egui::Label::new(
                RichText::new(truncate_middle(&file.path, 38))
                    .small()
                    .color(palette.text_faint),
            )
            .truncate(),
        )
        .on_hover_text(&file.path);
    }
}

fn current_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
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

fn pluralize(count: u64, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

fn format_signed_bytes(delta: i128) -> String {
    if delta >= 0 {
        format!("+{}", format_bytes(delta as u64))
    } else {
        format!("-{}", format_bytes(delta.unsigned_abs() as u64))
    }
}

fn default_export_path(format: ExportFormat) -> PathBuf {
    let timestamp = current_unix_secs();
    PathBuf::from(format!(
        "disk-map-export-{timestamp}.{}",
        format.extension()
    ))
}

fn default_report_path() -> PathBuf {
    let timestamp = current_unix_secs();
    PathBuf::from(format!("disk-map-report-{timestamp}.json"))
}

fn format_perf_stats(stats: &PerfStats) -> String {
    format!(
        "perf: messages={} batches={} entries={} nodes={} files={} dirs={} size_merges={} ancestor_delta_ms={:.2} parent_stack_hits={} parent_fallbacks={} progress_snapshots={} prefetched_files={} metadata_fallback_files={} scan_ms={:.2} metadata_ms={:.2} mtime_ms={:.2} size_ms={:.2} flush_ms={:.2} layouts={} layout_ms={:.2} search_rebuilds={} search_incremental={} db_hits={} db_misses={} db_flushes={}",
        stats.messages_sent,
        stats.batches_sent,
        stats.entries_seen,
        stats.nodes_discovered,
        stats.files_scanned,
        stats.dirs_scanned,
        stats.size_delta_merges,
        stats.ancestor_size_delta_total_ms,
        stats.parent_stack_hits,
        stats.parent_lookup_fallbacks,
        stats.progress_snapshots_sent,
        stats.prefetched_files,
        stats.metadata_fallback_files,
        stats.scan_elapsed_ms,
        stats.metadata_total_ms,
        stats.mtime_total_ms,
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
    use crate::scanner::CacheMode;
    use crate::scanner::{DiscoveredNode, ProgressSnapshot, ScanBatch};
    use crate::tree::NodeRecord;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[derive(Default)]
    struct TestStorage {
        values: BTreeMap<String, String>,
    }

    impl eframe::Storage for TestStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.values.insert(key.to_string(), value);
        }

        fn flush(&mut self) {}
    }

    fn root_started(scan_id: u64) -> ScanMessage {
        ScanMessage::Started {
            scan_id,
            path: "/root".into(),
            root_node: TreeStore::root_record("root".into()),
        }
    }

    fn root_started_at(scan_id: u64, path: &str) -> ScanMessage {
        ScanMessage::Started {
            scan_id,
            path: path.into(),
            root_node: TreeStore::root_record("root".into()),
        }
    }

    fn finished(scan_id: u64, total_bytes: u64) -> ScanMessage {
        ScanMessage::Finished {
            scan_id,
            total_bytes,
            perf_stats: PerfStats::default(),
        }
    }

    fn app_with_search_matches() -> DiskMapApp {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![
                    DiscoveredNode {
                        node_id: 1,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "match-dir".into(),
                            kind: NodeKind::Dir,
                            size: 10,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                    DiscoveredNode {
                        node_id: 2,
                        parent_id: 1,
                        node: NodeRecord {
                            name: "match-file".into(),
                            kind: NodeKind::File,
                            size: 1,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                    DiscoveredNode {
                        node_id: 3,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "match-root-file".into(),
                            kind: NodeKind::File,
                            size: 1,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                ],
                size_deltas: vec![(0, 11), (1, 1)],
                scanned_nodes: vec![1, 2, 3],
                progress: None,
            },
        });
        *app.search.input_mut() = "match".into();
        app.refresh_search_matches();
        app
    }

    fn app_for_scan(scan_id: u64) -> DiskMapApp {
        let mut app = DiskMapApp::default();
        app.set_active_scan_id_for_test(scan_id);
        app
    }

    #[test]
    fn incremental_messages_build_tree_correctly() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![DiscoveredNode {
                    node_id: 1,
                    parent_id: 0,
                    node: NodeRecord {
                        name: "child".into(),
                        kind: NodeKind::File,
                        size: 5,
                        modified_secs: None,
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
        let mut app = app_for_scan(2);
        app.apply_scan_message_for_test(root_started(1));
        assert!(app.tree.root.is_none());
    }

    #[test]
    fn cancel_like_new_scan_keeps_old_events_out() {
        let mut app = app_for_scan(2);
        app.apply_scan_message_for_test(root_started(2));
        app.set_active_scan_id_for_test(3);
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 2,
            batch: ScanBatch {
                discovered_nodes: vec![DiscoveredNode {
                    node_id: 1,
                    parent_id: 0,
                    node: NodeRecord {
                        name: "old".into(),
                        kind: NodeKind::File,
                        size: 1,
                        modified_secs: None,
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
    fn default_scan_options_keep_cache_disabled() {
        assert_eq!(
            DiskMapApp::default().scan_options().cache_mode,
            CacheMode::Disabled
        );
    }

    #[test]
    fn sqlite_cache_setting_enables_cache_mode() {
        let app = DiskMapApp {
            sqlite_cache_enabled: true,
            ..Default::default()
        };

        assert_eq!(app.scan_options().cache_mode, CacheMode::Enabled);
    }

    #[test]
    fn default_scan_options_preserve_safe_scan_defaults() {
        let options = DiskMapApp::default().scan_options();

        assert!(options.include_hidden);
        assert!(!options.follow_symlinks);
        assert!(!options.stay_on_filesystem);
    }

    #[test]
    fn realtime_watch_defaults_to_disabled() {
        let app = DiskMapApp::default();

        assert!(!app.realtime_watch_enabled);
        assert!(app.watcher.is_none());
    }

    #[test]
    fn preferences_restore_path_depth_and_theme() {
        let mut storage = TestStorage::default();
        storage
            .values
            .insert(STORAGE_PATH_INPUT.into(), "/restored".into());
        storage
            .values
            .insert(STORAGE_EXCLUDE_INPUT.into(), ".git,target".into());
        storage
            .values
            .insert(STORAGE_INCLUDE_HIDDEN.into(), "false".into());
        storage
            .values
            .insert(STORAGE_FOLLOW_SYMLINKS.into(), "true".into());
        storage
            .values
            .insert(STORAGE_STAY_ON_FILESYSTEM.into(), "true".into());
        storage
            .values
            .insert(STORAGE_REALTIME_WATCH.into(), "true".into());
        storage
            .values
            .insert(STORAGE_SQLITE_CACHE.into(), "true".into());
        storage
            .values
            .insert(STORAGE_SEARCH_FILTER.into(), "true".into());
        storage
            .values
            .insert(STORAGE_COLOR_BY_EXTENSION.into(), "true".into());
        storage.values.insert(
            STORAGE_RECENT_ROOTS.into(),
            "/recent-a\n\n/recent-b\n/recent-a".into(),
        );
        storage.values.insert(
            STORAGE_PINNED_ROOTS.into(),
            "/pinned-a\n/pinned-b\n/pinned-a".into(),
        );
        storage.values.insert(STORAGE_MAX_DEPTH.into(), "99".into());
        storage.values.insert(STORAGE_THEME.into(), "dark".into());
        let mut app = DiskMapApp::default();

        app.restore_preferences(&storage);

        assert_eq!(app.path_input, "/restored");
        assert_eq!(app.exclude_input, ".git,target");
        assert!(!app.include_hidden);
        assert!(app.follow_symlinks);
        assert!(app.stay_on_filesystem);
        assert!(app.realtime_watch_enabled);
        assert!(app.sqlite_cache_enabled);
        assert!(app.search_filter_enabled);
        assert!(app.color_by_extension);
        assert_eq!(app.recent_roots, vec!["/recent-a", "/recent-b"]);
        assert_eq!(app.pinned_roots, vec!["/pinned-a", "/pinned-b"]);
        assert_eq!(app.max_depth, 10);
        assert_eq!(app.theme_preference, Some(Theme::Dark));
    }

    #[test]
    fn preferences_save_path_depth_and_theme() {
        let mut storage = TestStorage::default();
        let app = DiskMapApp {
            path_input: "/next".into(),
            exclude_input: "node_modules;target".into(),
            include_hidden: false,
            follow_symlinks: true,
            stay_on_filesystem: true,
            realtime_watch_enabled: true,
            sqlite_cache_enabled: true,
            search_filter_enabled: true,
            color_by_extension: true,
            recent_roots: vec!["/recent".into(), "/older".into()],
            pinned_roots: vec!["/pinned".into()],
            max_depth: 4,
            theme_preference: Some(Theme::Light),
            ..Default::default()
        };

        app.save_preferences(&mut storage);

        assert_eq!(
            storage.values.get(STORAGE_PATH_INPUT).map(String::as_str),
            Some("/next")
        );
        assert_eq!(
            storage.values.get(STORAGE_MAX_DEPTH).map(String::as_str),
            Some("4")
        );
        assert_eq!(
            storage
                .values
                .get(STORAGE_EXCLUDE_INPUT)
                .map(String::as_str),
            Some("node_modules;target")
        );
        assert_eq!(
            storage
                .values
                .get(STORAGE_INCLUDE_HIDDEN)
                .map(String::as_str),
            Some("false")
        );
        assert_eq!(
            storage
                .values
                .get(STORAGE_FOLLOW_SYMLINKS)
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            storage
                .values
                .get(STORAGE_STAY_ON_FILESYSTEM)
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            storage
                .values
                .get(STORAGE_REALTIME_WATCH)
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            storage.values.get(STORAGE_SQLITE_CACHE).map(String::as_str),
            Some("true")
        );
        assert_eq!(
            storage
                .values
                .get(STORAGE_SEARCH_FILTER)
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            storage
                .values
                .get(STORAGE_COLOR_BY_EXTENSION)
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            storage.values.get(STORAGE_THEME).map(String::as_str),
            Some("light")
        );
        assert_eq!(
            storage.values.get(STORAGE_RECENT_ROOTS).map(String::as_str),
            Some("/recent\n/older")
        );
        assert_eq!(
            storage.values.get(STORAGE_PINNED_ROOTS).map(String::as_str),
            Some("/pinned")
        );
    }

    #[test]
    fn record_recent_root_deduplicates_and_caps_history() {
        let mut app = DiskMapApp::default();

        for index in 0..12 {
            app.record_recent_root(&PathBuf::from(format!("/root-{index}")));
        }
        app.record_recent_root(&PathBuf::from("/root-4"));

        assert_eq!(
            app.recent_roots.first().map(String::as_str),
            Some("/root-4")
        );
        assert_eq!(app.recent_roots.len(), MAX_RECENT_ROOTS);
        assert_eq!(
            app.recent_roots
                .iter()
                .filter(|path| path.as_str() == "/root-4")
                .count(),
            1
        );
    }

    #[test]
    fn started_scan_message_records_recent_root() {
        let mut app = app_for_scan(1);

        app.apply_scan_message_for_test(root_started(1));

        assert_eq!(app.recent_roots, vec!["/root"]);
    }

    #[test]
    fn repeated_scan_of_same_root_builds_snapshot_diff() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![DiscoveredNode {
                    node_id: 1,
                    parent_id: 0,
                    node: NodeRecord {
                        name: "file.txt".into(),
                        kind: NodeKind::File,
                        size: 4,
                        modified_secs: None,
                        scanned: true,
                        error: None,
                    },
                }],
                size_deltas: vec![(0, 4)],
                scanned_nodes: vec![1],
                progress: None,
            },
        });
        app.apply_scan_message_for_test(finished(1, 4));
        assert!(app.snapshot_diff.is_none());

        app.set_active_scan_id_for_test(2);
        app.apply_scan_message_for_test(root_started(2));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 2,
            batch: ScanBatch {
                discovered_nodes: vec![
                    DiscoveredNode {
                        node_id: 1,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "file.txt".into(),
                            kind: NodeKind::File,
                            size: 9,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                    DiscoveredNode {
                        node_id: 2,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "new.txt".into(),
                            kind: NodeKind::File,
                            size: 3,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                ],
                size_deltas: vec![(0, 12)],
                scanned_nodes: vec![1, 2],
                progress: None,
            },
        });
        app.apply_scan_message_for_test(finished(2, 12));

        let diff = app.snapshot_diff.as_ref().expect("snapshot diff");
        assert_eq!(diff.total_delta(), 8);
        assert!(diff
            .grown
            .iter()
            .any(|change| change.path == "/root/file.txt"));
        assert!(diff
            .added
            .iter()
            .any(|change| change.path == "/root/new.txt"));
    }

    #[test]
    fn snapshot_diff_is_not_built_across_different_roots() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started_at(1, "/root-a"));
        app.apply_scan_message_for_test(finished(1, 0));

        app.set_active_scan_id_for_test(2);
        app.apply_scan_message_for_test(root_started_at(2, "/root-b"));
        app.apply_scan_message_for_test(finished(2, 0));

        assert!(app.snapshot_diff.is_none());
        assert_eq!(
            app.last_snapshot
                .as_ref()
                .map(|snapshot| &snapshot.root_path),
            Some(&PathBuf::from("/root-b"))
        );
    }

    #[test]
    fn duplicate_analysis_reports_candidates_without_scan_state_changes() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![
                    DiscoveredNode {
                        node_id: 1,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "a".into(),
                            kind: NodeKind::Dir,
                            size: 0,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                    DiscoveredNode {
                        node_id: 2,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "b".into(),
                            kind: NodeKind::Dir,
                            size: 0,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                    DiscoveredNode {
                        node_id: 3,
                        parent_id: 1,
                        node: NodeRecord {
                            name: "same.bin".into(),
                            kind: NodeKind::File,
                            size: 5,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                    DiscoveredNode {
                        node_id: 4,
                        parent_id: 2,
                        node: NodeRecord {
                            name: "same.bin".into(),
                            kind: NodeKind::File,
                            size: 5,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                ],
                size_deltas: vec![(0, 10), (1, 5), (2, 5)],
                scanned_nodes: vec![1, 2, 3, 4],
                progress: None,
            },
        });
        let active_scan_id = app.scan.active_id();

        app.analyze_duplicate_candidates();

        let report = app.duplicate_report.as_ref().expect("duplicate report");
        assert_eq!(report.group_count, 1);
        assert_eq!(report.file_count, 2);
        assert_eq!(report.total_reclaimable_bytes, 5);
        assert_eq!(app.scan.active_id(), active_scan_id);
        assert!(app.status.contains("1 candidate group"));
    }

    #[test]
    fn duplicate_analysis_is_unavailable_without_focused_root() {
        let mut app = DiskMapApp::default();

        app.analyze_duplicate_candidates();

        assert!(app.duplicate_report.is_none());
        assert_eq!(
            app.status,
            "Duplicate analysis unavailable: no focused directory"
        );
    }

    #[test]
    fn insight_analysis_reports_age_and_type_without_scan_state_changes() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![
                    DiscoveredNode {
                        node_id: 1,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "photo.jpg".into(),
                            kind: NodeKind::File,
                            size: 100,
                            modified_secs: Some(current_unix_secs()),
                            scanned: true,
                            error: None,
                        },
                    },
                    DiscoveredNode {
                        node_id: 2,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "archive.zip".into(),
                            kind: NodeKind::File,
                            size: 200,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                ],
                size_deltas: vec![(0, 300)],
                scanned_nodes: vec![1, 2],
                progress: None,
            },
        });
        let active_scan_id = app.scan.active_id();
        let layout_dirty = app.layout_dirty;

        app.analyze_file_insights();

        let report = app.insight_report.as_ref().expect("insight report");
        assert_eq!(report.file_count, 2);
        assert_eq!(report.known_mtime_count, 1);
        assert!(report
            .type_summaries
            .iter()
            .any(|summary| summary.category == "Archives" && summary.extension == "zip"));
        assert_eq!(app.scan.active_id(), active_scan_id);
        assert_eq!(app.layout_dirty, layout_dirty);
        assert_eq!(app.status, "Insights analyzed 2 files");
    }

    #[test]
    fn insight_analysis_is_unavailable_without_focused_root() {
        let mut app = DiskMapApp::default();

        app.analyze_file_insights();

        assert!(app.insight_report.is_none());
        assert_eq!(app.status, "Insights unavailable: no focused directory");
    }

    #[test]
    fn focused_report_metadata_captures_reproducible_view_state() {
        let mut app = app_with_search_matches();
        app.exclude_input = ".git,target".into();
        app.include_hidden = false;
        app.follow_symlinks = true;
        app.stay_on_filesystem = true;
        app.sqlite_cache_enabled = true;
        app.realtime_watch_enabled = true;
        app.search_filter_enabled = true;
        app.color_by_extension = true;
        app.max_depth = 4;
        *app.search.input_mut() = "match".into();

        let metadata = app.focused_report_metadata(1).expect("metadata");

        assert_eq!(metadata.scan_root_path, "/root");
        assert_eq!(metadata.focused_path, "/root/match-dir");
        assert_eq!(metadata.max_depth, 4);
        assert_eq!(metadata.search_query, "match");
        assert!(metadata.search_filter_enabled);
        assert_eq!(metadata.color_mode, "extension");
        assert!(!metadata.include_hidden);
        assert!(metadata.follow_symlinks);
        assert!(metadata.stay_on_filesystem);
        assert!(metadata.sqlite_cache_enabled);
        assert!(metadata.realtime_watch_enabled);
        assert_eq!(metadata.exclude_patterns, vec![".git", "target"]);
    }

    #[test]
    fn pinned_roots_toggle_without_touching_recent_roots() {
        let mut app = DiskMapApp {
            recent_roots: vec!["/root".into()],
            ..Default::default()
        };

        app.toggle_pinned_root("/root");
        assert_eq!(app.pinned_roots, vec!["/root"]);
        assert_eq!(app.recent_roots, vec!["/root"]);
        assert_eq!(app.status, "Pinned /root");

        app.toggle_pinned_root("/root");
        assert!(app.pinned_roots.is_empty());
        assert_eq!(app.recent_roots, vec!["/root"]);
        assert_eq!(app.status, "Unpinned /root");
    }

    #[test]
    fn extension_color_is_stable_for_case_variants() {
        assert_eq!(
            color_for_extension("photo.JPG"),
            color_for_extension("x.jpg")
        );
    }

    #[test]
    fn extension_key_is_empty_without_extension() {
        assert_eq!(file_extension_key("README"), "");
    }

    #[test]
    fn trash_action_is_disabled_by_default() {
        let mut app = app_with_search_matches();
        app.tree.set_root_path("/root".into());

        app.queue_cleanup_candidate(2);

        assert_eq!(app.status, "Move to Trash is disabled");
        assert!(app.trash_confirm_target_id.is_none());
        assert!(app.cleanup_queue.is_empty());
    }

    #[test]
    fn trash_action_queues_real_path_before_confirmation() {
        let mut app = app_with_search_matches();
        app.tree.set_root_path("/root".into());
        app.destructive_actions_enabled = true;
        let active_scan_id = app.scan.active_id();

        app.queue_cleanup_candidate(2);

        assert_eq!(app.cleanup_queue.len(), 1);
        assert_eq!(
            app.cleanup_queue.candidates()[0].path,
            PathBuf::from("/root/match-dir/match-file")
        );
        assert_eq!(app.cleanup_queue.candidates()[0].item_count, 1);
        assert_eq!(app.trash_confirm_target_id, None);
        assert_eq!(app.scan.active_id(), active_scan_id);
        assert!(app.status.starts_with("Queued cleanup candidate: "));
    }

    #[test]
    fn queued_trash_requires_confirmation_with_path_size_and_item_count() {
        let mut app = app_with_search_matches();
        app.tree.set_root_path("/root".into());
        app.destructive_actions_enabled = true;

        app.queue_cleanup_candidate(2);
        app.arm_or_confirm_queued_trash(2);

        assert_eq!(app.trash_confirm_target_id, Some(2));
        assert!(app.status.contains("/root/match-dir/match-file"));
        assert!(app.status.contains("1 B"));
        assert!(app.status.contains("1 item"));
    }

    #[test]
    fn trash_action_rejects_virtual_aggregate_nodes() {
        let mut app = DiskMapApp::default();
        let root = app.tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        app.tree.set_root_path("/root".into());
        let aggregate =
            app.tree
                .add_node(Some(root), "Other Files (2)".into(), NodeKind::Aggregate, 8);
        app.destructive_actions_enabled = true;

        app.queue_cleanup_candidate(aggregate);

        assert_eq!(app.status, "Cleanup queue unavailable for virtual nodes");
        assert!(app.trash_confirm_target_id.is_none());
        assert!(app.cleanup_queue.is_empty());
    }

    #[test]
    fn cleanup_queue_blocks_protected_paths() {
        let mut app = DiskMapApp::default();
        let root = app.tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        app.tree.set_root_path("/".into());
        app.destructive_actions_enabled = true;

        app.queue_cleanup_candidate(root);

        assert!(app.status.starts_with("Protected path blocked: / "));
        assert!(app.cleanup_queue.is_empty());
    }

    #[test]
    fn scan_options_include_user_exclude_patterns() {
        let app = DiskMapApp {
            exclude_input: ".git, node_modules; target".into(),
            ..Default::default()
        };

        assert_eq!(
            app.scan_options().exclude_patterns,
            vec![".git", "node_modules", "target"]
        );
    }

    #[test]
    fn scan_options_include_safe_scan_flags() {
        let app = DiskMapApp {
            include_hidden: false,
            follow_symlinks: true,
            stay_on_filesystem: true,
            ..Default::default()
        };
        let options = app.scan_options();

        assert!(!options.include_hidden);
        assert!(options.follow_symlinks);
        assert!(options.stay_on_filesystem);
    }

    #[test]
    fn rescan_paths_target_scan_root_and_focused_directory() {
        let mut app = app_with_search_matches();
        app.tree.set_root_path("/root".into());

        assert_eq!(app.scan_root_rescan_path(), Some(PathBuf::from("/root")));
        app.enter_root(1, true);

        assert_eq!(
            app.focused_subtree_rescan_path(),
            Some(PathBuf::from("/root/match-dir"))
        );
        assert!(app.can_rescan_scan_root());
        assert!(app.can_rescan_focused_subtree());
    }

    #[test]
    fn focused_rescan_uses_parent_directory_after_file_search_match() {
        let mut app = app_with_search_matches();
        app.tree.set_root_path("/root".into());

        app.navigate_search_match(SearchDirection::Next);
        app.navigate_search_match(SearchDirection::Next);

        assert_eq!(app.navigation.selected_id(), Some(2));
        assert_eq!(app.navigation.focused_root(), Some(1));
        assert_eq!(
            app.focused_subtree_rescan_path(),
            Some(PathBuf::from("/root/match-dir"))
        );
    }

    #[test]
    fn rescan_is_unavailable_without_loaded_scan_root() {
        let mut app = DiskMapApp::default();

        assert!(!app.can_rescan_scan_root());
        assert!(!app.can_rescan_focused_subtree());
    }

    #[test]
    fn incremental_rescan_target_uses_deepest_known_directory() {
        let mut app = app_with_search_matches();
        app.tree.set_root_path("/root".into());

        let target =
            app.incremental_rescan_target(&[PathBuf::from("/root/match-dir/nested/file.txt")]);

        assert_eq!(target, Some((1, PathBuf::from("/root/match-dir"))));
    }

    #[test]
    fn incremental_rescan_target_falls_back_to_scan_root_for_unknown_child() {
        let mut app = app_with_search_matches();
        app.tree.set_root_path("/root".into());

        let target = app.incremental_rescan_target(&[PathBuf::from("/root/new-dir/file.txt")]);

        assert_eq!(target, Some((0, PathBuf::from("/root"))));
    }

    #[test]
    fn enter_selected_directory_focuses_selected_dir() {
        let mut app = app_with_search_matches();
        app.navigation.set_selected_id(Some(1));

        assert!(app.enter_selected_directory());
        assert_eq!(app.navigation.focused_root(), Some(1));
        assert_eq!(app.navigation.selected_id(), Some(1));
    }

    #[test]
    fn enter_selected_directory_ignores_files() {
        let mut app = app_with_search_matches();
        app.navigation.set_selected_id(Some(2));

        assert!(!app.enter_selected_directory());
        assert_eq!(app.navigation.focused_root(), Some(0));
    }

    #[test]
    fn depth_keyboard_helpers_clamp_and_mark_layout_dirty() {
        let mut app = DiskMapApp {
            max_depth: 1,
            layout_dirty: false,
            ..Default::default()
        };

        assert!(!app.decrease_depth());
        assert!(app.increase_depth());
        assert_eq!(app.max_depth, 2);
        assert!(app.layout_dirty);

        app.max_depth = 10;
        app.layout_dirty = false;
        assert!(!app.increase_depth());
        assert_eq!(app.max_depth, 10);
        assert!(!app.layout_dirty);
    }

    #[test]
    fn platform_errors_update_status_without_starting_scan() {
        let mut app = app_for_scan(7);

        app.apply_platform_result("Open", Err(anyhow::anyhow!("boom")));

        assert_eq!(app.scan.active_id(), 7);
        assert!(!app.scan.has_handle());
        assert_eq!(app.status, "Open failed: boom");
    }

    #[test]
    fn scan_issue_summary_updates_from_discovered_nodes() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));

        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![
                    DiscoveredNode {
                        node_id: 1,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "private".into(),
                            kind: NodeKind::Error,
                            size: 0,
                            modified_secs: None,
                            scanned: true,
                            error: Some("Operation not permitted".into()),
                        },
                    },
                    DiscoveredNode {
                        node_id: 2,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "linked".into(),
                            kind: NodeKind::Symlink,
                            size: 0,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                ],
                size_deltas: vec![],
                scanned_nodes: vec![1, 2],
                progress: None,
            },
        });

        let summary = app.scan.issue_summary();
        assert_eq!(summary.error_entries, 1);
        assert_eq!(summary.permission_errors, 1);
        assert_eq!(summary.skipped_paths, 1);
        assert_eq!(summary.symlinks, 1);
        assert_eq!(app.finished_status(0), "Finished: 0 B · 1 issue");
    }

    #[test]
    fn no_root_state_message_reflects_error_and_cancelled_states() {
        let mut app = DiskMapApp {
            status: "Error: Path does not exist: /missing".into(),
            ..Default::default()
        };

        let error_message = app.no_root_state_message();
        assert_eq!(error_message.title, "Unable to scan path");
        assert_eq!(error_message.detail, "Path does not exist: /missing");

        app.status = "Scan cancelled".into();
        let cancelled_message = app.no_root_state_message();
        assert_eq!(cancelled_message.title, "Scan cancelled");
    }

    #[test]
    fn empty_root_state_message_reports_empty_finished_folder() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));

        let message = app.empty_root_state_message(0).expect("empty root state");

        assert_eq!(message.title, "Empty folder");
        assert!(message.detail.contains("root"));
    }

    #[test]
    fn return_to_scan_root_pushes_previous_focus_to_back_history() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![DiscoveredNode {
                    node_id: 1,
                    parent_id: 0,
                    node: NodeRecord {
                        name: "child-dir".into(),
                        kind: NodeKind::Dir,
                        size: 10,
                        modified_secs: None,
                        scanned: true,
                        error: None,
                    },
                }],
                size_deltas: vec![(0, 10)],
                scanned_nodes: vec![1],
                progress: None,
            },
        });
        app.enter_root(1, false);
        app.layout_dirty = false;

        app.return_to_scan_root();

        assert_eq!(app.navigation.focused_root(), Some(0));
        assert_eq!(app.navigation.selected_id(), Some(0));
        assert_eq!(app.navigation.back_history(), &[1]);
        assert!(app.navigation.forward_history().is_empty());
        assert!(app.layout_dirty);
    }

    #[test]
    fn return_to_scan_root_is_noop_when_already_at_scan_root() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));
        app.navigation.push_back_for_test(42);

        app.return_to_scan_root();

        assert_eq!(app.navigation.focused_root(), Some(0));
        assert_eq!(app.navigation.back_history(), &[42]);
        assert!(!app.navigation.can_return_to_scan_root(&app.tree));
    }

    #[test]
    fn apply_progress_keeps_current_scan_path() {
        let mut app = DiskMapApp::default();

        app.apply_progress(ProgressSnapshot {
            files_scanned: 3,
            dirs_scanned: 2,
            bytes_seen: 128,
            current_path: "/root/current/file.txt".into(),
        });

        let progress = app.scan.progress().expect("progress summary");
        assert_eq!(progress.files_scanned, 3);
        assert_eq!(progress.dirs_scanned, 2);
        assert_eq!(progress.bytes_seen, 128);
        assert_eq!(
            progress.current_path,
            PathBuf::from("/root/current/file.txt")
        );
        assert_eq!(app.status, "Scanning...");
    }

    #[test]
    fn search_next_cycles_through_ordered_matches() {
        let mut app = app_with_search_matches();

        app.navigate_search_match(SearchDirection::Next);
        assert_eq!(app.search.active_match(), Some(0));
        assert_eq!(app.navigation.focused_root(), Some(1));
        assert_eq!(app.navigation.selected_id(), Some(1));

        app.navigate_search_match(SearchDirection::Next);
        assert_eq!(app.search.active_match(), Some(1));
        assert_eq!(app.navigation.focused_root(), Some(1));
        assert_eq!(app.navigation.selected_id(), Some(2));

        app.navigate_search_match(SearchDirection::Next);
        assert_eq!(app.search.active_match(), Some(2));
        assert_eq!(app.navigation.focused_root(), Some(0));
        assert_eq!(app.navigation.selected_id(), Some(3));

        app.navigate_search_match(SearchDirection::Next);
        assert_eq!(app.search.active_match(), Some(0));
        assert_eq!(app.navigation.focused_root(), Some(1));
        assert_eq!(app.navigation.selected_id(), Some(1));
    }

    #[test]
    fn search_previous_cycles_from_no_active_match_to_last_match() {
        let mut app = app_with_search_matches();

        app.navigate_search_match(SearchDirection::Previous);
        assert_eq!(app.search.active_match(), Some(2));
        assert_eq!(app.navigation.focused_root(), Some(0));
        assert_eq!(app.navigation.selected_id(), Some(3));

        app.navigate_search_match(SearchDirection::Previous);
        assert_eq!(app.search.active_match(), Some(1));
        assert_eq!(app.navigation.focused_root(), Some(1));
        assert_eq!(app.navigation.selected_id(), Some(2));
    }

    #[test]
    fn search_jump_preserves_results() {
        let mut app = app_with_search_matches();
        app.navigate_search_match(SearchDirection::Next);

        assert_eq!(app.search.state().matches(), &[1, 2, 3]);
        assert_eq!(app.navigation.focused_root(), Some(1));
    }

    #[test]
    fn manual_navigation_rebuilds_search_scope() {
        let mut app = app_with_search_matches();

        app.enter_root(1, false);

        assert_eq!(app.search.state().matches(), &[1, 2]);
        assert_eq!(app.search.active_match(), None);
    }

    #[test]
    fn clear_search_clears_active_match_cursor() {
        let mut app = app_with_search_matches();
        app.navigate_search_match(SearchDirection::Next);

        app.clear_search();

        assert!(app.search.input().is_empty());
        assert!(app.search.state().matches().is_empty());
        assert_eq!(app.search.active_match(), None);
    }

    #[test]
    fn search_rebuild_marks_matches_in_current_root() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![DiscoveredNode {
                    node_id: 1,
                    parent_id: 0,
                    node: NodeRecord {
                        name: "match-me".into(),
                        kind: NodeKind::File,
                        size: 1,
                        modified_secs: None,
                        scanned: true,
                        error: None,
                    },
                }],
                size_deltas: vec![],
                scanned_nodes: vec![],
                progress: None,
            },
        });
        *app.search.input_mut() = "match".into();
        app.refresh_search_matches();

        assert_eq!(app.search.state().match_count(), 1);
        assert!(app.search.state().is_match(1));
    }

    #[test]
    fn truncate_middle_should_keep_prefix_and_suffix() {
        let truncated = truncate_middle("abcdefghijklmnopqrstuvwxyz", 10);
        assert_eq!(truncated, "abcd…vwxyz");
    }
}
