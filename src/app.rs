use crate::cleanup::CleanupQueue;
use crate::duplicates::DuplicateReport;
#[cfg(test)]
use crate::export::ExportFormat;
use crate::format::format_bytes;
use crate::i18n::{Locale, TextKey};
use crate::insights::InsightReport;
#[cfg(test)]
use crate::scanner::ScanMessage;
use crate::scanner::{parse_exclude_patterns, CacheMode, PerfStats, ScanBatch, ScanOptions};
#[cfg(test)]
use crate::snapshot::{capture_snapshot, compare_snapshots, ScanSnapshot};
use crate::storage::{app_data_dir, LocalState, Preferences, SafeStorage};
#[cfg(test)]
use crate::tree::node_id_from_index;
use crate::tree::{NodeId, NodeKind, TreeStore};
use crate::treemap::{VisualKind, VisualNode};

#[cfg(test)]
mod analysis_actions;
mod cleanup_actions;
#[cfg(test)]
mod export_actions;
mod navigation;
mod panels;
#[cfg(test)]
mod profile_actions;
#[cfg(test)]
mod rule_actions;
mod scan_session;
mod search_nav;
mod status;
mod treemap_state;

use navigation::{NavigationOutcome, NavigationState};
use scan_session::{ScanPhase, ScanSession, ScanSessionEvent, WatchAction};
use search_nav::{SearchController, SearchDirection, SEARCH_REFRESH_INTERVAL};
use status::{AppStatus, StatusLevel, StatusSource};
use treemap_state::TreemapViewState;

use eframe::egui;
use egui::{
    Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Shadow, Stroke, Theme, Vec2,
};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) const LAYOUT_REFRESH_INTERVAL: Duration = Duration::from_millis(33);
pub(super) const CONTEXT_MENU_MIN_WIDTH: f32 = 240.0;
pub(super) const CONTEXT_MENU_MAX_TITLE_CHARS: usize = 36;
const HOVER_TOOLTIP_MAX_WIDTH: f32 = 720.0;
const HOVER_TOOLTIP_MIN_WIDTH: f32 = 260.0;
const HOVER_TOOLTIP_SCREEN_MARGIN: f32 = 48.0;
const STORAGE_PATH_INPUT: &str = "disk_map.path_input";
const STORAGE_EXCLUDE_INPUT: &str = "disk_map.exclude_input";
const STORAGE_PROTECTED_PATHS: &str = "disk_map.protected_paths";
const STORAGE_INCLUDE_HIDDEN: &str = "disk_map.include_hidden";
const STORAGE_FOLLOW_SYMLINKS: &str = "disk_map.follow_symlinks";
const STORAGE_STAY_ON_FILESYSTEM: &str = "disk_map.stay_on_filesystem";
const STORAGE_SQLITE_CACHE: &str = "disk_map.sqlite_cache";
const STORAGE_SEARCH_FILTER: &str = "disk_map.search_filter";
const STORAGE_COLOR_BY_EXTENSION: &str = "disk_map.color_by_extension";
const STORAGE_REALTIME_WATCH: &str = "disk_map.realtime_watch";
const STORAGE_RECENT_ROOTS: &str = "disk_map.recent_roots";
const STORAGE_PINNED_ROOTS: &str = "disk_map.pinned_roots";
const STORAGE_MAX_DEPTH: &str = "disk_map.max_depth";
const STORAGE_THEME: &str = "disk_map.theme";
const STORAGE_LOCALE: &str = "disk_map.locale";
const MAX_RECENT_ROOTS: usize = 10;
const MAX_PINNED_ROOTS: usize = 12;
#[cfg(test)]
const SNAPSHOT_DIFF_LIMIT: usize = 5;

#[derive(Clone, Copy)]
pub(super) struct Palette {
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

pub(super) fn palette(ctx: &egui::Context) -> &'static Palette {
    palette_for(ctx.theme())
}

pub(super) fn pick_label_color(bg: Color32) -> Color32 {
    let luma = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if luma < 140.0 {
        Color32::from_rgb(245, 245, 250)
    } else {
        Color32::from_rgb(20, 20, 24)
    }
}

pub fn configure_theme(ctx: &egui::Context) {
    configure_fonts(ctx, Locale::default());
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

fn configure_fonts(ctx: &egui::Context, _locale: Locale) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    if let Some(font_path) = first_readable_font_path(cjk_font_candidates().iter().copied()) {
        if let Ok(font_bytes) = std::fs::read(font_path) {
            install_cjk_font_data(&mut fonts, font_bytes);
        }
    }
    ctx.set_fonts(fonts);
}

fn install_cjk_font_data(fonts: &mut egui::FontDefinitions, font_bytes: Vec<u8>) {
    const CJK_FONT_NAME: &str = "disk-map-cjk";
    fonts.font_data.insert(
        CJK_FONT_NAME.to_string(),
        Arc::new(egui::FontData::from_owned(font_bytes)),
    );

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let entries = fonts.families.entry(family).or_default();
        if !entries.iter().any(|entry| entry == CJK_FONT_NAME) {
            entries.push(CJK_FONT_NAME.to_string());
        }
    }
}

fn first_readable_font_path<'a>(candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .into_iter()
        .find(|path| std::fs::File::open(path).is_ok())
}

fn cjk_font_candidates() -> &'static [&'static str] {
    &[
        // Linux: Noto / Source Han are common on current desktop distributions.
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.otf",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.otf",
        "/usr/share/fonts/opentype/noto/NotoSansSC-Regular.otf",
        "/usr/share/fonts/truetype/noto/NotoSansSC-Regular.otf",
        "/usr/share/fonts/opentype/source-han-sans/SourceHanSansSC-Regular.otf",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        "/usr/share/fonts/truetype/arphic/uming.ttc",
        // macOS: installed system CJK fonts.
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
        "/System/Library/Fonts/Supplemental/Songti.ttc",
        "/System/Library/Fonts/Supplemental/Heiti.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/Library/Fonts/Arial Unicode.ttf",
    ]
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
    pub(super) protected_paths_input: String,
    include_hidden: bool,
    follow_symlinks: bool,
    stay_on_filesystem: bool,
    sqlite_cache_enabled: bool,
    initial_scan_pending: bool,
    pub(super) tree: TreeStore,
    pub(super) navigation: NavigationState,
    pub(super) search: SearchController,
    pub(super) search_filter_enabled: bool,
    pub(super) color_by_extension: bool,
    pub(super) recent_roots: Vec<String>,
    pub(super) pinned_roots: Vec<String>,
    #[cfg(test)]
    last_snapshot: Option<ScanSnapshot>,
    #[cfg(test)]
    pub(super) snapshot_diff: Option<crate::snapshot::SnapshotDiff>,
    pub(super) duplicate_report: Option<DuplicateReport>,
    pub(super) insight_report: Option<InsightReport>,
    pub(super) scan: ScanSession,
    pub(super) hovered_id: Option<NodeId>,
    pub(super) context_menu_target_id: Option<NodeId>,
    pub(super) trash_confirm_target_id: Option<NodeId>,
    pub(super) trash_confirm_path: Option<PathBuf>,
    pub(super) cleanup_queue: CleanupQueue,
    pub(super) hovered_visual_kind: Option<VisualKind>,
    pub(super) max_depth: usize,
    theme_preference: Option<Theme>,
    pub(super) locale: Locale,
    pub(super) locale_follow_system: bool,
    pub(in crate::app) status: AppStatus,
    pub(in crate::app) treemap: TreemapViewState,
    pub(super) pending_repaint: bool,
    safe_storage: SafeStorage,
    /// Bounded ring of recent error/status messages for diagnostics export.
    /// Capped at 64 entries; oldest dropped on overflow.
    recent_errors: VecDeque<String>,
    /// Read-only rule engine state. Initialized from `default_ruleset` on
    /// first launch; production UI actions are currently deferred.
    pub(super) rules: crate::rules::RuleSet,
    /// Most recent result from the test-only `evaluate_current_rules` helper.
    #[cfg(test)]
    pub(super) last_rule_hits: Option<Vec<crate::rules::RuleHit>>,
    /// Sticky text field for the rules import path. Self-clears after
    /// a successful import. Avoids spawning a native file dialog (we
    /// don't depend on a GUI toolkit for picking files).
    #[cfg(test)]
    pub(super) rules_import_path: String,
    /// Pending rules import preview. Import only replaces the live
    /// ruleset after the user confirms this preview.
    #[cfg(test)]
    pub(super) pending_rules_import: Option<crate::rules::RuleImportPreview>,
    /// Legacy per-root scan option profiles kept for local-state compatibility.
    /// The simplified GUI no longer auto-applies hidden profile state.
    pub(super) profiles: crate::profiles::ProfileStore,
    /// Settings window open state. The window owns the scan root path
    /// and scan option controls.
    pub(super) settings_open: bool,
    /// Legacy per-root saved view state retained for local-state compatibility.
    pub(super) views: crate::views::ViewStore,
    /// Legacy report discriminator retained for saved-view compatibility.
    #[cfg(test)]
    pub(super) last_report_mode: String,
    /// Saved filter presets (named bundles of search query +
    /// filter_enabled toggle).
    pub(super) filter_presets: crate::views::FilterStore,
    /// Sticky text field for the new-preset name input. Self-clears
    /// after a successful add.
    #[cfg(test)]
    pub(super) filter_preset_name: String,
}

impl Default for DiskMapApp {
    fn default() -> Self {
        Self {
            path_input: dirs_home_fallback(),
            exclude_input: String::new(),
            protected_paths_input: String::new(),
            include_hidden: ScanOptions::default().include_hidden,
            follow_symlinks: ScanOptions::default().follow_symlinks,
            stay_on_filesystem: ScanOptions::default().stay_on_filesystem,
            sqlite_cache_enabled: false,
            initial_scan_pending: true,
            tree: TreeStore::new(),
            navigation: NavigationState::default(),
            search: SearchController::default(),
            search_filter_enabled: false,
            color_by_extension: false,
            recent_roots: Vec::new(),
            pinned_roots: Vec::new(),
            #[cfg(test)]
            last_snapshot: None,
            #[cfg(test)]
            snapshot_diff: None,
            duplicate_report: None,
            insight_report: None,
            scan: ScanSession::default(),
            hovered_id: None,
            context_menu_target_id: None,
            trash_confirm_target_id: None,
            trash_confirm_path: None,
            cleanup_queue: CleanupQueue::default(),
            hovered_visual_kind: None,
            max_depth: 1,
            theme_preference: None,
            locale: Locale::default(),
            locale_follow_system: true,
            status: AppStatus::default(),
            treemap: TreemapViewState::default(),
            pending_repaint: false,
            safe_storage: SafeStorage::new(&app_data_dir("disk-map")),
            recent_errors: VecDeque::new(),
            rules: crate::rules::default_ruleset(),
            #[cfg(test)]
            last_rule_hits: None,
            #[cfg(test)]
            rules_import_path: String::new(),
            #[cfg(test)]
            pending_rules_import: None,
            profiles: crate::profiles::ProfileStore::new(),
            settings_open: false,
            views: crate::views::ViewStore::new(),
            #[cfg(test)]
            last_report_mode: "none".to_string(),
            filter_presets: crate::views::FilterStore::new(),
            #[cfg(test)]
            filter_preset_name: String::new(),
        }
    }
}

