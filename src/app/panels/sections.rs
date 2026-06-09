//! Sidebar section renderers (progress, scan issues, cleanup queue,
//! snapshot diff, duplicate report, insight report) plus the bottom status
//! bar. Extracted from `app.rs` so each section reads top-to-bottom.
//!
//! Each section is a free function that takes `&DiskMapApp` (or `&mut` for
//! sections that mutate cleanup-queue state) and the egui `Ui` plus a
//! `&Palette` for theme colors. The owning `app.rs` impl method is a
//! one-line forwarder.

use super::super::DiskMapApp;
use super::super::Palette;
use super::super::StateMessage;
use crate::app::{describe_node_kind, palette, pluralize, truncate_middle};
use crate::cleanup::CleanupCandidate;
use crate::duplicates::DuplicateCandidate;
use crate::format::{format_bytes, format_duration};
use crate::insights::{AgeBucketSummary, FileTypeSummary, OldLargeFile};
use crate::scanner::{size_basis_detail, size_basis_label};
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};

pub fn show_progress_section(ui: &mut egui::Ui, p: &Palette, app: &DiskMapApp) {
    let Some(progress) = app.scan.progress() else {
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
    let files_text = file_progress_label(progress.files_scanned, progress.total_files);
    ui.label(
        RichText::new(format!("{files_text} · {} dirs", progress.dirs_scanned))
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

pub fn show_scan_issue_section(ui: &mut egui::Ui, p: &Palette, app: &DiskMapApp) {
    let summary = app.scan.issue_summary();
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

pub fn show_cleanup_queue_section(ui: &mut egui::Ui, p: &Palette, app: &mut DiskMapApp) {
    if app.cleanup_queue.is_empty() {
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
            pluralize(app.cleanup_queue.len() as u64, "candidate", "candidates"),
            format_bytes(app.cleanup_queue.total_size())
        ))
        .small()
        .color(p.text_muted),
    );

    let candidates = app.cleanup_queue.candidates().to_vec();
    for candidate in candidates {
        cleanup_candidate_row(ui, p, &candidate);
        ui.columns(2, |cols| {
            let w0 = cols[0].available_width();
            if cols[0]
                .add(
                    egui::Button::new(if app.trash_confirm_target_id == Some(candidate.node_id) {
                        "Confirm Trash"
                    } else {
                        "Trash"
                    })
                    .min_size(Vec2::new(w0, 24.0)),
                )
                .clicked()
            {
                app.arm_or_confirm_queued_trash(candidate.node_id);
            }
            let w1 = cols[1].available_width();
            if cols[1]
                .add(egui::Button::new("Remove").min_size(Vec2::new(w1, 24.0)))
                .clicked()
            {
                app.remove_cleanup_candidate(candidate.node_id);
            }
        });
        ui.add_space(4.0);
    }
}

pub fn show_snapshot_diff_section(ui: &mut egui::Ui, p: &Palette, app: &DiskMapApp) {
    let Some(diff) = &app.snapshot_diff else {
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

pub fn show_duplicate_report_section(ui: &mut egui::Ui, p: &Palette, app: &DiskMapApp) {
    let Some(report) = &app.duplicate_report else {
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

pub fn show_insight_report_section(ui: &mut egui::Ui, p: &Palette, app: &DiskMapApp) {
    let Some(report) = &app.insight_report else {
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

pub fn show_diagnostics_section(ui: &mut egui::Ui, p: &Palette, app: &mut DiskMapApp) {
    ui.add_space(12.0);
    ui.label(
        RichText::new("DIAGNOSTICS")
            .size(10.0)
            .strong()
            .color(p.text_faint),
    );
    ui.add_space(4.0);
    let error_count = app.recent_errors.len();
    if error_count > 0 {
        ui.label(
            RichText::new(format!("{} recent error(s) recorded", error_count))
                .small()
                .color(p.danger),
        );
    } else {
        ui.label(
            RichText::new("No recent errors")
                .small()
                .color(p.text_muted),
        );
    }
    let button_width = ui.available_width();
    if ui
        .add(
            egui::Button::new("Export Diagnostics Bundle")
                .min_size(Vec2::new(button_width, 28.0)),
        )
        .on_hover_text("Write a snapshot of app state, scan options, perf stats, and recent errors to disk-map-diagnostics-<ts>/ in the current directory. Paths are redacted (home -> ~).")
        .clicked()
    {
        let dest = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        match app.export_diagnostics(&dest) {
            Ok(path) => {
                app.status = format!("Wrote diagnostics: {}", path.display());
            }
            Err(error) => {
                app.record_error(format!("diagnostics export failed: {error}"));
                app.status = format!("Diagnostics export failed: {error}");
            }
        }
        app.pending_repaint = true;
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
        let dot_color = if app.status.starts_with("Error") {
            p.danger
        } else if app.scan.is_scanning() {
            p.accent
        } else if app.status.starts_with("Cancel") {
            p.text_faint
        } else {
            Color32::from_rgb(0x4A, 0xC4, 0x7A)
        };
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, dot_color);
        ui.label(RichText::new(&app.status).size(11.5).color(p.text_muted));

        let elapsed_text = app
            .scan
            .elapsed()
            .map(|elapsed| format!("Elapsed {}", format_duration(elapsed)));
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
                let files_text = file_progress_label(progress.files_scanned, progress.total_files);
                let text = format!(
                    "{} · {} dirs · {}",
                    files_text,
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

// --- private helpers used only by the section renderers above ---

fn file_progress_label(files_scanned: u64, total_files: Option<u64>) -> String {
    match total_files {
        Some(total_files) => format!("{files_scanned}/{total_files} files"),
        None => format!("{files_scanned} files"),
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

fn snapshot_change_group(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    changes: &[crate::snapshot::SnapshotChange],
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
    for summary in summaries.iter().take(crate::insights::INSIGHT_REPORT_LIMIT) {
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
    for file in files.iter().take(crate::insights::INSIGHT_REPORT_LIMIT) {
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

fn format_signed_bytes(delta: i128) -> String {
    if delta >= 0 {
        format!("+{}", format_bytes(delta as u64))
    } else {
        format!("-{}", format_bytes(delta.unsigned_abs() as u64))
    }
}
