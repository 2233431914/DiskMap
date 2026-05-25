use crate::format::format_bytes;
use crate::platform::{move_to_trash, open_path, reveal_in_finder};
use crate::scanner::{
    self, CacheMode, PerfStats, ProgressSnapshot, ScanBatch, ScanHandle, ScanMessage, ScanOptions,
};
use crate::tree::{NodeId, NodeKind, TreeStore};
use crate::treemap::{layout_treemap, Camera, LayoutScratch, SearchState, VisualKind, VisualNode};

use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui;
use egui::{
    Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Shadow, Stroke, Theme,
    Vec2,
};
use std::time::{Duration, Instant};

const SEARCH_REFRESH_INTERVAL: Duration = Duration::from_millis(150);
const LAYOUT_REFRESH_INTERVAL: Duration = Duration::from_millis(33);
const CONTEXT_MENU_MIN_WIDTH: f32 = 240.0;
const CONTEXT_MENU_MAX_TITLE_CHARS: usize = 36;

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
    search_input: String,
    initial_scan_pending: bool,
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
    layout_scratch: LayoutScratch,
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
}

impl Default for DiskMapApp {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            path_input: dirs_home_fallback(),
            search_input: String::new(),
            initial_scan_pending: true,
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
            layout_scratch: LayoutScratch::default(),
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
}

impl DiskMapApp {
    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if icon_button(ui, !self.back_history.is_empty(), ToolbarIcon::ArrowLeft)
                .on_hover_text("Back")
                .clicked()
            {
                self.navigate_back();
            }

            if icon_button(ui, !self.forward_history.is_empty(), ToolbarIcon::ArrowRight)
                .on_hover_text("Forward")
                .clicked()
            {
                self.navigate_forward();
            }

