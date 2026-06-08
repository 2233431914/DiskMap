//! Crash-safe local storage for app preferences and small local state.
//!
//! eframe's `Storage` trait only exposes a key-value API; the underlying
//! file write is implementation-defined and is **not** guaranteed atomic
//! against a SIGKILL mid-save. This module provides a thin wrapper for
//! the app's small local state JSON in the app data directory using a
//! write-to-temp + fsync + rename pattern, so a crash can leave either
//! the old file intact or the new file intact — never an empty or
//! truncated half-written file.
//!
//! Window size / position (managed by eframe's `persist_window: true`)
//! is left to eframe's own storage since it's small and infrequently
//! written.

use crate::profiles::ProfileStore;
use crate::rules::{default_ruleset, RuleSet};
use crate::views::{FilterStore, ViewStore};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const LOCAL_STATE_VERSION: u32 = 1;
const PREFERENCES_FILENAME: &str = "disk-map-prefs.json";
const PREFERENCES_TMP_FILENAME: &str = "disk-map-prefs.json.tmp";

/// Backing store for all `STORAGE_*` user preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Preferences {
    /// BTreeMap (not HashMap) for deterministic on-disk ordering — makes
    /// diffs and corruption recovery easier to read.
    #[serde(default)]
    pub values: BTreeMap<String, String>,
}

impl Preferences {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.values.insert(key.into(), value.into());
    }
}

/// Versioned local state kept in the crash-safe file.
///
/// This intentionally stores only compact user state. Full scan trees,
/// snapshots, history, and audit logs remain out of scope until their
/// crash-safe stores are designed separately.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalState {
    #[serde(default = "current_local_state_version")]
    pub version: u32,
    #[serde(default)]
    pub preferences: Preferences,
    #[serde(default)]
    pub profiles: ProfileStore,
    #[serde(default)]
    pub views: ViewStore,
    #[serde(default)]
    pub filter_presets: FilterStore,
    #[serde(default = "default_ruleset_for_storage")]
    pub rules: RuleSet,
}

impl Default for LocalState {
    fn default() -> Self {
        Self {
            version: LOCAL_STATE_VERSION,
            preferences: Preferences::default(),
            profiles: ProfileStore::new(),
            views: ViewStore::new(),
            filter_presets: FilterStore::new(),
            rules: default_ruleset_for_storage(),
        }
    }
}

impl LocalState {
    pub fn from_preferences(preferences: Preferences) -> Self {
        Self {
            preferences,
            ..Self::default()
        }
    }
}

fn current_local_state_version() -> u32 {
    LOCAL_STATE_VERSION
}

fn default_ruleset_for_storage() -> RuleSet {
    default_ruleset()
}

/// Filesystem-backed store for `LocalState`. Writes are crash-safe.
#[derive(Debug, Clone)]
pub struct SafeStorage {
    path: PathBuf,
    tmp_path: PathBuf,
}

impl SafeStorage {
    /// Construct a store that reads/writes `<app_data_dir>/disk-map-prefs.json`.
    pub fn new(app_data_dir: &Path) -> Self {
        Self {
            path: app_data_dir.join(PREFERENCES_FILENAME),
            tmp_path: app_data_dir.join(PREFERENCES_TMP_FILENAME),
        }
    }

    /// Final on-disk path. Useful for diagnostics output.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Atomically write preferences while preserving any other small
    /// local state already on disk. Prefer `write_state` when the
    /// caller has the whole app state available.
    pub fn write(&self, prefs: &Preferences) -> Result<()> {
        let mut state = self.read_state();
        state.preferences = prefs.clone();
        self.write_state(&state)
    }