impl DiskMapApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::default();
        let state = app.safe_storage.read_state();
        app.restore_local_state(&state);
        configure_fonts(&cc.egui_ctx, app.locale);
        if let Some(theme) = app.theme_preference {
            apply_theme_preference(&cc.egui_ctx, theme);
        } else {
            app.theme_preference = Some(cc.egui_ctx.theme());
        }
        app
    }

    fn restore_local_state(&mut self, state: &LocalState) {
        self.restore_preferences(&state.preferences);
        self.profiles = state.profiles.clone();
        self.views = state.views.clone();
        self.filter_presets = state.filter_presets.clone();
        self.rules = state.rules.clone();
        #[cfg(test)]
        {
            self.pending_rules_import = None;
            self.last_rule_hits = None;
        }
    }

    fn restore_preferences(&mut self, prefs: &Preferences) {
        if let Some(path_input) = prefs.get(STORAGE_PATH_INPUT) {
            if !path_input.trim().is_empty() {
                self.path_input = path_input.to_string();
            }
        }

        if let Some(exclude_input) = prefs.get(STORAGE_EXCLUDE_INPUT) {
            self.exclude_input = exclude_input.to_string();
        }

        if let Some(protected_paths_input) = prefs.get(STORAGE_PROTECTED_PATHS) {
            self.protected_paths_input = protected_paths_input.to_string();
        }

        if let Some(include_hidden) = prefs
            .get(STORAGE_INCLUDE_HIDDEN)
            .and_then(parse_storage_bool)
        {
            self.include_hidden = include_hidden;
        }

        // Symlink traversal is intentionally disabled; migrate old settings.
        self.follow_symlinks = false;

        if let Some(stay_on_filesystem) = prefs
            .get(STORAGE_STAY_ON_FILESYSTEM)
            .and_then(parse_storage_bool)
        {
            self.stay_on_filesystem = stay_on_filesystem;
        }

        self.sqlite_cache_enabled = false;

        if let Some(search_filter_enabled) = prefs
            .get(STORAGE_SEARCH_FILTER)
            .and_then(parse_storage_bool)
        {
            self.search_filter_enabled = search_filter_enabled;
        }

        if let Some(color_by_extension) = prefs
            .get(STORAGE_COLOR_BY_EXTENSION)
            .and_then(parse_storage_bool)
        {
            self.color_by_extension = color_by_extension;
        }

        if let Some(realtime_watch_enabled) = prefs
            .get(STORAGE_REALTIME_WATCH)
            .and_then(parse_storage_bool)
        {
            let _ = self.scan.set_watch_enabled(realtime_watch_enabled);
        }

        if let Some(recent_roots) = prefs.get(STORAGE_RECENT_ROOTS) {
            self.recent_roots = parse_stored_paths(recent_roots, MAX_RECENT_ROOTS);
        }

        if let Some(pinned_roots) = prefs.get(STORAGE_PINNED_ROOTS) {
            self.pinned_roots = parse_stored_paths(pinned_roots, MAX_PINNED_ROOTS);
        }

        if let Some(depth) = prefs
            .get(STORAGE_MAX_DEPTH)
            .and_then(|value| value.parse::<usize>().ok())
        {
            self.max_depth = depth.clamp(1, 10);
        }

        self.theme_preference = prefs.get(STORAGE_THEME).and_then(parse_theme_preference);
        match prefs.get(STORAGE_LOCALE) {
            Some("system") | None => {
                self.locale_follow_system = true;
                self.locale = Locale::from_system();
            }
            Some(value) => {
                self.locale_follow_system = false;
                self.locale = Locale::from_storage(value).unwrap_or_else(Locale::from_system);
            }
        }
    }

    fn collect_preferences(&self) -> Preferences {
        let mut prefs = Preferences::default();
        prefs.set(STORAGE_PATH_INPUT, self.path_input.clone());
        prefs.set(STORAGE_EXCLUDE_INPUT, self.exclude_input.clone());
        prefs.set(STORAGE_PROTECTED_PATHS, self.protected_paths_input.clone());
        prefs.set(STORAGE_INCLUDE_HIDDEN, self.include_hidden.to_string());
        prefs.set(STORAGE_FOLLOW_SYMLINKS, false.to_string());
        prefs.set(
            STORAGE_STAY_ON_FILESYSTEM,
            self.stay_on_filesystem.to_string(),
        );
        prefs.set(STORAGE_SQLITE_CACHE, false.to_string());
        prefs.set(
            STORAGE_SEARCH_FILTER,
            self.search_filter_enabled.to_string(),
        );
        prefs.set(
            STORAGE_COLOR_BY_EXTENSION,
            self.color_by_extension.to_string(),
        );
        prefs.set(
            STORAGE_REALTIME_WATCH,
            self.scan.watch_enabled().to_string(),
        );
        prefs.set(STORAGE_RECENT_ROOTS, serialize_paths(&self.recent_roots));
        prefs.set(STORAGE_PINNED_ROOTS, serialize_paths(&self.pinned_roots));
        prefs.set(STORAGE_MAX_DEPTH, self.max_depth.to_string());
        if let Some(theme) = self.theme_preference {
            prefs.set(STORAGE_THEME, theme_preference_name(theme).to_string());
        }
        prefs.set(
            STORAGE_LOCALE,
            if self.locale_follow_system {
                "system"
            } else {
                self.locale.storage_name()
            },
        );
        prefs
    }

    fn save_preferences(&self) {
        if let Err(error) = self.safe_storage.write_state(&self.collect_local_state()) {
            eprintln!("disk-map: failed to write local state: {error}");
        }
    }

    fn collect_local_state(&self) -> LocalState {
        LocalState {
            preferences: self.collect_preferences(),
            profiles: self.profiles.clone(),
            views: self.views.clone(),
            filter_presets: self.filter_presets.clone(),
            rules: self.rules.clone(),
            ..LocalState::default()
        }
    }

    pub(super) fn text(&self, key: TextKey) -> &'static str {
        self.locale.text(key)
    }

    pub(super) fn reveal_action_text(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            self.text(TextKey::RevealInFinder)
        } else {
            self.text(TextKey::OpenContainingFolder)
        }
    }

    pub(super) fn localized_status_text(&self) -> String {
        let raw = self.status.display_text().into_owned();
        let (primary, watch_failure) = raw
            .split_once(" · Watch failed: ")
            .map_or((raw.as_str(), None), |(primary, error)| {
                (primary, Some(error))
            });
        let localized = if primary == "Ready" {
            self.text(TextKey::Ready).to_string()
        } else if primary == "Scanning..." {
            format!("{}...", self.text(TextKey::Scanning))
        } else if let Some(path) = primary.strip_prefix("Scanning ") {
            format!("{} {path}", self.text(TextKey::Scanning))
        } else if let Some(summary) = primary.strip_prefix("Finished: ") {
            format!("{}: {summary}", self.text(TextKey::Finished))
        } else if let Some(summary) = primary.strip_prefix("Cancelling scan...") {
            format!("{}{}", self.text(TextKey::CancellingScan), summary)
        } else if let Some(summary) = primary.strip_prefix("Rescanning after ") {
            format!("{} {summary}", self.text(TextKey::RescanningAfter))
        } else if let Some(summary) = primary.strip_prefix("Watch noticed ") {
            format!("{} {summary}", self.text(TextKey::WatchNoticed))
        } else if let Some(path) = primary.strip_prefix("Moved to Trash: ") {
            format!("{}: {path}", self.text(TextKey::MovedToTrash))
        } else if let Some(error) = primary.strip_prefix("Move to Trash failed: ") {
            format!("{}: {error}", self.text(TextKey::MoveToTrashFailed))
        } else {
            primary.to_string()
        };

        match watch_failure {
            Some(error) => format!("{localized} · {}: {error}", self.text(TextKey::WatchFailed)),
            None => localized,
        }
    }

    pub(super) fn set_locale_preference(
        &mut self,
        ctx: &egui::Context,
        follow_system: bool,
        locale: Locale,
    ) {
        let next_locale = if follow_system {
            Locale::from_system()
        } else {
            locale
        };
        if self.locale_follow_system == follow_system && self.locale == next_locale {
            return;
        }
        self.locale_follow_system = follow_system;
        self.locale = next_locale;
        configure_fonts(ctx, next_locale);
        self.save_preferences();
        self.pending_repaint = true;
    }

    #[cfg(test)]
    pub(super) fn persist_local_state(&mut self) {
        if let Err(error) = self.safe_storage.write_state(&self.collect_local_state()) {
            self.record_error(format!("local state save failed: {error}"));
            self.set_status(
                StatusSource::Persistence,
                StatusLevel::Error,
                format!("Local state save failed: {error}"),
            );
        }
        self.pending_repaint = true;
    }

    fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            exclude_patterns: parse_exclude_patterns(&self.exclude_input),
            cache_mode: CacheMode::Disabled,
            cache_path: None,
            include_hidden: self.include_hidden,
            follow_symlinks: false,
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
        self.handle_watch_events();
        self.maybe_refresh_search(ctx);
        self.maybe_request_deferred_repaint(ctx);
        self.drive_background_updates(ctx);

        panels::settings::show_settings_window(ctx, self);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            self.show_toolbar(ui);
        });

        egui::Panel::right("details_panel")
            .resizable(true)
            .default_size(320.0)
            .min_size(280.0)
            .max_size(420.0)
            .show_inside(ui, |ui| self.show_details_panel(ui));

        egui::Panel::bottom("status_bar")
            .exact_size(28.0)
            .show_inside(ui, |ui| self.show_status_bar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(0))
            .show_inside(ui, |ui| {
                self.show_treemap(ui);
            });

        panels::trash_confirmation::show(ui.ctx(), self);
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        // eframe passes its own storage here, but we use SafeStorage for
        // app preferences (crash-safe atomic writes). Window state is
        // managed by eframe's own persist_window.
        self.save_preferences();
    }
}

impl DiskMapApp {
    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if icon_button(ui, self.navigation.can_go_back(), ToolbarIcon::ArrowLeft)
                .on_hover_text(self.text(TextKey::Back))
                .clicked()
            {
                self.navigate_back();
            }

            if icon_button(
                ui,
                self.navigation.can_go_forward(),
                ToolbarIcon::ArrowRight,
            )
            .on_hover_text(self.text(TextKey::Forward))
            .clicked()
            {
                self.navigate_forward();
            }

            if icon_button(ui, self.navigation.can_go_up(&self.tree), ToolbarIcon::Up)
                .on_hover_text(self.text(TextKey::UpToParent))
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
            .on_hover_text(self.text(TextKey::ReturnToRoot))
            .clicked()
            {
                self.return_to_scan_root();
            }

            if icon_button(ui, true, ToolbarIcon::Refresh)
                .on_hover_text(self.text(TextKey::RefreshLayout))
                .clicked()
            {
                self.refresh_treemap_layout();
            }

            ui.add_space(4.0);

            if icon_button(ui, true, ToolbarIcon::Settings)
                .on_hover_text(self.text(TextKey::Settings))
                .clicked()
            {
                self.settings_open = true;
            }

            self.show_roots_menu(ui);

