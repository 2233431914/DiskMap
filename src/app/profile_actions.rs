use super::DiskMapApp;
#[cfg(test)]
use crate::profiles::ScanProfile;
#[cfg(test)]
use crate::scanner::parse_exclude_patterns;

impl DiskMapApp {
    /// Apply a saved profile to the live UI fields. Does not change
    /// any in-flight scan; just overwrites the user-facing options.
    /// `start_scan_path` calls this before spawning the scanner so
    /// saved root options affect the scan being started.
    pub fn apply_profile_to_ui(&mut self, root: &str) {
        let Some(profile) = self.profiles.get(root).cloned() else {
            return;
        };
        self.exclude_input = profile.exclude_patterns.join(",");
        self.include_hidden = profile.include_hidden;
        self.follow_symlinks = false;
        self.stay_on_filesystem = profile.stay_on_filesystem;
        self.sqlite_cache_enabled = false;
        self.search_filter_enabled = profile.search_filter_enabled;
        self.color_by_extension = profile.color_by_extension;
        self.realtime_watch_enabled = profile.realtime_watch_enabled;
        self.status = format!("Applied profile for {root}");
        self.pending_repaint = true;
    }

    /// Save the current UI option values to the profile for `root`.
    /// Overwrites any existing profile for that key.
    #[cfg(test)]
    pub fn save_current_as_profile(&mut self, root: &str) {
        let profile = ScanProfile {
            exclude_patterns: parse_exclude_patterns(&self.exclude_input),
            include_hidden: self.include_hidden,
            follow_symlinks: false,
            stay_on_filesystem: self.stay_on_filesystem,
            sqlite_cache_enabled: false,
            search_filter_enabled: self.search_filter_enabled,
            color_by_extension: self.color_by_extension,
            realtime_watch_enabled: self.realtime_watch_enabled,
        };
        self.profiles.set(root, profile);
        self.status = format!("Saved profile for {root} ({} stored)", self.profiles.len());
        self.persist_local_state();
    }
}