    /// Atomically write `state` to disk. The pattern is:
    /// 1. Serialize to JSON.
    /// 2. Write to `disk-map-prefs.json.tmp` and `sync_all()` (fsync).
    /// 3. `rename` over the final file — atomic on POSIX.
    ///
    /// On crash between any two steps, the previous on-disk file remains
    /// intact and the temp file may be partially written. The next read
    /// falls back to the previous file (see `read`).
    pub fn write_state(&self, state: &LocalState) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).context("create storage dir")?;
        }
        let json = serde_json::to_string_pretty(state).context("serialize local state")?;
        {
            let mut f = fs::File::create(&self.tmp_path)
                .with_context(|| format!("create temp file at {}", self.tmp_path.display()))?;
            f.write_all(json.as_bytes()).context("write to temp file")?;
            f.sync_all().context("fsync temp file")?;
        }
        // fsync the directory so the rename is durable. Best-effort —
        // some filesystems (e.g. some FUSE) don't support it; ignore the
        // error rather than fail the whole write.
        if let Some(parent) = self.path.parent() {
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        fs::rename(&self.tmp_path, &self.path)
            .with_context(|| format!("atomic rename to {}", self.path.display()))?;
        Ok(())
    }

    /// Read preferences from disk with crash-recovery fallback. This
    /// preserves the original preferences-only interface for callers
    /// and tests that don't need the full local state.
    pub fn read(&self) -> Preferences {
        self.read_state().preferences
    }

    /// Read local state from disk with crash-recovery fallback.
    /// Returns default state if both the main file and the temp file
    /// are missing, empty, corrupt, or from an unsupported version.
    pub fn read_state(&self) -> LocalState {
        if let Some(state) = Self::read_state_from(&self.path) {
            return state;
        }
        if let Some(state) = Self::read_state_from(&self.tmp_path) {
            return state;
        }
        LocalState::default()
    }

    fn read_state_from(path: &Path) -> Option<LocalState> {
        let text = fs::read_to_string(path).ok()?;
        if text.trim().is_empty() {
            return None;
        }
        let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
        if value.get("version").is_some() || value.get("preferences").is_some() {
            let state = serde_json::from_value::<LocalState>(value).ok()?;
            if state.version == LOCAL_STATE_VERSION {
                Some(state)
            } else {
                None
            }
        } else {
            let preferences = serde_json::from_value::<Preferences>(value).ok()?;
            Some(LocalState::from_preferences(preferences))
        }
    }
}

