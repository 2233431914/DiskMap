use super::{current_unix_secs, DiskMapApp, StatusLevel, StatusSource};
#[cfg(test)]
use crate::export::FocusedReportMetadata;
use crate::export::{export_snapshot_diff, export_subtree, ExportFormat};
use crate::i18n::TextKey;
#[cfg(test)]
use crate::scanner::{parse_exclude_patterns, size_basis_label};
use crate::tree::NodeId;
use std::path::PathBuf;

impl DiskMapApp {
    pub(super) fn export_focused_subtree(&mut self, format: ExportFormat) {
        let Some(root_id) = self.navigation.focused_root() else {
            self.set_status(
                StatusSource::Export,
                StatusLevel::Warning,
                self.text(TextKey::ExportUnavailable),
            );
            self.pending_repaint = true;
            return;
        };

        match self.write_focused_export(root_id, format) {
            Ok(path) => {
                self.set_status(
                    StatusSource::Export,
                    StatusLevel::Success,
                    format!(
                        "{} {} to {}",
                        self.text(TextKey::Exported),
                        format.label(),
                        path.display()
                    ),
                );
            }
            Err(error) => {
                self.set_status(
                    StatusSource::Export,
                    StatusLevel::Error,
                    format!("{}: {error}", self.text(TextKey::ExportFailed)),
                );
            }
        }
        self.pending_repaint = true;
    }

    pub(super) fn export_snapshot_diff(&mut self, format: ExportFormat) {
        let Some(diff) = self.snapshot_diff.clone() else {
            self.set_status(
                StatusSource::Export,
                StatusLevel::Warning,
                self.text(TextKey::NoSnapshotBaseline),
            );
            self.pending_repaint = true;
            return;
        };

        let output_path = default_export_path(format);
        match std::fs::write(&output_path, export_snapshot_diff(&diff, format)) {
            Ok(()) => self.set_status(
                StatusSource::Export,
                StatusLevel::Success,
                format!(
                    "{} {} to {}",
                    self.text(TextKey::Exported),
                    format.label(),
                    output_path.display()
                ),
            ),
            Err(error) => self.set_status(
                StatusSource::Export,
                StatusLevel::Error,
                format!("{}: {error}", self.text(TextKey::ExportFailed)),
            ),
        }
        self.pending_repaint = true;
    }

    fn write_focused_export(
        &mut self,
        root_id: NodeId,
        format: ExportFormat,
    ) -> anyhow::Result<PathBuf> {
        if !self.tree.contains_id(root_id) {
            anyhow::bail!("focused directory is no longer available");
        }

        let content = export_subtree(&mut self.tree, root_id, format);
        let output_path = default_export_path(format);
        std::fs::write(&output_path, content)?;
        Ok(output_path)
    }

    #[cfg(test)]
    pub(super) fn focused_report_metadata(
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
            follow_symlinks: false,
            stay_on_filesystem: self.stay_on_filesystem,
            sqlite_cache_enabled: self.sqlite_cache_enabled,
            realtime_watch_enabled: self.realtime_watch_enabled(),
            exclude_patterns: parse_exclude_patterns(&self.exclude_input),
        })
    }
}

fn default_export_path(format: ExportFormat) -> PathBuf {
    let timestamp = current_unix_secs();
    PathBuf::from(format!(
        "disk-map-export-{timestamp}.{}",
        format.extension()
    ))
}
