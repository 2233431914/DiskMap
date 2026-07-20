//! Read-only duplicate and file insight reports for the focused subtree.

use super::super::{icon_text_button, palette, section_divider, truncate_middle, DiskMapApp};
use crate::export::ExportFormat;
use crate::format::format_bytes;
use crate::i18n::TextKey;
use crate::insights::AgeBucket;
use crate::snapshot::{SnapshotChange, SnapshotKind};
use eframe::egui::{self, RichText};
use std::path::Path;

pub fn show(ui: &mut egui::Ui, app: &mut DiskMapApp) {
    let p = palette(ui.ctx());
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(app.text(TextKey::Analysis).to_uppercase())
                .size(10.0)
                .strong()
                .color(p.text_faint),
        );
        if app.active_report_view != super::super::ReportView::None
            && ui
                .button(egui_phosphor::regular::X)
                .on_hover_text(app.text(TextKey::Close))
                .clicked()
        {
            app.active_report_view = super::super::ReportView::None;
        }
    });
    ui.add_space(4.0);
    section_divider(ui, p);
    ui.add_space(6.0);

    let can_analyze = app.navigation.focused_root().is_some() && !app.scan.is_scanning();
    ui.horizontal_wrapped(|ui| {
        if ui
            .selectable_label(
                app.active_report_view == super::super::ReportView::Duplicates,
                app.text(TextKey::Duplicates),
            )
            .clicked()
        {
            app.analyze_duplicate_candidates();
        }
        if ui
            .selectable_label(
                app.active_report_view == super::super::ReportView::Insights,
                app.text(TextKey::Insights),
            )
            .clicked()
        {
            app.analyze_file_insights();
        }
        if ui
            .selectable_label(
                app.active_report_view == super::super::ReportView::Changes,
                app.text(TextKey::Changes),
            )
            .clicked()
        {
            app.update_snapshot_comparison();
            app.active_report_view = super::super::ReportView::Changes;
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let exporting_diff = app.active_report_view == super::super::ReportView::Changes;
        let can_export = if exporting_diff {
            app.snapshot_diff.is_some()
        } else {
            can_analyze
        };
        let csv = icon_text_button(
            ui,
            can_export,
            egui_phosphor::regular::DOWNLOAD_SIMPLE,
            app.text(TextKey::ExportCsv),
            126.0,
        )
        .on_hover_text(app.text(TextKey::ExportCsv));
        if csv.clicked() {
            if exporting_diff {
                app.export_snapshot_diff(ExportFormat::Csv);
            } else {
                app.export_focused_subtree(ExportFormat::Csv);
            }
        }
        let json = icon_text_button(
            ui,
            can_export,
            egui_phosphor::regular::DOWNLOAD_SIMPLE,
            app.text(TextKey::ExportJson),
            126.0,
        )
        .on_hover_text(app.text(TextKey::ExportJson));
        if json.clicked() {
            if exporting_diff {
                app.export_snapshot_diff(ExportFormat::Json);
            } else {
                app.export_focused_subtree(ExportFormat::Json);
            }
        }
    });

    match app.active_report_view {
        super::super::ReportView::Duplicates => show_duplicate_report(ui, app, p),
        super::super::ReportView::Insights => show_insight_report(ui, app, p),
        super::super::ReportView::Changes => show_changes_report(ui, app, p),
        super::super::ReportView::None if app.navigation.focused_root().is_none() => {
            ui.add_space(6.0);
            ui.label(
                RichText::new(app.text(TextKey::NoFocusedDirectory))
                    .small()
                    .color(p.text_faint),
            );
        }
        super::super::ReportView::None => {}
    }
}

