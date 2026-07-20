use super::{current_unix_secs, DiskMapApp, ReportView, StatusLevel, StatusSource};
use crate::duplicates::find_duplicate_candidates;
use crate::i18n::TextKey;
use crate::insights::analyze_insights;

const DUPLICATE_REPORT_LIMIT: usize = 8;

impl DiskMapApp {
    pub(crate) fn analyze_duplicate_candidates(&mut self) {
        self.active_report_view = ReportView::Duplicates;
        #[cfg(test)]
        {
            self.last_report_mode = "duplicates".to_string();
        }
        let Some(root_id) = self.navigation.focused_root() else {
            self.duplicate_report = None;
            self.set_status(
                StatusSource::Analysis,
                StatusLevel::Warning,
                self.text(TextKey::NoFocusedDirectory),
            );
            self.pending_repaint = true;
            return;
        };

        match find_duplicate_candidates(&mut self.tree, root_id, DUPLICATE_REPORT_LIMIT) {
            Some(report) => {
                let status = if report.group_count == 0 {
                    self.text(TextKey::NoCandidates).to_string()
                } else {
                    format!(
                        "{}: {}",
                        self.text(TextKey::CandidateGroups),
                        report.group_count
                    )
                };
                self.duplicate_report = Some(report);
                self.set_status(StatusSource::Analysis, StatusLevel::Success, status);
            }
            None => {
                self.duplicate_report = None;
                self.set_status(
                    StatusSource::Analysis,
                    StatusLevel::Warning,
                    self.text(TextKey::ReportUnavailable),
                );
            }
        }
        self.pending_repaint = true;
    }

    pub(crate) fn analyze_file_insights(&mut self) {
        self.active_report_view = ReportView::Insights;
        #[cfg(test)]
        {
            self.last_report_mode = "insights".to_string();
        }
        let Some(root_id) = self.navigation.focused_root() else {
            self.insight_report = None;
            self.set_status(
                StatusSource::Analysis,
                StatusLevel::Warning,
                self.text(TextKey::NoFocusedDirectory),
            );
            self.pending_repaint = true;
            return;
        };

        match analyze_insights(
            &mut self.tree,
            root_id,
            current_unix_secs(),
            crate::insights::INSIGHT_REPORT_LIMIT,
        ) {
            Some(report) => {
                self.set_status(
                    StatusSource::Analysis,
                    StatusLevel::Success,
                    format!("{}: {}", self.text(TextKey::FileCount), report.file_count),
                );
                self.insight_report = Some(report);
            }
            None => {
                self.insight_report = None;
                self.set_status(
                    StatusSource::Analysis,
                    StatusLevel::Warning,
                    self.text(TextKey::ReportUnavailable),
                );
            }
        }
        self.pending_repaint = true;
    }
}