            let scan_label = if self.scan.is_scanning() {
                self.text(TextKey::Cancel)
            } else {
                self.text(TextKey::Scan)
            };
            let scan_icon = if self.scan.is_scanning() {
                egui_phosphor::regular::X
            } else {
                egui_phosphor::regular::FOLDER_OPEN
            };
            if icon_text_button(ui, true, scan_icon, scan_label, 86.0).clicked() {
                if self.scan.is_scanning() {
                    self.cancel_scan();
                } else {
                    self.start_scan();
                }
            }

            ui.add_space(8.0);
            ui.label(
                RichText::new(self.text(TextKey::Depth).to_uppercase())
                    .size(10.0)
                    .color(palette(ui.ctx()).text_faint)
                    .strong(),
            );
            if ui
                .add_sized(
                    [110.0, 18.0],
                    egui::Slider::new(&mut self.max_depth, 1..=10).text(""),
                )
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
        panels::roots_menu::show_roots_menu(ui, self);
    }

    fn show_details_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("details_panel_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                panels::details::show(ui, self);
            });
    }

    fn show_progress_section(&self, ui: &mut egui::Ui, p: &Palette) {
        panels::sections::show_progress_section(ui, p, self);
    }

    fn show_state_message(&self, ui: &mut egui::Ui, p: &Palette, message: &StateMessage) {
        panels::sections::show_state_message(ui, p, message);
    }

    fn show_scan_issue_section(&mut self, ui: &mut egui::Ui, p: &Palette) {
        panels::sections::show_scan_issue_section(ui, p, self);
    }

    fn show_status_bar(&self, ui: &mut egui::Ui) {
        panels::sections::show_status_bar(ui, self);
    }

    fn no_root_state_message(&self) -> StateMessage {
        match self.scan.phase() {
            ScanPhase::Running => StateMessage {
                title: self.text(TextKey::StartingScan),
                detail: format!(
                    "{}: {}",
                    self.text(TextKey::ScanRoot),
                    self.path_input.trim()
                ),
            },
            ScanPhase::Failed(message) => StateMessage {
                title: self.text(TextKey::UnableToScanPath),
                detail: message.clone(),
            },
            ScanPhase::Cancelled => StateMessage {
                title: self.text(TextKey::ScanCancelled),
                detail: self.text(TextKey::ChoosePathToScan).to_string(),
            },
            ScanPhase::Idle | ScanPhase::Finished => StateMessage {
                title: self.text(TextKey::NoScanLoaded),
                detail: self.text(TextKey::ChoosePathToScan).to_string(),
            },
        }
    }

    fn empty_root_state_message(&self, root_id: NodeId) -> Option<StateMessage> {
        let root = self.tree.node(root_id);
        match root.kind {
            NodeKind::File => {
                return Some(StateMessage {
                    title: "File scanned",
                    detail: format!("{} uses {}.", root.name, format_bytes(root.size)),
                });
            }
            NodeKind::Symlink => {
                return Some(StateMessage {
                    title: "Symbolic link",
                    detail: format!("{} was recorded without following its target.", root.name),
                });
            }
            NodeKind::Error => {
                return Some(StateMessage {
                    title: "Unable to read item",
                    detail: root
                        .error
                        .clone()
                        .unwrap_or_else(|| format!("{} could not be read.", root.name)),
                });
            }
            NodeKind::Aggregate => {
                return Some(StateMessage {
                    title: "Grouped files",
                    detail: format!("This group uses {}.", format_bytes(root.size)),
                });
            }
            NodeKind::Dir => {}
        }

        if root.size == 0 && !root.children.is_empty() {
            let item_count = root.children.len() as u64;
            return Some(StateMessage {
                title: if self.scan.is_scanning() {
                    "Scanning folder"
                } else {
                    "No disk space used"
                },
                detail: format!(
                    "{} contains {}; the visible items currently use 0 B.",
                    root.name,
                    pluralize(item_count, "item", "items")
                ),
            });
        }

        if !root.children.is_empty() {
            return None;
        }

        if self.scan.is_scanning() {
            Some(StateMessage {
                title: "Waiting for first results",
                detail: format!("Scanning {}.", self.path_input.trim()),
            })
        } else {
            Some(StateMessage {
                title: self.text(TextKey::EmptyFolder),
                detail: format!("{} · 0 B", root.name),
            })
        }
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
        panels::treemap_view::show(ui, self);
    }

    fn show_hover_tooltip(&mut self, ui: &egui::Ui, node_id: NodeId, pos: Pos2) {
        let p = palette(ui.ctx());
        let node_path = self.tree.node_real_path(node_id);
        let node = self.tree.node(node_id);
        let tooltip_max_width = (ui.ctx().content_rect().width() - HOVER_TOOLTIP_SCREEN_MARGIN)
            .clamp(HOVER_TOOLTIP_MIN_WIDTH, HOVER_TOOLTIP_MAX_WIDTH);
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
                        ui.set_max_width(tooltip_max_width);
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
                                .wrap(),
                            );
                        } else {
                            ui.label(
                                RichText::new(self.text(TextKey::AggregatedSmallFiles))
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

    pub(super) fn show_small_files_hover_tooltip(
        &mut self,
        ui: &egui::Ui,
        parent_id: NodeId,
        count: u32,
        size: u64,
        pos: Pos2,
    ) {
        let p = palette(ui.ctx());
        let parent_name = self.tree.node(parent_id).name.clone();
        egui::Area::new(egui::Id::new("small_files_hover_tooltip"))
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
                        ui.label(
                            RichText::new(format!("{} ({count})", self.text(TextKey::OtherFiles)))
                                .strong()
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(format_bytes(size))
                                .monospace()
                                .color(p.accent),
                        );
                        ui.label(
                            RichText::new(format!("Grouped from {}", parent_name))
                                .small()
                                .color(p.text_muted),
                        );
                        ui.label(
                            RichText::new(self.text(TextKey::SelectDirectory))
                                .small()
                                .color(p.text_faint),
                        );
                    });
            });
    }

    fn paint_visual(&self, ui: &egui::Ui, painter: &egui::Painter, visual: &VisualNode) {
        let palette = palette(ui.ctx());
        let is_hovered = self.hovered_visual_kind == Some(visual.kind);
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
        let VisualKind::Node(node_id) = visual.kind else {
            return None;
        };
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
        // A modal owns keyboard input while a destructive action is pending.
        // In particular, Escape must cancel the modal without clearing the
        // selection/search state underneath it.
        if self.trash_confirm_target_id.is_some() {
            return;
        }

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
            if self.settings_open {
                self.settings_open = false;
            } else if self.navigation.selected_id().is_some() {
                self.navigation.set_selected_id(None);
            } else if !self.search.input().is_empty() {
                self.clear_search();
            }
        }
    }

    fn handle_scan_messages(&mut self) {
        while let Some(event) = self.scan.try_next_event() {
            self.apply_scan_event(event);
        }
    }

    fn apply_scan_event(&mut self, event: ScanSessionEvent) {
        match event {
            ScanSessionEvent::Started { path, root_node } => {
                self.set_status(
                    StatusSource::Scan,
                    StatusLevel::Progress,
                    format!("Scanning {}", path.display()),
                );
                self.record_recent_root(&path);
                self.tree.clear();
                self.tree.push_node(None, root_node);
                self.tree.set_root_path(path);
                self.navigation.set_scan_root(self.tree.root);
                self.treemap.invalidate();
                self.mark_search_dirty();
                self.navigation.rebuild_breadcrumb_cache(&self.tree);
            }
            ScanSessionEvent::Batch(batch) => {
                self.apply_scan_batch(batch);
                self.pending_repaint = true;
            }
            ScanSessionEvent::Finished {
                total_bytes,
                follow_up_rescan,
                watch_error,
            } => {
                self.prune_invalid_selection();
                self.refresh_search_matches();
                self.treemap.invalidate();
                self.set_status(
                    StatusSource::Scan,
                    StatusLevel::Success,
                    self.finished_status(total_bytes),
                );
                #[cfg(test)]
                self.update_snapshot_comparison();
                if let Some(error) = watch_error {
                    self.record_error(format!("watch failed: {error}"));
                    self.status.set_watch_failure(error);
                } else if self.scan.watch_active() {
                    self.status.clear_watch_failure();
                }
                if let Some(path) = follow_up_rescan {
                    self.start_scan_path(path);
                }
                self.pending_repaint = true;
                eprintln!("{}", format_perf_stats(self.scan.perf_stats()));
            }
            ScanSessionEvent::Cancelled { watch_paused } => {
                let text = if watch_paused {
                    "Scan cancelled · Watch paused until the next successful scan".to_string()
                } else {
                    "Scan cancelled".to_string()
                };
                self.set_status(StatusSource::Scan, StatusLevel::Warning, text);
                self.pending_repaint = true;
                eprintln!("{}", format_perf_stats(self.scan.perf_stats()));
            }
            ScanSessionEvent::Error {
                message,
                watch_paused,
            } => {
                self.record_error(format!("scan error: {message}"));
                let text = if watch_paused {
                    format!("Error: {message} · Watch paused until the next successful scan")
                } else {
                    format!("Error: {message}")
                };
                self.set_status(StatusSource::Scan, StatusLevel::Error, text);
                self.pending_repaint = true;
                eprintln!("{}", format_perf_stats(self.scan.perf_stats()));
            }
        }
    }

    fn apply_scan_batch(&mut self, batch: ScanBatch) {
        let application = batch.apply_to_tree(&mut self.tree);
        let touched_visible_subtree = application
            .dirty_node_ids
            .iter()
            .copied()
            .any(|node_id| self.batch_touches_visible_subtree(node_id));
        if application.had_progress {
            self.set_status(StatusSource::Scan, StatusLevel::Progress, "Scanning...");
        }

        if !application.discovered_node_ids.is_empty() && !self.search.query().is_empty() {
            let updates = self
                .search
                .ingest_new_nodes(&mut self.tree, &application.discovered_node_ids)
                as u64;
            self.scan.record_search_incremental_updates(updates);
        }

        if touched_visible_subtree {
            self.treemap.invalidate();
        }
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
            self.set_status(
                StatusSource::Roots,
                StatusLevel::Info,
                format!("Unpinned {}", truncate_middle(path, 48)),
            );
        } else {
            push_unique_front(&mut self.pinned_roots, path.to_string(), MAX_PINNED_ROOTS);
            self.set_status(
                StatusSource::Roots,
                StatusLevel::Info,
                format!("Pinned {}", truncate_middle(path, 48)),
            );
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

    #[cfg(test)]
    fn update_snapshot_comparison(&mut self) {
        self.last_report_mode = "snapshot".to_string();
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

    pub(super) fn clear_search(&mut self) {
        self.search.clear(self.tree.len());
        self.treemap.invalidate();
    }

    fn apply_platform_result(&mut self, action: &str, result: anyhow::Result<()>) {
        if let Err(error) = result {
            self.record_error(format!("{action} failed: {error}"));
            self.set_status(
                StatusSource::Platform,
                StatusLevel::Error,
                format!("{action} failed: {error}"),
            );
            self.pending_repaint = true;
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn open_full_disk_access_settings(&mut self) {
        match crate::platform::open_full_disk_access_settings() {
            Ok(()) => {
                self.set_status(
                    StatusSource::Platform,
                    StatusLevel::Info,
                    "Enable DiskMap in Full Disk Access, then quit and reopen it before rescanning",
                );
                self.pending_repaint = true;
            }
            Err(error) => self.apply_platform_result("Open Full Disk Access settings", Err(error)),
        }
    }

    pub(in crate::app) fn set_status(
        &mut self,
        source: StatusSource,
        level: StatusLevel,
        text: impl Into<String>,
    ) {
        self.status.set_primary(source, level, text);
    }

    /// Record an error or notable status change for the diagnostics
    /// export. Capped at 64 entries (oldest dropped on overflow).
    pub(super) fn record_error(&mut self, message: String) {
        const MAX_RECENT_ERRORS: usize = 64;
        if self.recent_errors.len() >= MAX_RECENT_ERRORS {
            self.recent_errors.pop_front();
        }
        self.recent_errors.push_back(message);
    }

    /// Evaluate the current `rules` against the focused subtree and
    /// cache the result in `last_rule_hits`. Returns the number of
    /// hits found (capped at the same limit the engine uses).
    #[cfg(test)]
    pub fn evaluate_current_rules(&mut self) -> usize {
        self.last_report_mode = "rules".to_string();
        use crate::rules::{evaluate_rules, RuleContext, INSIGHT_REPORT_LIMIT_FROM_RULES};
        let root_id = match self.navigation.focused_root().or(self.tree.root) {
            Some(id) => id,
            None => {
                self.last_rule_hits = Some(Vec::new());
                self.set_status(
                    StatusSource::Rules,
                    StatusLevel::Warning,
                    "No scan loaded — apply rules to current view",
                );
                self.pending_repaint = true;
                return 0;
            }
        };
        let ctx = RuleContext {
            now_unix_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let hits = evaluate_rules(
            &self.rules,
            &mut self.tree,
            root_id,
            &ctx,
            INSIGHT_REPORT_LIMIT_FROM_RULES,
        );
        let count = hits.len();
        self.last_rule_hits = Some(hits);
        self.set_status(
            StatusSource::Rules,
            StatusLevel::Success,
            format!(
                "Applied {} rules, found {count} hits",
                self.rules.enabled_count()
            ),
        );
        self.pending_repaint = true;
        count
    }

    /// Capture the current view state under `root`. Existing entry
    /// for the same root is overwritten (last-write-wins, same
    /// convention as `ProfileStore::set`).
    #[cfg(test)]
    pub fn save_current_view(&mut self, root: &str) {
        use crate::views::ViewState;
        let state = ViewState {
            depth: self.max_depth,
            search_query: self.search.input().to_string(),
            search_filter_enabled: self.search_filter_enabled,
            color_by_extension: self.color_by_extension,
            last_report_mode: self.last_report_mode.clone(),
            focused_id: self.navigation.focused_root(),
            selected_id: self.navigation.selected_id(),
        };
        self.views.set(root, state);
        self.set_status(
            StatusSource::View,
            StatusLevel::Success,
            format!("Saved view for {} ({} stored)", root, self.views.len()),
        );
        self.persist_local_state();
    }

    /// Apply a previously-saved view's state to the live UI fields.
    /// No-op if no view is stored for `root`. Does not change the
    /// loaded scan — the user re-runs the scan if they want to
    /// re-evaluate search results against new data.
    #[cfg(test)]
    pub fn apply_saved_view(&mut self, root: &str) {
        let Some(view) = self.views.get(root).cloned() else {
            return;
        };
        self.max_depth = view.depth.clamp(1, 10);
        *self.search.input_mut() = view.search_query;
        self.search_filter_enabled = view.search_filter_enabled;
        self.color_by_extension = view.color_by_extension;
        self.last_report_mode = view.last_report_mode;
        if let Some(focused) = view.focused_id {
            if self.tree.contains_id(focused) {
                self.navigation.set_focused_root(Some(focused));
            }
        }
        if let Some(selected) = view.selected_id {
            if self.tree.contains_id(selected) {
                self.navigation.set_selected_id(Some(selected));
            }
        }
        self.treemap.invalidate();
        self.mark_search_dirty();
        self.set_status(
            StatusSource::View,
            StatusLevel::Success,
            format!("Applied saved view for {}", root),
        );
        self.pending_repaint = true;
    }

    /// Add a new filter preset with the given name. Uses the current
    /// search query and filter toggle as the preset body. Returns
    /// false (and does nothing) if the name is empty or already
    /// taken, so the UI can show a clear status.
    #[cfg(test)]
    pub fn add_filter_preset(&mut self, name: &str) -> bool {
        use crate::views::FilterPreset;
        let trimmed = name.trim();
        if trimmed.is_empty() {
            self.set_status(
                StatusSource::View,
                StatusLevel::Warning,
                "Filter preset name cannot be empty",
            );
            self.pending_repaint = true;
            return false;
        }
        let preset = FilterPreset {
            name: trimmed.to_string(),
            query: self.search.input().to_string(),
            filter_enabled: self.search_filter_enabled,
        };
        if self.filter_presets.add(preset) {
            self.set_status(
                StatusSource::View,
                StatusLevel::Success,
                format!(
                    "Saved filter preset '{}' ({} total)",
                    trimmed,
                    self.filter_presets.len()
                ),
            );
            self.filter_preset_name.clear();
            self.persist_local_state();
            true
        } else {
            self.set_status(
                StatusSource::View,
                StatusLevel::Warning,
                format!("Filter preset '{}' already exists", trimmed),
            );
            self.pending_repaint = true;
            false
        }
    }

    /// Apply a saved filter preset to the live search state. No-op
    /// if no preset by that name exists. Marks the search dirty so
    /// the next frame re-runs the matcher.
    #[cfg(test)]
    pub fn apply_filter_preset(&mut self, name: &str) {
        let Some(preset) = self.filter_presets.get(name).cloned() else {
            return;
        };
        *self.search.input_mut() = preset.query.clone();
        self.search_filter_enabled = preset.filter_enabled;
        self.mark_search_dirty();
        self.treemap.invalidate();
        self.set_status(
            StatusSource::View,
            StatusLevel::Success,
            format!("Applied filter preset '{}'", name),
        );
        self.pending_repaint = true;
    }

    /// Remove a saved filter preset by name.
    #[cfg(test)]
    pub fn remove_filter_preset(&mut self, name: &str) -> bool {
        if self.filter_presets.remove(name).is_some() {
            self.set_status(
                StatusSource::View,
                StatusLevel::Success,
                format!("Removed filter preset '{}'", name),
            );
            self.persist_local_state();
            true
        } else {
            false
        }
    }

    /// Build a snapshot bundle from the current app state and write it
    /// to a timestamped directory under `dest_dir`. Returns the path to
    /// the created bundle directory, or an error if the write failed.
    #[cfg(test)]
    pub fn export_diagnostics(&mut self, dest_dir: &Path) -> anyhow::Result<PathBuf> {
        use crate::diagnostics::DiagnosticsBundle;
        let scan_root = self.tree.root.and_then(|id| self.tree.node_real_path(id));
        let scan_root = scan_root.map(|p| p.display().to_string());
        let scan_options: Vec<(String, String)> = vec![
            ("include_hidden".into(), self.include_hidden.to_string()),
            ("follow_symlinks".into(), "false".into()),
            (
                "stay_on_filesystem".into(),
                self.stay_on_filesystem.to_string(),
            ),
            (
                "sqlite_cache_enabled".into(),
                self.sqlite_cache_enabled.to_string(),
            ),
            (
                "realtime_watch_enabled".into(),
                self.scan.watch_enabled().to_string(),
            ),
            (
                "search_filter_enabled".into(),
                self.search_filter_enabled.to_string(),
            ),
            (
                "color_by_extension".into(),
                self.color_by_extension.to_string(),
            ),
            ("exclude_patterns".into(), self.exclude_input.clone()),
            ("protected_paths".into(), self.protected_paths_input.clone()),
            ("max_depth".into(), self.max_depth.to_string()),
        ];
        let bundle = DiagnosticsBundle {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            generated_at_unix_secs: crate::diagnostics::current_unix_secs(),
            scan_root,
            status: self.status.display_text().into_owned(),
            scan_options,
            perf_stats: self.scan.terminal_perf_stats().cloned(),
            recent_errors: self.recent_errors.iter().cloned().collect(),
        };
        bundle
            .write_to(dest_dir)
            .map_err(|e| anyhow::anyhow!("write diagnostics bundle: {e}"))
    }

    fn handle_watch_events(&mut self) {
        match self.scan.poll_watch(Instant::now()) {
            WatchAction::Noop => {}
            WatchAction::Pending => {
                self.pending_repaint = true;
            }
            WatchAction::Deferred { change_count } => {
                self.set_status(
                    StatusSource::Watch,
                    StatusLevel::Progress,
                    format!(
                        "Watch noticed {} while scan is running",
                        pluralize(change_count as u64, "change", "changes")
                    ),
                );
                self.pending_repaint = true;
            }
            WatchAction::Rescan { path, change_count } => {
                self.set_status(
                    StatusSource::Watch,
                    StatusLevel::Progress,
                    format!(
                        "Rescanning after {}",
                        pluralize(change_count as u64, "change", "changes")
                    ),
                );
                self.start_scan_path(path);
            }
            WatchAction::Failed(error) => {
                self.record_error(format!("watch failed: {error}"));
                self.status.set_watch_failure(error);
                self.pending_repaint = true;
            }
        }
    }

    fn start_scan(&mut self) {
        let path = std::path::PathBuf::from(self.path_input.trim());
        self.start_scan_path(path);
    }

    fn start_scan_path(&mut self, path: std::path::PathBuf) {
        self.scan.start(path.clone(), self.scan_options());

        self.tree.clear();
        self.navigation.clear_for_new_scan();
        self.hovered_id = None;
        self.context_menu_target_id = None;
        self.trash_confirm_target_id = None;
        self.trash_confirm_path = None;
        self.cleanup_queue.clear();
        self.hovered_visual_kind = None;
        #[cfg(test)]
        {
            self.snapshot_diff = None;
        }
        self.duplicate_report = None;
        self.insight_report = None;
        self.search.clear(0);
        self.treemap.clear();
        self.path_input = path.display().to_string();
        self.set_status(
            StatusSource::Scan,
            StatusLevel::Progress,
            format!("Scanning {}", path.display()),
        );
        self.pending_repaint = true;
    }

    pub(super) fn set_realtime_watch_enabled(&mut self, enabled: bool) {
        match self.scan.set_watch_enabled(enabled) {
            Ok(()) if enabled && self.scan.watch_active() => {
                self.status.clear_watch_failure();
            }
            Err(error) => {
                self.record_error(format!("watch failed: {error}"));
                self.status.set_watch_failure(error);
            }
            Ok(()) => {}
        }
        if !enabled {
            self.status.clear_watch_failure();
        }
        self.pending_repaint = true;
    }

    pub(super) fn realtime_watch_enabled(&self) -> bool {
        self.scan.watch_enabled()
    }

    fn stop_watching(&mut self) {
        self.scan.pause_watching();
        self.status.clear_watch_failure();
    }

    fn scan_root_rescan_path(&mut self) -> Option<std::path::PathBuf> {
        self.tree
            .root
            .and_then(|root_id| self.tree.node_real_path(root_id))
    }

    fn cancel_scan(&mut self) {
        if self.scan.cancel() {
            self.set_status(
                StatusSource::Scan,
                StatusLevel::Progress,
                "Cancelling scan...",
            );
            self.pending_repaint = true;
        }
    }

    fn enter_root(&mut self, node_id: NodeId, push_history: bool) {
        let outcome = self.navigation.enter_root(node_id, push_history);
        self.apply_navigation_outcome(outcome);
    }

    pub(super) fn return_to_scan_root(&mut self) {
        let outcome = self.navigation.return_to_scan_root(&self.tree);
        self.apply_navigation_outcome(outcome);
    }

    pub(super) fn navigate_back(&mut self) {
        let outcome = self.navigation.navigate_back();
        self.apply_navigation_outcome(outcome);
    }

    fn navigate_forward(&mut self) {
        let outcome = self.navigation.navigate_forward();
        self.apply_navigation_outcome(outcome);
    }

    pub(super) fn refresh_treemap_layout(&mut self) {
        self.mark_layout_dirty_now();
    }

    fn mark_layout_dirty_now(&mut self) {
        self.treemap.invalidate_now();
    }

    fn apply_navigation_outcome(&mut self, outcome: NavigationOutcome) {
        match outcome {
            NavigationOutcome::Noop => {}
            NavigationOutcome::RefreshLayoutOnly => self.mark_layout_dirty_now(),
            NavigationOutcome::FocusChanged { refresh_search } => {
                self.mark_layout_dirty_now();
                if refresh_search {
                    self.refresh_search_matches();
                }
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
            self.treemap.invalidate();
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

    pub(super) fn enter_selected_directory(&mut self) -> bool {
        let Some(selected_id) = self.navigation.selected_id() else {
            return false;
        };
        if !self.tree.contains_id(selected_id) || self.tree.node(selected_id).children.is_empty() {
            return false;
        }
        self.enter_root(selected_id, true);
        true
    }

    pub(super) fn increase_depth(&mut self) -> bool {
        if self.max_depth >= 10 {
            return false;
        }
        self.max_depth += 1;
        self.mark_layout_dirty_now();
        true
    }

    pub(super) fn decrease_depth(&mut self) -> bool {
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
        } else if self.scan.watch_active() {
            ctx.request_repaint_after(LAYOUT_REFRESH_INTERVAL);
        } else if self.search.is_dirty() {
            ctx.request_repaint_after(SEARCH_REFRESH_INTERVAL);
        }
    }

    #[cfg(test)]
    fn apply_scan_message_for_test(&mut self, message: ScanMessage) {
        let Some(event) = self.scan.process_message_for_test(message) else {
            return;
        };
        self.apply_scan_event(event);
    }

    #[cfg(test)]
    fn set_active_scan_id_for_test(&mut self, scan_id: u64) {
        self.scan.set_active_id_for_test(scan_id);
    }
}

pub(super) fn find_hovered_visual(
    visuals: &[VisualNode],
    pos: Option<Pos2>,
) -> Option<&VisualNode> {
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

pub(super) fn describe_node_kind(kind: NodeKind, has_children: bool) -> &'static str {
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
    Settings,
    ThemeLight,
    ThemeDark,
}

fn icon_button(ui: &mut egui::Ui, enabled: bool, icon: ToolbarIcon) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(icon_glyph(icon)).size(17.0)).min_size(Vec2::splat(30.0)),
    )
}

fn icon_glyph(icon: ToolbarIcon) -> &'static str {
    use egui_phosphor::regular;
    match icon {
        ToolbarIcon::ArrowLeft => regular::ARROW_LEFT,
        ToolbarIcon::ArrowRight => regular::ARROW_RIGHT,
        ToolbarIcon::Up => regular::ARROW_UP,
        ToolbarIcon::Home => regular::HOUSE,
        ToolbarIcon::Refresh => regular::ARROW_CLOCKWISE,
        ToolbarIcon::Settings => regular::GEAR,
        ToolbarIcon::ThemeLight => regular::SUN,
        ToolbarIcon::ThemeDark => regular::MOON,
    }
}

pub(super) fn icon_text_button(
    ui: &mut egui::Ui,
    enabled: bool,
    icon: &'static str,
    label: &str,
    width: f32,
) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(format!("{icon}  {label}")))
            .min_size(Vec2::new(width, 32.0)),
    )
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