fn show_changes_report(ui: &mut egui::Ui, app: &mut DiskMapApp, p: &super::super::Palette) {
    let Some(diff) = app.snapshot_diff.clone() else {
        ui.add_space(6.0);
        ui.label(
            RichText::new(app.text(TextKey::NoSnapshotBaseline))
                .small()
                .color(p.text_faint),
        );
        return;
    };

    show_scope(ui, app, &diff.root_path, p);
    ui.label(
        RichText::new(format!(
            "{}: {} · {}: {} · {}: {:+}",
            app.text(TextKey::PreviousSize),
            format_bytes(diff.previous_total),
            app.text(TextKey::CurrentSize),
            format_bytes(diff.current_total),
            app.text(TextKey::TotalDelta),
            diff.total_delta()
        ))
        .small()
        .color(p.text_muted),
    );
    if !diff.has_changes() {
        ui.add_space(6.0);
        ui.label(
            RichText::new(app.text(TextKey::NoChanges))
                .small()
                .color(p.text_faint),
        );
        return;
    }

    show_change_group(ui, app, p, TextKey::Added, &diff.added);
    show_change_group(ui, app, p, TextKey::Grown, &diff.grown);
    show_change_group(ui, app, p, TextKey::Shrunk, &diff.shrunk);
    show_change_group(ui, app, p, TextKey::Removed, &diff.removed);
}

fn show_change_group(
    ui: &mut egui::Ui,
    app: &mut DiskMapApp,
    p: &super::super::Palette,
    label: TextKey,
    changes: &[SnapshotChange],
) {
    if changes.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.label(RichText::new(app.text(label)).strong().color(p.text));
    for change in changes {
        let kind = snapshot_kind_label(app, change.kind);
        let row = format!(
            "{} · {}: {} · {}: {} · {}: {:+}",
            truncate_middle(&change.path, 30),
            app.text(TextKey::PreviousSize),
            format_bytes(change.previous_size),
            app.text(TextKey::CurrentSize),
            format_bytes(change.current_size),
            app.text(TextKey::Delta),
            change.delta
        );
        let response = ui
            .add_sized(
                [ui.available_width(), 30.0],
                egui::Button::new(RichText::new(row).small()),
            )
            .on_hover_text(format!("{} · {}", change.path, kind));
        if response.clicked() {
            app.focus_report_path(Path::new(&change.path));
        }
    }
}

fn snapshot_kind_label(app: &DiskMapApp, kind: SnapshotKind) -> &'static str {
    match kind {
        SnapshotKind::File => app.text(TextKey::File),
        SnapshotKind::Directory => app.text(TextKey::Directory),
        SnapshotKind::Symlink => app.text(TextKey::Symlink),
        SnapshotKind::Error => app.text(TextKey::ErrorEntry),
    }
}

fn show_duplicate_report(ui: &mut egui::Ui, app: &mut DiskMapApp, p: &super::super::Palette) {
    let Some(report) = app.duplicate_report.clone() else {
        return;
    };

    show_scope(ui, app, &report.root_path, p);
    ui.label(
        RichText::new(format!(
            "{}: {} · {}: {} · {}: {}",
            app.text(TextKey::CandidateGroups),
            report.group_count,
            app.text(TextKey::FileCount),
            report.file_count,
            app.text(TextKey::ReclaimableBytes),
            format_bytes(report.total_reclaimable_bytes)
        ))
        .small()
        .color(p.text_muted),
    );
    ui.add_space(4.0);
    if report.candidates.is_empty() {
        ui.label(
            RichText::new(app.text(TextKey::NoCandidates))
                .small()
                .color(p.text_faint),
        );
        return;
    }

    for candidate in report.candidates {
        let title = format!(
            "{} · {}: {} · {}: {} · {}: {}",
            candidate.name,
            app.text(TextKey::Size),
            format_bytes(candidate.size),
            app.text(TextKey::FileCount),
            candidate.paths.len(),
            app.text(TextKey::ReclaimableBytes),
            format_bytes(candidate.reclaimable_bytes)
        );
        egui::CollapsingHeader::new(title)
            .default_open(true)
            .show(ui, |ui| {
                for path in candidate.paths {
                    report_path_button(ui, app, &path, p);
                }
            });
    }
}

