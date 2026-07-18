//! Sidebar section renderers for scan progress, scan issues, and the bottom
//! status bar.

use super::super::status::StatusLevel;
use super::super::DiskMapApp;
use super::super::Palette;
use super::super::StateMessage;
use crate::app::{palette, truncate_middle};
use crate::format::{format_bytes, format_duration};
use crate::i18n::{Locale, TextKey};
use crate::scanner::{size_basis_detail, size_basis_label};
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};

pub fn show_progress_section(ui: &mut egui::Ui, p: &Palette, app: &DiskMapApp) {
    let Some(progress) = app.scan.progress() else {
        return;
    };
    ui.add_space(12.0);
    ui.label(
        RichText::new(app.text(TextKey::Scan).to_uppercase())
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);
    let files_text = file_progress_label(app.locale, progress.files_scanned, progress.total_files);
    ui.label(
        RichText::new(format!(
            "{files_text} · {} {}",
            progress.dirs_scanned,
            app.text(TextKey::Dirs)
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

pub fn show_state_message(ui: &mut egui::Ui, p: &Palette, message: &StateMessage) {
    ui.label(
        RichText::new(message.title)
            .strong()
            .size(14.0)
            .color(p.text),
    );
    ui.add_space(4.0);
    ui.add(egui::Label::new(RichText::new(&message.detail).color(p.text_muted).small()).wrap());
}

pub fn show_scan_issue_section(ui: &mut egui::Ui, p: &Palette, app: &mut DiskMapApp) {
    let summary = app.scan.issue_summary();
    if !summary.has_findings() {
        return;
    }

    ui.add_space(12.0);
    ui.label(
        RichText::new(app.text(TextKey::ScanIssues).to_uppercase())
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);
    for (label, count, color) in [
        (TextKey::ErrorEntries, summary.error_entries, p.danger),
        (TextKey::SkippedPaths, summary.skipped_paths, p.text_muted),
        (
            TextKey::PermissionErrors,
            summary.permission_errors,
            p.danger,
        ),
        (TextKey::Symlinks, summary.symlinks, p.text_muted),
    ] {
        if count == 0 {
            continue;
        }
        ui.label(
            RichText::new(format!("{}: {count}", app.text(label)))
                .small()
                .color(color),
        );
    }

    #[cfg(target_os = "macos")]
    if summary.permission_errors > 0 {
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                RichText::new(app.text(TextKey::PermissionErrorsHint))
                    .small()
                    .color(p.text_muted),
            )
            .wrap(),
        );
        if ui.button(app.text(TextKey::OpenFullDiskAccess)).clicked() {
            app.open_full_disk_access_settings();
        }
    }
}

pub fn show_status_bar(ui: &mut egui::Ui, app: &DiskMapApp) {
    let p = palette(ui.ctx());
    let full_rect = ui.max_rect();
    ui.painter().line_segment(
        [full_rect.left_top(), full_rect.right_top()],
        Stroke::new(1.0, p.stroke_subtle),
    );

    ui.horizontal_centered(|ui| {
        ui.add_space(4.0);
        let dot_color =
            if app.status.has_watch_failure() || app.status.level() == StatusLevel::Error {
                p.danger
            } else if app.status.level() == StatusLevel::Progress {
                p.accent
            } else if app.status.level() == StatusLevel::Warning {
                p.text_faint
            } else {
                Color32::from_rgb(0x4A, 0xC4, 0x7A)
            };
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, dot_color);
        ui.label(
            RichText::new(app.localized_status_text())
                .size(11.5)
                .color(p.text_muted),
        );

        let elapsed_text = app.scan.elapsed().map(|elapsed| {
            format!(
                "{} {}",
                app.text(TextKey::Elapsed),
                format_duration(elapsed)
            )
        });
        if let Some(progress) = app.scan.progress() {
            if let Some(fraction) = progress.file_progress_fraction() {
                let available_width = ui.available_width();
                if available_width >= 120.0 {
                    let percent = (fraction * 100.0).round() as u64;
                    ui.add_space(8.0);
                    ui.add_sized(
                        [available_width.min(150.0), 16.0],
                        egui::ProgressBar::new(fraction).text(format!("{percent}%")),
                    );
                }
            }
        }
        if let Some(elapsed_text) = elapsed_text {
            if ui.available_width() >= 80.0 {
                ui.add_space(8.0);
                ui.label(RichText::new(elapsed_text).size(11.5).color(p.text_faint));
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            if !app.navigation.breadcrumb().is_empty() {
                let crumb = app.navigation.breadcrumb().replace(" / ", " › ");
                let display = truncate_middle(&crumb, 60);
                ui.label(
                    RichText::new(display)
                        .size(11.5)
                        .monospace()
                        .color(p.text_faint),
                );
            }

            if let Some(progress) = app.scan.progress() {
                ui.add_space(10.0);
                ui.label(RichText::new("│").size(11.0).color(p.text_faint));
                ui.add_space(10.0);
                let files_text =
                    file_progress_label(app.locale, progress.files_scanned, progress.total_files);
                let text = format!(
                    "{} · {} {} · {}",
                    files_text,
                    progress.dirs_scanned,
                    app.text(TextKey::Dirs),
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

// --- private helpers used only by the section renderers above ---

fn file_progress_label(locale: Locale, files_scanned: u64, total_files: Option<u64>) -> String {
    match total_files {
        Some(total_files) => format!(
            "{files_scanned}/{total_files} {}",
            locale.text(TextKey::Files)
        ),
        None => format!("{files_scanned} {}", locale.text(TextKey::Files)),
    }
}