pub(super) fn section_divider(ui: &mut egui::Ui, palette: &Palette) {
    let (_, rect) = ui.allocate_space(Vec2::new(ui.available_width(), 1.0));
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0, palette.stroke_subtle),
    );
}

fn protected_path_status(reason: crate::cleanup::ProtectedPathReason, path: &Path) -> String {
    format!(
        "Protected path blocked: {} ({})",
        path.display(),
        reason.label()
    )
}

fn cleanup_target_missing_status(path: &Path) -> String {
    format!(
        "Move to Trash unavailable: target no longer exists: {}",
        path.display()
    )
}

fn cleanup_target_inaccessible_status(path: &Path, error: &str) -> String {
    format!(
        "Move to Trash unavailable: cannot verify {}: {}",
        path.display(),
        error
    )
}

#[cfg(test)]
fn current_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn dirs_home_fallback() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}

pub(super) fn truncate_middle(input: &str, max_chars: usize) -> String {
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

pub(super) fn pluralize(count: u64, singular: &str, plural: &str) -> String {
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
    use crate::storage::SafeStorage;
    use crate::tree::NodeRecord;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Allocate a fresh temp directory for one test. Includes pid + nanos
    /// + an atomic counter so parallel tests don't collide.
    fn unique_temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let p = std::env::temp_dir().join(format!("disk-map-prefs-test-{pid}-{nanos}-{n}"));
        std::fs::create_dir_all(&p).expect("temp dir should be creatable");
        p
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

    #[test]
    fn scroll_wrapped_details_panel_does_not_expand_central_treemap_area() {
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0));
        let mut central_rect = None;

        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| {
                egui::Panel::top("layout_test_toolbar")
                    .exact_size(40.0)
                    .show_inside(ui, |ui| {
                        ui.label("toolbar");
                    });

                egui::Panel::right("layout_test_details")
                    .exact_size(280.0)
                    .show_inside(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("layout_test_details_scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for row in 0..200 {
                                    ui.label(format!("details row {row}"));
                                }
                            });
                    });

                egui::Panel::bottom("layout_test_status")
                    .exact_size(28.0)
                    .show_inside(ui, |ui| {
                        ui.label("status");
                    });

                egui::CentralPanel::default()
                    .frame(egui::Frame::central_panel(ui.style()).inner_margin(0))
                    .show_inside(ui, |ui| {
                        central_rect = Some(ui.max_rect().intersect(ui.clip_rect()));
                    });
            },
        );

        let central_rect = central_rect.expect("central panel should be shown");
        let expected_bottom = screen.bottom() - 28.0;
        assert!(
            central_rect.bottom() <= expected_bottom + 1.0,
            "central panel bottom {} should stay within visible viewport bottom {expected_bottom}",
            central_rect.bottom()
        );
        assert!(
            central_rect.height() <= screen.height() - 40.0 - 28.0 + 1.0,
            "central panel height {} should not include overflowing details content",
            central_rect.height()
        );
    }

    #[test]
    fn treemap_render_does_not_clear_details_panel_trash_confirmation() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![DiscoveredNode {
                    node_id: 1,
                    parent_id: 0,
                    node: NodeRecord {
                        name: "file.bin".into(),
                        kind: NodeKind::File,
                        size: 1,
                        modified_secs: None,
                        scanned: true,
                        error: None,
                    },
                }],
                size_deltas: vec![(0, 1)],
                scanned_nodes: vec![1],
                progress: None,
            },
        });
        app.trash_confirm_target_id = Some(1);
        app.trash_confirm_path = Some(PathBuf::from("/root/file.bin"));

        let ctx = egui::Context::default();
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                ..Default::default()
            },
            |ui| panels::treemap_view::show(ui, &mut app),
        );

        assert_eq!(app.trash_confirm_target_id, Some(1));
        assert_eq!(
            app.trash_confirm_path,
            Some(PathBuf::from("/root/file.bin"))
        );
    }

    #[test]
    fn cjk_font_install_adds_fallback_to_ui_font_families() {
        let mut fonts = egui::FontDefinitions::default();

        install_cjk_font_data(&mut fonts, vec![0, 1, 2, 3]);

        assert!(fonts.font_data.contains_key("disk-map-cjk"));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let entries = fonts
                .families
                .get(&family)
                .expect("default font family should exist");
            assert_eq!(entries.last().map(String::as_str), Some("disk-map-cjk"));
        }
    }

    #[test]
    fn first_readable_font_path_skips_missing_candidates() {
        let dir = unique_temp_dir();
        let missing = dir.join("missing-font.ttf");
        let readable = dir.join("readable-font.ttf");
        std::fs::write(&readable, b"font").expect("test font file should be writable");
        let candidates = [
            missing.display().to_string(),
            readable.display().to_string(),
        ];

        let selected = first_readable_font_path(candidates.iter().map(String::as_str));

        assert_eq!(selected, Some(candidates[1].as_str()));
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

    fn unique_temp_path(prefix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let temp_root = std::env::current_dir()
            .expect("test current dir should be available")
            .join("target/test-temp");
        temp_root.join(format!("{prefix}-{nanos}"))
    }

    fn drain_scan_for_test(app: &mut DiskMapApp) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let Some(event) = app.scan.try_next_event() else {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            };
            let finished = matches!(&event, ScanSessionEvent::Finished { .. });
            let terminal = matches!(
                &event,
                ScanSessionEvent::Finished { .. }
                    | ScanSessionEvent::Cancelled { .. }
                    | ScanSessionEvent::Error { .. }
            );
            app.apply_scan_event(event);
            if terminal {
                return finished;
            }
        }
        false
    }

    #[test]
    fn scan_messages_build_tree_correctly() {
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
    fn scan_batches_leave_child_sorting_lazy_until_a_reader_needs_it() {
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
                            name: "small.bin".into(),
                            kind: NodeKind::File,
                            size: 1,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                    DiscoveredNode {
                        node_id: 2,
                        parent_id: 0,
                        node: NodeRecord {
                            name: "large.bin".into(),
                            kind: NodeKind::File,
                            size: 10,
                            modified_secs: None,
                            scanned: true,
                            error: None,
                        },
                    },
                ],
                size_deltas: vec![(0, 11)],
                scanned_nodes: vec![1, 2],
                progress: None,
            },
        });

        assert_eq!(app.tree.node(0).children, vec![1, 2]);
        assert!(app.treemap.is_dirty());

        app.tree.ensure_sorted_children(0);

        assert_eq!(app.tree.sorted_children(0), &[2, 1]);
    }

    #[test]
    fn scan_root_symlink_is_counted_in_issue_summary() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(ScanMessage::Started {
            scan_id: 1,
            path: "/root-link".into(),
            root_node: NodeRecord {
                name: "root-link".into(),
                kind: NodeKind::Symlink,
                size: 0,
                modified_secs: None,
                scanned: true,
                error: None,
            },
        });

        assert_eq!(app.scan.issue_summary().symlinks, 1);
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
        let options = DiskMapApp::default().scan_options();

        assert_eq!(options.cache_mode, CacheMode::Disabled);
        assert!(options.cache_path.is_none());
    }

    #[test]
    fn sqlite_cache_setting_is_ignored_for_simplified_ui() {
        let app = DiskMapApp {
            sqlite_cache_enabled: true,
            safe_storage: SafeStorage::new(&unique_temp_dir()),
            ..Default::default()
        };
        let options = app.scan_options();

        assert_eq!(options.cache_mode, CacheMode::Disabled);
        assert!(options.cache_path.is_none());
    }

    #[test]
    fn default_scan_options_preserve_safe_scan_defaults() {
        let options = DiskMapApp::default().scan_options();

        assert!(options.include_hidden);
        assert!(!options.follow_symlinks);
        assert!(!options.stay_on_filesystem);
    }

    #[test]
    fn realtime_watch_defaults_to_enabled() {
        let app = DiskMapApp::default();

        assert!(app.realtime_watch_enabled());
        assert!(!app.scan.watch_active());
    }

    #[test]
    fn preferences_round_trip_through_safe_storage() {
        let dir = unique_temp_dir();
        let store = SafeStorage::new(&dir);

        let mut app = DiskMapApp {
            path_input: "/next".into(),
            exclude_input: "node_modules;target".into(),
            protected_paths_input: "/keep\n/safe".into(),
            include_hidden: false,
            follow_symlinks: true,
            stay_on_filesystem: true,
            sqlite_cache_enabled: true,
            search_filter_enabled: true,
            color_by_extension: true,
            recent_roots: vec!["/recent".into(), "/older".into()],
            pinned_roots: vec!["/pinned".into()],
            max_depth: 4,
            theme_preference: Some(Theme::Light),
            locale: Locale::TraditionalChinese,
            locale_follow_system: false,
            safe_storage: SafeStorage::new(&dir),
            ..Default::default()
        };
        app.set_realtime_watch_enabled(false);

        app.save_preferences();
        let prefs = store.read();
        assert_eq!(prefs.get(STORAGE_PATH_INPUT), Some("/next"));
        assert_eq!(prefs.get(STORAGE_MAX_DEPTH), Some("4"));
        assert_eq!(
            prefs.get(STORAGE_EXCLUDE_INPUT),
            Some("node_modules;target")
        );
        assert_eq!(prefs.get(STORAGE_PROTECTED_PATHS), Some("/keep\n/safe"));
        assert_eq!(prefs.get(STORAGE_INCLUDE_HIDDEN), Some("false"));
        assert_eq!(prefs.get(STORAGE_FOLLOW_SYMLINKS), Some("false"));
        assert_eq!(prefs.get(STORAGE_STAY_ON_FILESYSTEM), Some("true"));
        assert_eq!(prefs.get(STORAGE_SQLITE_CACHE), Some("false"));
        assert_eq!(prefs.get(STORAGE_SEARCH_FILTER), Some("true"));
        assert_eq!(prefs.get(STORAGE_COLOR_BY_EXTENSION), Some("true"));
        assert_eq!(prefs.get(STORAGE_REALTIME_WATCH), Some("false"));
        assert_eq!(prefs.get(STORAGE_THEME), Some("light"));
        assert_eq!(prefs.get(STORAGE_LOCALE), Some("zh-Hant"));
        assert_eq!(prefs.get(STORAGE_RECENT_ROOTS), Some("/recent\n/older"));
        assert_eq!(prefs.get(STORAGE_PINNED_ROOTS), Some("/pinned"));
    }

    #[test]
    fn preferences_restore_from_safe_storage() {
        let dir = unique_temp_dir();
        let store = SafeStorage::new(&dir);
        let mut prefs = crate::storage::Preferences::default();
        prefs.set(STORAGE_PATH_INPUT, "/restored");
        prefs.set(STORAGE_EXCLUDE_INPUT, ".git,target");
        prefs.set(STORAGE_PROTECTED_PATHS, "/keep,/safe");
        prefs.set(STORAGE_INCLUDE_HIDDEN, "false");
        prefs.set(STORAGE_FOLLOW_SYMLINKS, "true");
        prefs.set(STORAGE_STAY_ON_FILESYSTEM, "true");
        prefs.set(STORAGE_SQLITE_CACHE, "true");
        prefs.set(STORAGE_SEARCH_FILTER, "true");
        prefs.set(STORAGE_COLOR_BY_EXTENSION, "true");
        prefs.set(STORAGE_REALTIME_WATCH, "false");
        prefs.set(STORAGE_RECENT_ROOTS, "/recent-a\n\n/recent-b\n/recent-a");
        prefs.set(STORAGE_PINNED_ROOTS, "/pinned-a\n/pinned-b\n/pinned-a");
        prefs.set(STORAGE_MAX_DEPTH, "99");
        prefs.set(STORAGE_THEME, "dark");
        prefs.set(STORAGE_LOCALE, "zh-Hans");
        store.write(&prefs).unwrap();

        let mut app = DiskMapApp {
            safe_storage: SafeStorage::new(&dir),
            ..DiskMapApp::default()
        };
        app.restore_preferences(&store.read());

        assert_eq!(app.path_input, "/restored");
        assert_eq!(app.exclude_input, ".git,target");
        assert_eq!(app.protected_paths_input, "/keep,/safe");
        assert!(!app.include_hidden);
        assert!(!app.follow_symlinks);
        assert!(app.stay_on_filesystem);
        assert!(!app.realtime_watch_enabled());
        assert!(!app.sqlite_cache_enabled);
        assert!(app.search_filter_enabled);
        assert!(app.color_by_extension);
        assert_eq!(app.recent_roots, vec!["/recent-a", "/recent-b"]);
        assert_eq!(app.pinned_roots, vec!["/pinned-a", "/pinned-b"]);
        assert_eq!(app.max_depth, 10);
        assert_eq!(app.theme_preference, Some(Theme::Dark));
        assert_eq!(app.locale, Locale::SimplifiedChinese);
        assert!(!app.locale_follow_system);
    }

    #[test]
    fn realtime_watch_preference_round_trips_through_safe_storage() {
        let dir = unique_temp_dir();
        let mut app = DiskMapApp {
            safe_storage: SafeStorage::new(&dir),
            ..Default::default()
        };
        app.set_realtime_watch_enabled(false);

        app.save_preferences();
        let mut restored = DiskMapApp {
            safe_storage: SafeStorage::new(&dir),
            ..Default::default()
        };
        restored.restore_preferences(&app.safe_storage.read());

        assert!(
            !restored.realtime_watch_enabled(),
            "disabled Watch must survive save/restore"
        );
    }

    #[test]
    fn local_state_restores_profiles_views_filter_presets_and_rules() {
        let dir = unique_temp_dir();
        let root = "/state-root";
        let mut app = DiskMapApp {
            safe_storage: SafeStorage::new(&dir),
            ..Default::default()
        };
        app.path_input = root.into();
        app.exclude_input = "target,.git".into();
        app.include_hidden = false;
        app.follow_symlinks = true;
        app.stay_on_filesystem = true;
        app.sqlite_cache_enabled = true;
        app.search_filter_enabled = true;
        app.color_by_extension = true;
        app.set_realtime_watch_enabled(false);
        app.save_current_as_profile(root);

        app.max_depth = 5;
        *app.search.input_mut() = "cache".into();
        app.last_report_mode = "rules".into();
        app.save_current_view(root);
        assert!(app.add_filter_preset("cache-filter"));
        assert!(app.set_rule_enabled("hidden-files", false));

        let state = app.safe_storage.read_state();
        let mut restored = DiskMapApp {
            safe_storage: SafeStorage::new(&dir),
            ..Default::default()
        };
        restored.restore_local_state(&state);

        assert_eq!(restored.path_input, root);
        let profile = restored
            .profiles
            .get(root)
            .expect("profile should restore from local state");
        assert_eq!(profile.exclude_patterns, vec!["target", ".git"]);
        assert!(!profile.include_hidden);
        assert!(!profile.follow_symlinks);
        assert!(profile.stay_on_filesystem);
        assert!(!profile.sqlite_cache_enabled);
        assert!(!profile.realtime_watch_enabled);

        let view = restored
            .views
            .get(root)
            .expect("view should restore from local state");
        assert_eq!(view.depth, 5);
        assert_eq!(view.search_query, "cache");
        assert_eq!(view.last_report_mode, "rules");

        let preset = restored
            .filter_presets
            .get("cache-filter")
            .expect("filter preset should restore from local state");
        assert_eq!(preset.query, "cache");
        assert!(preset.filter_enabled);
        assert!(!restored.rules.get("hidden-files").unwrap().enabled);
    }

    #[test]
    fn rules_import_preview_requires_confirmation_and_persists() {
        let dir = unique_temp_dir();
        let rules_path = dir.join("incoming-rules.json");
        let mut incoming = crate::rules::RuleSet::new();
        incoming.add(crate::rules::Rule {
            id: "incoming-hidden".into(),
            name: "Incoming Hidden".into(),
            description: "Imported hidden-file hint".into(),
            category: crate::rules::RuleCategory::AnomalyHint,
            predicate: crate::rules::RulePredicate::Hidden,
            enabled: false,
        });
        std::fs::write(&rules_path, crate::rules::export_ruleset_json(&incoming))
            .expect("incoming ruleset should be writable");

        let mut app = DiskMapApp {
            safe_storage: SafeStorage::new(&dir),
            ..Default::default()
        };
        assert!(app.rules.get("large-file-1gb").is_some());
        assert!(app.rules.get("incoming-hidden").is_none());

        app.rules_import_path = rules_path.display().to_string();
        app.preview_rules_import_from_input();

        let preview = app
            .pending_rules_import
            .as_ref()
            .expect("rules import should create a preview");
        assert_eq!(preview.incoming_rule_count, 1);
        assert_eq!(preview.incoming_enabled_count, 0);
        assert!(app.rules.get("incoming-hidden").is_none());

        assert!(app.confirm_rules_import());
        assert!(app.pending_rules_import.is_none());
        assert!(app.rules_import_path.is_empty());
        assert!(app.rules.get("incoming-hidden").is_some());
        assert!(app.rules.get("large-file-1gb").is_none());
        assert!(app
            .safe_storage
            .read_state()
            .rules
            .get("incoming-hidden")
            .is_some());
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
    fn start_scan_ignores_hidden_saved_profile_state() {
        let root = unique_temp_path("disk-map-profile-start");
        std::fs::create_dir_all(&root).expect("profile scan test root should be created");
        std::fs::write(root.join("keep.bin"), vec![1_u8; 20 * 1024])
            .expect("profile scan keep file should be written");
        std::fs::write(root.join("skip.bin"), vec![2_u8; 20 * 1024])
            .expect("profile scan skip file should be written");

        let root_key = root.display().to_string();
        let mut app = DiskMapApp::default();
        app.set_realtime_watch_enabled(false);
        app.exclude_input = "skip.bin".into();
        app.save_current_as_profile(&root_key);
        app.exclude_input.clear();

        app.start_scan_path(root.clone());
        assert!(drain_scan_for_test(&mut app), "scan should finish");

        let names = app
            .tree
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"keep.bin"));
        assert!(names.contains(&"skip.bin"));
        assert!(app.exclude_input.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn primary_workflow_smoke_covers_scan_navigation_search_export_watch_and_trash_confirmation() {
        let storage_dir = unique_temp_dir();
        let root = unique_temp_path("disk-map-smoke");
        let nested = root.join("nested");
        let file = nested.join("large-match.bin");
        std::fs::create_dir_all(&nested).expect("smoke test nested dir should be created");
        std::fs::write(&file, vec![9_u8; 20 * 1024]).expect("smoke test file should be written");

        let mut app = DiskMapApp {
            sqlite_cache_enabled: true,
            safe_storage: SafeStorage::new(&storage_dir),
            ..Default::default()
        };
        app.set_realtime_watch_enabled(false);

        app.start_scan_path(root.clone());
        assert!(drain_scan_for_test(&mut app), "smoke scan should finish");
        assert_eq!(app.scan_options().cache_mode, CacheMode::Disabled);
        assert!(app.scan_options().cache_path.is_none());
        app.set_realtime_watch_enabled(true);
        assert!(
            app.scan.watch_active(),
            "watcher should start for scanned temp root; status: {}",
            app.status.display_text()
        );

        let nested_id = app
            .tree
            .nodes
            .iter()
            .position(|node| node.name == "nested")
            .map(node_id_from_index)
            .expect("nested directory should be present");
        let file_id = app
            .tree
            .nodes
            .iter()
            .position(|node| node.name == "large-match.bin")
            .map(node_id_from_index)
            .expect("large file should be present");

        app.navigation.set_selected_id(Some(nested_id));
        assert!(app.enter_selected_directory());
        *app.search.input_mut() = "large-match".into();
        app.refresh_search_matches();
        app.navigate_search_match(SearchDirection::Next);
        assert_eq!(app.navigation.selected_id(), Some(file_id));

        app.export_focused_subtree(ExportFormat::Json);
        let exported_path = PathBuf::from(
            app.status
                .primary_text()
                .rsplit_once(" to ")
                .expect("export status should include output path")
                .1,
        );
        let exported =
            std::fs::read_to_string(&exported_path).expect("exported json should be readable");
        assert!(exported.contains("large-match.bin"));
        let _ = std::fs::remove_file(exported_path);

        app.queue_cleanup_candidate(file_id);
        app.arm_or_confirm_queued_trash(file_id);
        assert_eq!(app.trash_confirm_target_id, Some(file_id));
        assert!(app
            .status
            .display_text()
            .contains(&file.display().to_string()));

        app.stop_watching();
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(storage_dir);
    }

    #[test]
    fn successful_scan_keeps_finished_status_when_watch_starts() {
        let root = unique_temp_path("disk-map-watch-finished-status");
        std::fs::create_dir_all(&root).expect("watch test root should be created");
        std::fs::write(root.join("sample.bin"), vec![0_u8; 1024])
            .expect("watch test file should be written");

        let mut app = DiskMapApp::default();
        app.start_scan_path(root.clone());

        let deadline = Instant::now() + Duration::from_secs(5);
        while app.scan.is_scanning() && Instant::now() < deadline {
            app.handle_scan_messages();
            std::thread::sleep(Duration::from_millis(10));
        }
        app.handle_scan_messages();

        assert!(
            !app.scan.is_scanning(),
            "scan did not finish before timeout"
        );
        assert!(app.scan.watch_active());
        assert!(
            app.status.primary_text().starts_with("Finished:"),
            "unexpected status: {}",
            app.status.display_text()
        );

        app.stop_watching();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn successful_watch_restart_clears_stale_failure_status() {
        let root = unique_temp_path("disk-map-watch-recovery-status");
        std::fs::create_dir_all(&root).expect("watch test root should be created");

        let mut app = DiskMapApp::default();
        app.start_scan_path(root.clone());
        assert!(
            drain_scan_for_test(&mut app),
            "watch test scan should finish"
        );
        let finished_status = app.status.primary_text().to_string();
        app.set_realtime_watch_enabled(false);
        std::fs::remove_dir_all(&root).expect("watch test root should be removed");
        app.set_realtime_watch_enabled(true);
        assert!(app.status.has_watch_failure());

        std::fs::create_dir_all(&root).expect("watch test root should be restored");
        app.set_realtime_watch_enabled(false);
        app.set_realtime_watch_enabled(true);

        assert_eq!(app.status.display_text(), finished_status);
        app.stop_watching();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn successful_watch_restart_preserves_finished_status() {
        let root = unique_temp_path("disk-map-watch-finished-recovery");
        std::fs::create_dir_all(&root).expect("watch test root should be created");

        let mut app = DiskMapApp::default();
        app.start_scan_path(root.clone());
        assert!(
            drain_scan_for_test(&mut app),
            "watch test scan should finish"
        );
        app.set_realtime_watch_enabled(false);
        app.set_status(StatusSource::Scan, StatusLevel::Success, "Finished: 1 KiB");
        app.status.set_watch_failure("backend failed");

        app.set_realtime_watch_enabled(true);

        assert_eq!(app.status.display_text(), "Finished: 1 KiB");
        app.stop_watching();
        let _ = std::fs::remove_dir_all(root);
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
        assert!(app.status.primary_text().contains("1 candidate group"));
    }

    #[test]
    fn duplicate_analysis_is_unavailable_without_focused_root() {
        let mut app = DiskMapApp::default();

        app.analyze_duplicate_candidates();

        assert!(app.duplicate_report.is_none());
        assert_eq!(
            app.status.primary_text(),
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
        let layout_dirty = app.treemap.is_dirty();

        app.analyze_file_insights();

        let report = app.insight_report.as_ref().expect("insight report");
        assert_eq!(report.file_count, 2);
        assert_eq!(report.known_mtime_count, 1);
        assert!(report
            .type_summaries
            .iter()
            .any(|summary| summary.category == "Archives" && summary.extension == "zip"));
        assert_eq!(app.scan.active_id(), active_scan_id);
        assert_eq!(app.treemap.is_dirty(), layout_dirty);
        assert_eq!(app.status.primary_text(), "Insights analyzed 2 files");
    }

    #[test]
    fn insight_analysis_is_unavailable_without_focused_root() {
        let mut app = DiskMapApp::default();

        app.analyze_file_insights();

        assert!(app.insight_report.is_none());
        assert_eq!(
            app.status.primary_text(),
            "Insights unavailable: no focused directory"
        );
    }

    #[test]
    fn focused_report_metadata_captures_reproducible_view_state() {
        let mut app = app_with_search_matches();
        app.exclude_input = ".git,target".into();
        app.include_hidden = false;
        app.follow_symlinks = true;
        app.stay_on_filesystem = true;
        app.sqlite_cache_enabled = true;
        app.set_realtime_watch_enabled(true);
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
        assert!(!metadata.follow_symlinks);
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
        assert_eq!(app.status.primary_text(), "Pinned /root");

        app.toggle_pinned_root("/root");
        assert!(app.pinned_roots.is_empty());
        assert_eq!(app.recent_roots, vec!["/root"]);
        assert_eq!(app.status.primary_text(), "Unpinned /root");
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
    fn cleanup_queue_is_available_without_enable_toggle() {
        let mut app = app_with_search_matches();
        let root = unique_temp_path("disk-map-cleanup-queue");
        app.tree.set_root_path(root);

        app.queue_cleanup_candidate(2);

        assert!(app.trash_confirm_target_id.is_none());
        assert_eq!(app.cleanup_queue.len(), 1);
        assert!(app
            .status
            .primary_text()
            .starts_with("Queued cleanup candidate: "));
    }

    #[test]
    fn trash_action_queues_real_path_before_confirmation() {
        let mut app = app_with_search_matches();
        let root = unique_temp_path("disk-map-trash-queue-file");
        app.tree.set_root_path(root.clone());
        let active_scan_id = app.scan.active_id();

        app.queue_cleanup_candidate(2);

        assert_eq!(app.cleanup_queue.len(), 1);
        assert_eq!(
            app.cleanup_queue.candidates()[0].path,
            root.join("match-dir").join("match-file")
        );
        assert_eq!(app.cleanup_queue.candidates()[0].item_count, 1);
        assert_eq!(app.trash_confirm_target_id, None);
        assert_eq!(app.scan.active_id(), active_scan_id);
        assert!(app
            .status
            .primary_text()
            .starts_with("Queued cleanup candidate: "));
    }

    #[test]
    fn queued_trash_requires_confirmation_with_path_size_and_item_count() {
        let mut app = app_with_search_matches();
        let root = unique_temp_path("disk-map-trash-confirm");
        let dir = root.join("match-dir");
        let file = dir.join("match-file");
        std::fs::create_dir_all(&dir).expect("trash confirmation test dir should be created");
        std::fs::write(&file, b"x").expect("trash confirmation test file should be created");
        app.tree.set_root_path(root.clone());

        app.queue_cleanup_candidate(2);
        app.arm_or_confirm_queued_trash(2);

        assert_eq!(app.trash_confirm_target_id, Some(2));
        assert!(app
            .status
            .primary_text()
            .contains(&file.display().to_string()));
        assert!(app.status.primary_text().contains("1 B"));
        assert!(app.status.primary_text().contains("1 item"));

        std::fs::remove_dir_all(root).expect("trash confirmation test root should be removed");
    }

    #[test]
    fn direct_trash_moves_real_file_without_enable_toggle() {
        let mut app = app_with_search_matches();
        let root = unique_temp_path("disk-map-direct-trash");
        let dir = root.join("match-dir");
        let file = dir.join("match-file");
        std::fs::create_dir_all(&dir).expect("direct trash test dir should be created");
        std::fs::write(&file, b"x").expect("direct trash test file should be created");
        app.tree.set_root_path(root.clone());
        app.navigation.set_selected_id(Some(2));
        let active_scan_id = app.scan.active_id();

        app.move_node_to_trash(2);
        assert!(file.exists(), "first click should only arm confirmation");
        app.move_node_to_trash(2);

        assert!(!file.exists());
        assert_eq!(app.scan.active_id(), active_scan_id);
        assert_eq!(app.tree.node(0).size, 10);
        assert!(app.tree.node(1).children.is_empty());
        assert_eq!(app.tree.node(2).parent, None);
        assert_eq!(app.navigation.selected_id(), Some(1));
        assert!(app.status.primary_text().contains("Moved to Trash"));
        assert!(!app.status.primary_text().contains("Rescan to refresh"));

        std::fs::remove_dir_all(root).expect("direct trash test root should be removed");
    }

    #[test]
    fn cancelling_trash_confirmation_leaves_target_untouched() {
        let mut app = app_with_search_matches();
        let root = unique_temp_path("disk-map-cancel-trash");
        let dir = root.join("match-dir");
        let file = dir.join("match-file");
        std::fs::create_dir_all(&dir).expect("trash cancellation test dir should be created");
        std::fs::write(&file, b"x").expect("trash cancellation test file should be created");
        app.tree.set_root_path(root.clone());

        app.move_node_to_trash(2);
        app.clear_trash_confirmation();

        assert!(file.exists());
        assert!(app.trash_confirm_target_id.is_none());
        assert!(app.trash_confirm_path.is_none());

        std::fs::remove_dir_all(root).expect("trash cancellation test root should be removed");
    }

    #[test]
    fn trash_confirmation_keeps_escape_from_clearing_selection() {
        let mut app = app_with_search_matches();
        app.navigation.set_selected_id(Some(2));
        app.trash_confirm_target_id = Some(2);
        app.trash_confirm_path = Some(PathBuf::from("/root/match-dir/match-file"));

        let ctx = egui::Context::default();
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |_ui| app.handle_keyboard(&ctx),
        );

        assert_eq!(app.navigation.selected_id(), Some(2));
        assert_eq!(app.trash_confirm_target_id, Some(2));
    }

    #[test]
    fn localized_status_text_translates_dynamic_status_prefixes() {
        let mut app = DiskMapApp {
            locale: Locale::SimplifiedChinese,
            ..Default::default()
        };
        app.set_status(StatusSource::Scan, StatusLevel::Success, "Finished: 1 KiB");
        assert_eq!(app.localized_status_text(), "扫描完成: 1 KiB");

        app.status.set_watch_failure("backend failed");
        assert_eq!(
            app.localized_status_text(),
            "扫描完成: 1 KiB · 实时监视失败: backend failed"
        );
    }

    #[test]
    fn trash_action_queues_directories_with_affected_item_count() {
        let mut app = app_with_search_matches();
        let root = unique_temp_path("disk-map-trash-queue-dir");
        app.tree.set_root_path(root.clone());

        app.queue_cleanup_candidate(1);

        assert_eq!(app.cleanup_queue.len(), 1);
        let candidate = &app.cleanup_queue.candidates()[0];
        assert_eq!(candidate.path, root.join("match-dir"));
        assert_eq!(candidate.kind, NodeKind::Dir);
        assert_eq!(candidate.item_count, 2);
    }

    #[test]
    fn queued_trash_drops_missing_target_before_confirmation() {
        let mut app = app_with_search_matches();
        app.tree
            .set_root_path(unique_temp_path("disk-map-missing-trash-root"));
        let active_scan_id = app.scan.active_id();

        app.queue_cleanup_candidate(2);
        app.arm_or_confirm_queued_trash(2);

        assert_eq!(app.trash_confirm_target_id, None);
        assert!(app.cleanup_queue.is_empty());
        assert_eq!(app.scan.active_id(), active_scan_id);
        assert!(app
            .status
            .primary_text()
            .contains("target no longer exists"));
    }

    #[test]
    fn queued_trash_rechecks_user_protected_paths_before_confirmation() {
        let mut app = app_with_search_matches();
        let root = unique_temp_path("disk-map-trash-user-protected");
        app.tree.set_root_path(root.clone());

        app.queue_cleanup_candidate(2);
        app.protected_paths_input = root.join("match-dir").display().to_string();
        app.arm_or_confirm_queued_trash(2);

        assert_eq!(app.trash_confirm_target_id, None);
        assert_eq!(app.cleanup_queue.len(), 1);
        assert!(app.status.primary_text().contains("user protected path"));
    }

    #[test]
    fn trash_action_rejects_virtual_aggregate_nodes() {
        let mut app = DiskMapApp::default();
        let root = app.tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        app.tree.set_root_path("/root".into());
        let aggregate =
            app.tree
                .add_node(Some(root), "Other Files (2)".into(), NodeKind::Aggregate, 8);

        app.queue_cleanup_candidate(aggregate);

        assert_eq!(
            app.status.primary_text(),
            "Cleanup queue unavailable for virtual nodes"
        );
        assert!(app.trash_confirm_target_id.is_none());
        assert!(app.cleanup_queue.is_empty());
    }

    #[test]
    fn cleanup_queue_blocks_protected_paths() {
        let mut app = DiskMapApp::default();
        let root = app.tree.add_node(None, "root".into(), NodeKind::Dir, 0);
        app.tree.set_root_path("/".into());

        app.queue_cleanup_candidate(root);

        assert!(app
            .status
            .primary_text()
            .starts_with("Protected path blocked: / "));
        assert!(app.cleanup_queue.is_empty());
    }

    #[test]
    fn cleanup_queue_blocks_user_protected_paths() {
        let mut app = app_with_search_matches();
        let root = unique_temp_path("disk-map-cleanup-user-protected");
        app.tree.set_root_path(root.clone());
        app.protected_paths_input = root.join("match-dir").display().to_string();

        app.queue_cleanup_candidate(2);

        assert!(app.status.primary_text().contains(&format!(
            "Protected path blocked: {}",
            root.join("match-dir").join("match-file").display()
        )));
        assert!(app.status.primary_text().contains("user protected path"));
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
        assert!(!options.follow_symlinks);
        assert!(options.stay_on_filesystem);
    }

    #[test]
    fn watch_rescan_path_tracks_scan_root() {
        let mut app = app_with_search_matches();
        app.tree.set_root_path("/root".into());

        assert_eq!(app.scan_root_rescan_path(), Some(PathBuf::from("/root")));
    }

    #[test]
    pub(super) fn enter_selected_directory_focuses_selected_dir() {
        let mut app = app_with_search_matches();
        app.navigation.set_selected_id(Some(1));

        assert!(app.enter_selected_directory());
        assert_eq!(app.navigation.focused_root(), Some(1));
        assert_eq!(app.navigation.selected_id(), Some(1));
    }

    #[test]
    pub(super) fn enter_selected_directory_ignores_files() {
        let mut app = app_with_search_matches();
        app.navigation.set_selected_id(Some(2));

        assert!(!app.enter_selected_directory());
        assert_eq!(app.navigation.focused_root(), Some(0));
    }

    #[test]
    fn depth_keyboard_helpers_clamp_and_mark_layout_dirty() {
        let mut app = DiskMapApp {
            max_depth: 1,
            ..Default::default()
        };
        app.treemap.mark_clean_for_test();

        assert!(!app.decrease_depth());
        assert!(app.increase_depth());
        assert_eq!(app.max_depth, 2);
        assert!(app.treemap.is_dirty());

        app.max_depth = 10;
        app.treemap.mark_clean_for_test();
        assert!(!app.increase_depth());
        assert_eq!(app.max_depth, 10);
        assert!(!app.treemap.is_dirty());
    }

    #[test]
    fn platform_errors_update_status_without_starting_scan() {
        let mut app = app_for_scan(7);

        app.apply_platform_result("Open", Err(anyhow::anyhow!("boom")));

        assert_eq!(app.scan.active_id(), 7);
        assert!(!app.scan.has_handle());
        assert_eq!(app.status.primary_text(), "Open failed: boom");
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
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(ScanMessage::Error {
            scan_id: 1,
            message: "Path does not exist: /missing".into(),
            perf_stats: PerfStats::default(),
        });

        let error_message = app.no_root_state_message();
        assert_eq!(error_message.title, "Unable to scan path");
        assert_eq!(error_message.detail, "Path does not exist: /missing");

        let mut app = app_for_scan(2);
        app.apply_scan_message_for_test(ScanMessage::Cancelled {
            scan_id: 2,
            perf_stats: PerfStats::default(),
        });
        let cancelled_message = app.no_root_state_message();
        assert_eq!(cancelled_message.title, "Scan cancelled");
    }

    #[test]
    fn empty_root_state_message_reflects_scanning_and_finished_states() {
        let mut app = app_for_scan(1);
        app.path_input = "/root".into();
        app.apply_scan_message_for_test(root_started(1));

        let scanning_message = app.empty_root_state_message(0).expect("empty root state");
        assert_eq!(scanning_message.title, "Waiting for first results");
        assert!(scanning_message.detail.contains("/root"));

        app.apply_scan_message_for_test(finished(1, 0));

        let finished_message = app.empty_root_state_message(0).expect("empty root state");

        assert_eq!(finished_message.title, "Empty folder");
        assert!(finished_message.detail.contains("root"));
    }

    #[test]
    fn zero_byte_only_directory_has_an_explicit_finished_state() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));
        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                discovered_nodes: vec![DiscoveredNode {
                    node_id: 1,
                    parent_id: 0,
                    node: NodeRecord {
                        name: "empty.txt".into(),
                        kind: NodeKind::File,
                        size: 0,
                        modified_secs: None,
                        scanned: true,
                        error: None,
                    },
                }],
                size_deltas: vec![],
                scanned_nodes: vec![1],
                progress: None,
            },
        });
        app.apply_scan_message_for_test(finished(1, 0));

        let message = app
            .empty_root_state_message(0)
            .expect("zero-byte directory state");

        assert_eq!(message.title, "No disk space used");
        assert!(message.detail.contains("1 item"));
    }

    #[test]
    fn regular_file_root_has_a_file_state_instead_of_empty_folder() {
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(ScanMessage::Started {
            scan_id: 1,
            path: "/root/file.bin".into(),
            root_node: NodeRecord {
                name: "file.bin".into(),
                kind: NodeKind::File,
                size: 4096,
                modified_secs: None,
                scanned: true,
                error: None,
            },
        });

        let message = app
            .empty_root_state_message(0)
            .expect("single-file root state");

        assert_eq!(message.title, "File scanned");
        assert!(message.detail.contains("file.bin"));
        assert!(message.detail.contains(&format_bytes(4096)));
    }

    #[test]
    fn disabling_watch_clears_a_pending_follow_up_scan() {
        let mut app = DiskMapApp::default();

        app.set_realtime_watch_enabled(false);

        assert!(!app.realtime_watch_enabled());
        assert!(!app.scan.watch_active());
    }

    #[test]
    pub(super) fn return_to_scan_root_pushes_previous_focus_to_back_history() {
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
        app.treemap.mark_clean_for_test();

        app.return_to_scan_root();

        assert_eq!(app.navigation.focused_root(), Some(0));
        assert_eq!(app.navigation.selected_id(), Some(0));
        assert_eq!(app.navigation.back_history(), &[1]);
        assert!(app.navigation.forward_history().is_empty());
        assert!(app.treemap.is_dirty());
    }

    #[test]
    pub(super) fn return_to_scan_root_is_noop_when_already_at_scan_root() {
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
        let mut app = app_for_scan(1);
        app.apply_scan_message_for_test(root_started(1));

        app.apply_scan_message_for_test(ScanMessage::Batch {
            scan_id: 1,
            batch: ScanBatch {
                progress: Some(ProgressSnapshot {
                    files_scanned: 3,
                    total_files: Some(6),
                    dirs_scanned: 2,
                    bytes_seen: 128,
                    current_path: "/root/current/file.txt".into(),
                }),
                ..Default::default()
            },
        });

        let progress = app.scan.progress().expect("progress summary");
        assert_eq!(progress.files_scanned, 3);
        assert_eq!(progress.total_files, Some(6));
        assert_eq!(progress.file_progress_fraction(), Some(0.5));
        assert_eq!(progress.dirs_scanned, 2);
        assert_eq!(progress.bytes_seen, 128);
        assert_eq!(
            progress.current_path,
            PathBuf::from("/root/current/file.txt")
        );
        assert_eq!(app.status.primary_text(), "Scanning...");
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
    pub(super) fn clear_search_clears_active_match_cursor() {
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
