use super::{current_unix_secs, DiskMapApp};
use crate::export::{export_focused_report, export_subtree, ExportFormat, FocusedReportMetadata};
use crate::scanner::{parse_exclude_patterns, size_basis_label};
use crate::tree::NodeId;
use std::path::PathBuf;

impl DiskMapApp {
    pub(super) fn export_focused_subtree(&mut self, format: ExportFormat) {
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

    pub(super) fn export_scan_root(&mut self, format: ExportFormat) {
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

    pub(super) fn export_focused_report_json(&mut self) {
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
            follow_symlinks: self.follow_symlinks,
            stay_on_filesystem: self.stay_on_filesystem,
            sqlite_cache_enabled: self.sqlite_cache_enabled,
            realtime_watch_enabled: self.realtime_watch_enabled,
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

fn default_report_path() -> PathBuf {
    let timestamp = current_unix_secs();
    PathBuf::from(format!("disk-map-report-{timestamp}.json"))
}
