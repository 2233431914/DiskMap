use crate::format::format_bytes;
use crate::platform::{open_path, reveal_in_finder};
use crate::scanner::{
    parse_exclude_patterns, PerfStats, ProgressSnapshot, ScanBatch, ScanMessage, ScanOptions,
};
use crate::tree::{NodeId, NodeKind, TreeStore};
use crate::treemap::{
    layout_treemap, Camera, LayoutScratch, TreemapLayoutParams, VisualKind, VisualNode,
};

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
use std::time::{Duration, Instant};

const LAYOUT_REFRESH_INTERVAL: Duration = Duration::from_millis(33);
const CONTEXT_MENU_MIN_WIDTH: f32 = 240.0;
const CONTEXT_MENU_MAX_TITLE_CHARS: usize = 36;
const STORAGE_PATH_INPUT: &str = "disk_map.path_input";
const STORAGE_EXCLUDE_INPUT: &str = "disk_map.exclude_input";
const STORAGE_INCLUDE_HIDDEN: &str = "disk_map.include_hidden";
const STORAGE_FOLLOW_SYMLINKS: &str = "disk_map.follow_symlinks";
const STORAGE_STAY_ON_FILESYSTEM: &str = "disk_map.stay_on_filesystem";
const STORAGE_MAX_DEPTH: &str = "disk_map.max_depth";
const STORAGE_THEME: &str = "disk_map.theme";

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
    initial_scan_pending: bool,
    tx: Sender<ScanMessage>,
    rx: Receiver<ScanMessage>,
    tree: TreeStore,
    navigation: NavigationState,
    search: SearchController,
    scan: ScanSession,
    hovered_id: Option<NodeId>,
    context_menu_target_id: Option<NodeId>,
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
        Self {
            path_input: dirs_home_fallback(),
            exclude_input: String::new(),
            include_hidden: ScanOptions::default().include_hidden,
            follow_symlinks: ScanOptions::default().follow_symlinks,
            stay_on_filesystem: ScanOptions::default().stay_on_filesystem,
            initial_scan_pending: true,
            tx,
            rx,
            tree: TreeStore::new(),
            navigation: NavigationState::default(),
            search: SearchController::default(),
            scan: ScanSession::default(),
            hovered_id: None,
            context_menu_target_id: None,
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
        storage.set_string(STORAGE_MAX_DEPTH, self.max_depth.to_string());
        if let Some(theme) = self.theme_preference {
            storage.set_string(STORAGE_THEME, theme_preference_name(theme).to_string());
        }
    }

    fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            exclude_patterns: parse_exclude_patterns(&self.exclude_input),
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
                self.layout_dirty = true;
                self.last_layout_refresh = Instant::now()
                    .checked_sub(LAYOUT_REFRESH_INTERVAL)
                    .unwrap_or_else(Instant::now);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(theme) = theme_cycle_button(ui) {
                    self.theme_preference = Some(theme);
                }
            });
        });
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
        let fill = fill_color_for_visual(visual, is_hovered, is_selected, palette);
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

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if !ctx.egui_wants_keyboard_input()
            && ctx.input(|input| input.key_pressed(egui::Key::Enter))
        {
            if let Some(selected_id) = self.navigation.selected_id() {
                if !self.tree.node(selected_id).children.is_empty() {
                    self.enter_root(selected_id, true);
                }
            }
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

    fn start_scan(&mut self) {
        self.scan.start(
            std::path::PathBuf::from(self.path_input.trim()),
            self.scan_options(),
            self.tx.clone(),
        );

        self.tree.clear();
        self.navigation.clear_for_new_scan();
        self.hovered_id = None;
        self.context_menu_target_id = None;
        self.hovered_visual_kind = None;
        self.search.clear(0);
        self.cached_visuals.clear();
        self.reset_camera();
        self.layout_dirty = true;
        self.status = format!("Scanning {}", self.path_input.trim());
        self.pending_repaint = true;
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
            ScanMessage::Started { root_node, .. } => {
                self.tree.clear();
                self.tree.push_node(None, root_node);
                self.navigation.set_scan_root(self.tree.root);
                self.navigation.rebuild_breadcrumb_cache(&self.tree);
            }
            ScanMessage::Batch { batch, .. } => self.apply_scan_batch(batch),
            ScanMessage::Finished { .. }
            | ScanMessage::Cancelled { .. }
            | ScanMessage::Error { .. } => {}
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
) -> Color32 {
    let mut color = if visual.is_dir {
        palette.dir_palette[visual.depth % palette.dir_palette.len()]
    } else {
        palette.file_neutral
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
    fn default_scan_options_preserve_safe_scan_defaults() {
        let options = DiskMapApp::default().scan_options();

        assert!(options.include_hidden);
        assert!(!options.follow_symlinks);
        assert!(!options.stay_on_filesystem);
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
        storage.values.insert(STORAGE_MAX_DEPTH.into(), "99".into());
        storage.values.insert(STORAGE_THEME.into(), "dark".into());
        let mut app = DiskMapApp::default();

        app.restore_preferences(&storage);

        assert_eq!(app.path_input, "/restored");
        assert_eq!(app.exclude_input, ".git,target");
        assert!(!app.include_hidden);
        assert!(app.follow_symlinks);
        assert!(app.stay_on_filesystem);
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
            storage.values.get(STORAGE_THEME).map(String::as_str),
            Some("light")
        );
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
