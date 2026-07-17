use super::{current_unix_secs, pluralize, DiskMapApp, StatusLevel, StatusSource};
use crate::duplicates::find_duplicate_candidates;
use crate::insights::analyze_insights;

const DUPLICATE_REPORT_LIMIT: usize = 8;

impl DiskMapApp {
    pub(crate) fn analyze_duplicate_candidates(&mut self) {
        self.last_report_mode = "duplicates".to_string();
        let Some(root_id) = self.navigation.focused_root() else {
            self.duplicate_report = None;
            self.set_status(
                StatusSource::Analysis,
                StatusLevel::Warning,
                "Duplicate analysis unavailable: no focused directory",
            );
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
                self.set_status(StatusSource::Analysis, StatusLevel::Success, status);
            }
            None => {
                self.duplicate_report = None;
                self.set_status(
                    StatusSource::Analysis,
                    StatusLevel::Warning,
                    "Duplicate analysis unavailable for this view",
                );
            }
        }
        self.pending_repaint = true;
    }

    pub(crate) fn analyze_file_insights(&mut self) {
        self.last_report_mode = "insights".to_string();
        let Some(root_id) = self.navigation.focused_root() else {
            self.insight_report = None;
            self.set_status(
                StatusSource::Analysis,
                StatusLevel::Warning,
                "Insights unavailable: no focused directory",
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
                    format!(
                        "Insights analyzed {}",
                        pluralize(report.file_count as u64, "file", "files")
                    ),
                );
                self.insight_report = Some(report);
            }
            None => {
                self.insight_report = None;
                self.set_status(
                    StatusSource::Analysis,
                    StatusLevel::Warning,
                    "Insights unavailable for this view",
                );
            }
        }
        self.pending_repaint = true;
    }
}