            if icon_button(ui, self.parent_of_focused_root().is_some(), ToolbarIcon::Up)
                .on_hover_text("Up to parent directory")
                .clicked()
            {
                if let Some(parent) = self.parent_of_focused_root() {
                    self.enter_root(parent, true);
                }
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

            let scan_label = if self.scanning { "Cancel" } else { "Scan" };
            if ui
                .add_sized([72.0, 28.0], egui::Button::new(scan_label))
                .clicked()
            {
                if self.scanning {
                    self.cancel_scan();
                } else {
                    self.start_scan();
                }
            }

            ui.add_space(6.0);

            let search_response = ui.add_sized(
                [180.0, 28.0],
                egui::TextEdit::singleline(&mut self.search_input)
                    .hint_text("Search files & folders"),
            );
            if search_response.changed() {
                self.mark_search_dirty();
            }
            if !self.search_input.is_empty()
                && icon_button(ui, true, ToolbarIcon::Close)
                    .on_hover_text("Clear search")
                    .clicked()
            {
                self.search_input.clear();
                self.search_state.clear(self.tree.len());
                self.search_dirty = false;
                self.layout_dirty = true;
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
                theme_cycle_button(ui);
            });
        });
    }

    fn show_details_panel(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui.ctx());
        ui.add_space(4.0);
        ui.label(RichText::new("DETAILS").size(11.0).strong().color(p.text_muted));
        ui.add_space(2.0);
        section_divider(ui, p);
        ui.add_space(8.0);

        let subject_id = self.selected_id.or(self.focused_root);
        let Some(node_id) = subject_id else {
            ui.label(
                RichText::new("Run a scan to populate the treemap.")
                    .color(p.text_muted),
            );
            self.show_progress_section(ui, p);
            self.show_search_section(ui, p);
            return;
        };

        let node_path = self.tree.node_real_path(node_id);
        let (
            node_name,
            node_size,
            node_kind,
            child_count,
            node_scanned,
            node_error,
            node_parent,
        ) = {
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
        let matched = self.search_state.is_match(node_id);
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
                if !self.search_input.trim().is_empty() {
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
                .fill(Color32::from_rgba_unmultiplied(p.danger.r(), p.danger.g(), p.danger.b(), 28))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.label(RichText::new(format!("Error: {err}")).color(p.danger));
                });
        }

        ui.add_space(12.0);
        ui.label(RichText::new("PRIMARY").size(10.0).strong().color(p.text_faint));
        ui.add_space(4.0);
        let path_available = node_path.is_some();
        ui.columns(2, |cols| {
            let w0 = cols[0].available_width();
            if accent_button(&mut cols[0], "Open", path_available, w0, p).clicked() {
                if let Some(path) = &node_path {
                    open_path(path);
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
                    reveal_in_finder(path);
                }
            }
        });

        ui.add_space(10.0);
        ui.label(RichText::new("UTILITY").size(10.0).strong().color(p.text_faint));
        ui.add_space(4.0);
        ui.columns(2, |cols| {
            let w0 = cols[0].available_width();
            if cols[0]
                .add_enabled(
                    path_available,
                    egui::Button::new("Copy Path").min_size(Vec2::new(w0, 28.0)),
                )
                .clicked()
            {
                if let Some(path) = &node_path {
                    cols[0].ctx().copy_text(path.display().to_string());
                }
            }
            let w1 = cols[1].available_width();
            if cols[1]
                .add_enabled(
                    path_available,
                    egui::Button::new(RichText::new("Move to Trash").color(p.danger))
                        .min_size(Vec2::new(w1, 28.0)),
                )
                .clicked()
            {
                if let Some(path) = &node_path {
                    let _ = move_to_trash(path);
                }
            }
        });

        if let Some(parent) = node_parent {
            ui.add_space(10.0);
            ui.label(RichText::new("PARENT").size(10.0).strong().color(p.text_faint));
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
                self.selected_id = Some(parent);
            }
        }

        self.show_progress_section(ui, p);
        self.show_search_section(ui, p);
    }

    fn show_progress_section(&self, ui: &mut egui::Ui, p: &Palette) {
        let Some(progress) = &self.progress_summary else {
            return;
        };
        ui.add_space(12.0);
        ui.label(RichText::new("SCAN").size(10.0).strong().color(p.text_faint));
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
    }

    fn show_search_section(&self, ui: &mut egui::Ui, p: &Palette) {
        let query = self.search_input.trim();
        if query.is_empty() {
            return;
        }
        ui.add_space(12.0);
        ui.label(RichText::new("SEARCH").size(10.0).strong().color(p.text_faint));
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!("Query: {query}"))
                .small()
                .color(p.text_muted),
        );
        ui.label(
            RichText::new(format!(
                "{} matches · {}",
                self.search_state.match_count(),
                if self.search_dirty { "Updating…" } else { "Ready" }
            ))
            .small()
            .color(if self.search_dirty { p.accent } else { p.text_muted }),
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
            } else if self.scanning {
                p.accent
            } else if self.status.starts_with("Cancel") {
                p.text_faint
            } else {
                Color32::from_rgb(0x4A, 0xC4, 0x7A)
            };
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, dot_color);
            ui.label(
                RichText::new(&self.status)
                    .size(11.5)
                    .color(p.text_muted),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                if !self.breadcrumb_cache.is_empty() {
                    let crumb = self.breadcrumb_cache.replace(" / ", " › ");
                    let display = truncate_middle(&crumb, 60);
                    ui.label(
                        RichText::new(display)
                            .size(11.5)
                            .monospace()
                            .color(p.text_faint),
                    );
                }

                if let Some(progress) = &self.progress_summary {
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
                }
            });
        });
    }

    fn show_treemap(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui.ctx());
        let available = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(available, Sense::click_and_drag());
        let painter = ui.painter_at(available);
        painter.rect_filled(available, 0.0, p.surface);

        let Some(root_id) = self.focused_root else {
            painter.text(
                available.center(),
                egui::Align2::CENTER_CENTER,
                "Input path and click Scan",
                egui::TextStyle::Heading.resolve(ui.style()),
                p.text_faint,
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
            layout_treemap(
                &mut self.tree,
                root_id,
                available,
                self.camera,
                self.max_depth,
                &self.search_state,
                &mut self.cached_visuals,
                &mut self.layout_scratch,
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
                let p = palette(ui.ctx());
                let node_path = self.tree.node_real_path(node_id);
                let node = self.tree.node(node_id);
                ui.set_min_width(CONTEXT_MENU_MIN_WIDTH);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(truncate_middle(&node.name, CONTEXT_MENU_MAX_TITLE_CHARS))
                            .strong()
                            .color(p.text),
                    );
                    ui.label(
                        RichText::new(format_bytes(node.size))
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
                            open_path(path);
                        }
                        ui.close();
                    }
                    if ui
                        .add_enabled(node_path.is_some(), egui::Button::new("Reveal in Finder"))
                        .clicked()
                    {
                        if let Some(path) = &node_path {
                            reveal_in_finder(path);
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
                    if ui
                        .add_enabled(
                            node_path.is_some(),
                            egui::Button::new(RichText::new("Move to Trash").color(p.danger)),
                        )
                        .clicked()
                    {
                        if let Some(path) = &node_path {
                            let _ = move_to_trash(path);
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
                            let dot_color = if node.scanned {
                                p.text_faint
                            } else {
                                p.accent
                            };
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::splat(8.0),
                                Sense::hover(),
                            );
                            ui.painter().circle_filled(rect.center(), 3.0, dot_color);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&node.name).strong().color(p.text),
                                )
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
                                egui::Label::new(
                                    RichText::new(error).small().color(p.danger),
                                )
                                .wrap(),
                            );
                        }
                    });
            });
    }

    fn paint_visual(&self, ui: &egui::Ui, painter: &egui::Painter, visual: &VisualNode) {
        let palette = palette(ui.ctx());
        let is_hovered = matches!(visual.kind, VisualKind::Node(node_id) if self.hovered_id == Some(node_id));
        let is_selected = matches!(visual.kind, VisualKind::Node(node_id) if self.selected_id == Some(node_id));
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
                    self.tree.set_root_path(path);
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
                    self.scan_handle = None;
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
                    self.scan_handle = None;
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
                    self.scan_handle = None;
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
        });
        self.status = "Scanning...".to_string();
    }

    fn merge_scan_perf_stats(&mut self, perf_stats: PerfStats) {
        self.perf_stats.messages_sent = perf_stats.messages_sent;
        self.perf_stats.batches_sent = perf_stats.batches_sent;
        self.perf_stats.entries_seen = perf_stats.entries_seen;
        self.perf_stats.nodes_discovered = perf_stats.nodes_discovered;
        self.perf_stats.files_scanned = perf_stats.files_scanned;
        self.perf_stats.dirs_scanned = perf_stats.dirs_scanned;
        self.perf_stats.size_delta_merges = perf_stats.size_delta_merges;
        self.perf_stats.ancestor_size_delta_total_ms = perf_stats.ancestor_size_delta_total_ms;
        self.perf_stats.parent_stack_hits = perf_stats.parent_stack_hits;
        self.perf_stats.parent_lookup_fallbacks = perf_stats.parent_lookup_fallbacks;
        self.perf_stats.progress_snapshots_sent = perf_stats.progress_snapshots_sent;
        self.perf_stats.prefetched_files = perf_stats.prefetched_files;
        self.perf_stats.metadata_fallback_files = perf_stats.metadata_fallback_files;
        self.perf_stats.metadata_total_ms = perf_stats.metadata_total_ms;
        self.perf_stats.mtime_total_ms = perf_stats.mtime_total_ms;
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

    fn cancel_scan(&mut self) {
        if let Some(handle) = &self.scan_handle {
            handle.cancel();
            self.status = "Cancelling scan...".to_string();
            self.pending_repaint = true;
        }
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
    ui.painter()
        .rect(rect, visuals.corner_radius, fill, stroke, egui::StrokeKind::Inside);

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
                [Pos2::new(c.x - 4.5, c.y - 4.5), Pos2::new(c.x + 4.5, c.y + 4.5)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x - 4.5, c.y + 4.5), Pos2::new(c.x + 4.5, c.y - 4.5)],
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

fn theme_cycle_button(ui: &mut egui::Ui) -> egui::Response {
    let current = ui.ctx().theme();
    let (icon, tooltip, next_pref, next_sys) = match current {
        Theme::Dark => (
            ToolbarIcon::ThemeLight,
            "Switch to light mode",
            egui::ThemePreference::Light,
            egui::SystemTheme::Light,
        ),
        Theme::Light => (
            ToolbarIcon::ThemeDark,
            "Switch to dark mode",
            egui::ThemePreference::Dark,
            egui::SystemTheme::Dark,
        ),
    };
    let response = icon_button(ui, true, icon).on_hover_text(tooltip);
    if response.clicked() {
        ui.ctx().set_theme(next_pref);
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::SetTheme(next_sys));
    }
    response
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
    use crate::scanner::{DiscoveredNode, ScanBatch};
    use crate::tree::NodeRecord;

    fn root_started(scan_id: u64) -> ScanMessage {
        ScanMessage::Started {
            scan_id,
            path: "/root".into(),
            root_node: TreeStore::root_record("root".into()),
        }
    }

    #[test]
    fn incremental_messages_build_tree_correctly() {
        let mut app = DiskMapApp {
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
        let mut app = DiskMapApp {
            active_scan_id: 2,
            ..Default::default()
        };
        app.apply_scan_message_for_test(root_started(1));
        assert!(app.tree.root.is_none());
    }

    #[test]
    fn cancel_like_new_scan_keeps_old_events_out() {
        let mut app = DiskMapApp {
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
        let mut app = DiskMapApp {
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