fn show_insight_report(ui: &mut egui::Ui, app: &mut DiskMapApp, p: &super::super::Palette) {
    let Some(report) = app.insight_report.clone() else {
        return;
    };

    show_scope(ui, app, &report.root_path, p);
    ui.label(
        RichText::new(format!(
            "{}: {} · {}: {} · {}: {}",
            app.text(TextKey::FileCount),
            report.file_count,
            app.text(TextKey::TotalSize),
            format_bytes(report.total_size),
            app.text(TextKey::KnownModifiedTimes),
            report.known_mtime_count
        ))
        .small()
        .color(p.text_muted),
    );

    ui.add_space(6.0);
    ui.label(
        RichText::new(app.text(TextKey::TypeSummary))
            .strong()
            .color(p.text),
    );
    for summary in report.type_summaries {
        ui.label(
            RichText::new(format!(
                "{} · .{} · {}: {} · {}: {}",
                app.locale.category_label(&summary.category),
                summary.extension,
                app.text(TextKey::FileCount),
                summary.file_count,
                app.text(TextKey::TotalSize),
                format_bytes(summary.total_size)
            ))
            .small()
            .color(p.text_muted),
        );
    }

    ui.add_space(6.0);
    ui.label(
        RichText::new(app.text(TextKey::AgeSummary))
            .strong()
            .color(p.text),
    );
    for bucket in report.age_buckets {
        ui.label(
            RichText::new(format!(
                "{} · {}: {} · {}: {}",
                age_bucket_label(app, bucket.bucket),
                app.text(TextKey::FileCount),
                bucket.file_count,
                app.text(TextKey::TotalSize),
                format_bytes(bucket.total_size)
            ))
            .small()
            .color(p.text_muted),
        );
    }

    ui.add_space(6.0);
    ui.label(
        RichText::new(app.text(TextKey::OldLargeFiles))
            .strong()
            .color(p.text),
    );
    if report.old_large_files.is_empty() {
        ui.label(
            RichText::new(app.text(TextKey::NoCandidates))
                .small()
                .color(p.text_faint),
        );
    } else {
        for file in report.old_large_files {
            report_path_button(ui, app, &file.path, p);
        }
    }
}

fn show_scope(ui: &mut egui::Ui, app: &DiskMapApp, path: &Path, p: &super::super::Palette) {
    ui.label(
        RichText::new(format!(
            "{}: {}",
            app.text(TextKey::ReportScope),
            truncate_middle(&path.display().to_string(), 48)
        ))
        .small()
        .monospace()
        .color(p.text_faint),
    )
    .on_hover_text(path.display().to_string());
}

fn report_path_button(
    ui: &mut egui::Ui,
    app: &mut DiskMapApp,
    path: &str,
    p: &super::super::Palette,
) {
    let label = truncate_middle(path, 46);
    let response = ui
        .add_sized(
            [ui.available_width(), 26.0],
            egui::Button::new(
                RichText::new(format!("{}  {label}", egui_phosphor::regular::ARROW_RIGHT))
                    .small()
                    .monospace()
                    .color(p.text_muted),
            ),
        )
        .on_hover_text(path);
    if response.clicked() {
        app.focus_report_path(Path::new(path));
    }
}

fn age_bucket_label(app: &DiskMapApp, bucket: AgeBucket) -> &'static str {
    match bucket {
        AgeBucket::Last30Days => app.text(TextKey::Last30Days),
        AgeBucket::Days31To180 => app.text(TextKey::Days31To180),
        AgeBucket::Days181To365 => app.text(TextKey::Days181To365),
        AgeBucket::OlderThan365 => app.text(TextKey::OlderThan365),
        AgeBucket::Unknown => app.text(TextKey::UnknownAge),
    }
}