/// Best-effort resolution of the per-user app data directory.
///
/// On macOS, this is `$HOME/Library/Application Support/<app_id>`. We
/// intentionally do not use the `dirs` crate to keep the dependency
/// surface small — the layout is stable on macOS (the only target
/// that matters for this app).
pub fn app_data_dir(app_id: &str) -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let path = PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join(app_id);
        return path;
    }
    PathBuf::from("/tmp").join(app_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{ProfileStore, ScanProfile};
    use crate::rules::{Rule, RuleCategory, RulePredicate, RuleSet};
    use crate::views::{FilterPreset, FilterStore, ViewState, ViewStore};

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let p = std::env::temp_dir().join(format!("disk-map-storage-test-{pid}-{nanos}-{n}"));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn roundtrip_writes_and_reads_preferences() {
        let dir = temp_dir();
        let store = SafeStorage::new(&dir);
        let mut prefs = Preferences::default();
        prefs.set("path_input", "/Users/me");
        prefs.set("include_hidden", "true");
        prefs.set("exclude_input", ".git,target");

        store.write(&prefs).unwrap();
        let loaded = store.read();

        assert_eq!(loaded, prefs);
    }

    #[test]
    fn read_returns_default_when_no_files_exist() {
        let dir = temp_dir();
        let store = SafeStorage::new(&dir);
        let loaded = store.read();
        assert_eq!(loaded, Preferences::default());
    }

    #[test]
    fn read_falls_back_to_temp_when_main_file_is_missing() {
        let dir = temp_dir();
        let store = SafeStorage::new(&dir);
        let mut prefs = Preferences::default();
        prefs.set("path_input", "/from-temp");
        let json = serde_json::to_string_pretty(&prefs).unwrap();
        // Simulate: main file is missing, temp file is intact (i.e. we
        // crashed between fsync and rename).
        fs::write(&store.tmp_path, &json).unwrap();

        let loaded = store.read();
        assert_eq!(loaded.get("path_input"), Some("/from-temp"));
    }

    #[test]
    fn read_falls_back_to_temp_when_main_file_is_corrupt() {
        let dir = temp_dir();
        let store = SafeStorage::new(&dir);
        let mut prefs = Preferences::default();
        prefs.set("path_input", "/recovered");
        let json = serde_json::to_string_pretty(&prefs).unwrap();
        // Simulate: main file is half-written (truncated mid-save, before
        // we had atomic rename). Temp file is intact.
        fs::write(&store.path, "{ this is not valid json").unwrap();
        fs::write(&store.tmp_path, &json).unwrap();

        let loaded = store.read();
        assert_eq!(loaded.get("path_input"), Some("/recovered"));
    }

    #[test]
    fn read_uses_main_when_both_files_present() {
        // When both files exist, the main file wins (it's the canonical
        // state after a successful rename). Temp is only a fallback.
        let dir = temp_dir();
        let store = SafeStorage::new(&dir);
        let mut main_prefs = Preferences::default();
        main_prefs.set("path_input", "/from-main");
        let mut tmp_prefs = Preferences::default();
        tmp_prefs.set("path_input", "/from-temp");
        store.write(&main_prefs).unwrap();
        fs::write(
            &store.tmp_path,
            serde_json::to_string_pretty(&tmp_prefs).unwrap(),
        )
        .unwrap();

        let loaded = store.read();
        assert_eq!(loaded.get("path_input"), Some("/from-main"));
    }

    #[test]
    fn second_write_does_not_leave_temp_file() {
        // After a successful write, the temp file should not remain
        // (rename moved it). Verifies we don't accumulate junk temp files.
        let dir = temp_dir();
        let store = SafeStorage::new(&dir);
        let mut prefs = Preferences::default();
        prefs.set("a", "1");
        store.write(&prefs).unwrap();
        assert!(!store.tmp_path.exists(), "temp should be gone after rename");
    }

    #[test]
    fn write_creates_missing_parent_directory() {
        let dir = temp_dir();
        let nested = dir.join("a").join("b").join("c");
        let store = SafeStorage::new(&nested);
        let mut prefs = Preferences::default();
        prefs.set("x", "1");
        store.write(&prefs).unwrap();
        assert!(store.path.exists());
    }

    #[test]
    fn local_state_round_trips_small_user_state() {
        let dir = temp_dir();
        let store = SafeStorage::new(&dir);

        let mut preferences = Preferences::default();
        preferences.set("path_input", "/state");

        let mut profiles = ProfileStore::new();
        profiles.set("/state", ScanProfile::default());

        let mut views = ViewStore::new();
        views.set(
            "/state",
            ViewState {
                depth: 4,
                search_query: "cache".into(),
                search_filter_enabled: true,
                color_by_extension: true,
                last_report_mode: "rules".into(),
                focused_id: Some(1),
                selected_id: Some(2),
            },
        );

        let mut filter_presets = FilterStore::new();
        assert!(filter_presets.add(FilterPreset {
            name: "logs".into(),
            query: ".log".into(),
            filter_enabled: true,
        }));

        let mut rules = RuleSet::new();
        rules.add(Rule {
            id: "hidden".into(),
            name: "Hidden".into(),
            description: "Hidden files".into(),
            category: RuleCategory::AnomalyHint,
            predicate: RulePredicate::Hidden,
            enabled: false,
        });

        let state = LocalState {
            preferences,
            profiles,
            views,
            filter_presets,
            rules,
            ..LocalState::default()
        };

        store.write_state(&state).unwrap();

        assert_eq!(store.read_state(), state);
    }

    #[test]
    fn read_state_accepts_legacy_preferences_file() {
        let dir = temp_dir();
        let store = SafeStorage::new(&dir);
        let mut prefs = Preferences::default();
        prefs.set("path_input", "/legacy");
        fs::write(&store.path, serde_json::to_string_pretty(&prefs).unwrap()).unwrap();

        let state = store.read_state();

        assert_eq!(state.preferences.get("path_input"), Some("/legacy"));
        assert!(state.profiles.is_empty());
        assert!(state.views.is_empty());
        assert!(state.filter_presets.is_empty());
        assert!(!state.rules.rules.is_empty());
    }
}
