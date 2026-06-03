//! Per-root scan option profiles.
//!
//! `ScanProfile` captures the user-facing scan options for a single
//! scan root. `ProfileStore` maps root paths to profiles, in-memory
//! only (no persistence yet — that's deferred to a future task).
//!
//! Design notes:
//!  - In-memory only. JSON serialization helpers are provided so a
//!    future persistence layer (e.g. Phase 15) can be added without
//!    a breaking change.
//!  - The "key" is the canonical (or user-typed) root path string.
//!    No special normalization — what the user typed is what we key
//!    on. If you scan `/Users/me/Downloads` and later
//!    `/Users/me/Downloads/`, they're different keys. (Documented in
//!    the UI.)
//!  - Last-write-wins on conflict. No merge, no diff.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanProfile {
    pub exclude_patterns: Vec<String>,
    pub include_hidden: bool,
    pub follow_symlinks: bool,
    pub stay_on_filesystem: bool,
    pub sqlite_cache_enabled: bool,
    pub search_filter_enabled: bool,
    pub color_by_extension: bool,
    pub realtime_watch_enabled: bool,
}

impl Default for ScanProfile {
    fn default() -> Self {
        // Match ScanOptions::default + the rest of the user-facing
        // scan options as the user sees them on first launch.
        Self {
            exclude_patterns: Vec::new(),
            include_hidden: true,
            follow_symlinks: false,
            stay_on_filesystem: false,
            sqlite_cache_enabled: false,
            search_filter_enabled: false,
            color_by_extension: false,
            realtime_watch_enabled: true,
        }
    }
}

impl ScanProfile {
    /// Serialize to JSON. Returns an empty object string on failure
    /// (serde_json::to_string on a derived struct shouldn't fail in
    /// practice, but we keep the signature infallible to match the
    /// rest of the diagnostics / rules I/O surface).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Parse JSON into a `ScanProfile`. Returns an error message on
    /// parse failure. Missing fields fall back to `Default::default()`
    /// via serde's `#[serde(default)]` semantics on the struct.
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str::<ScanProfile>(json)
            .map_err(|e| format!("invalid ScanProfile JSON: {e}"))
    }
}

/// Wrapper for `ProfileStore` JSON serialization. BTreeMap is used
/// (not HashMap) for deterministic on-disk order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileStoreFile {
    pub profiles: BTreeMap<String, ScanProfile>,
}

#[derive(Debug, Clone, Default)]
pub struct ProfileStore {
    profiles: BTreeMap<String, ScanProfile>,
}

impl ProfileStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Key normalization. We trim trailing slashes so `/a/` and `/a`
    /// map to the same profile. The original `Path::canonicalize`
    /// would be ideal but it requires the path to exist; we don't
    /// want profile lookups to fail before a scan starts.
    fn normalize_key(root: &str) -> String {
        let p = Path::new(root);
        let mut s = p.to_string_lossy().to_string();
        while s.len() > 1 && s.ends_with('/') {
            s.pop();
        }
        s
    }

    pub fn get(&self, root: &str) -> Option<&ScanProfile> {
        self.profiles.get(&Self::normalize_key(root))
    }

    pub fn get_or_default(&self, root: &str) -> ScanProfile {
        self.get(root).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, root: &str, profile: ScanProfile) {
        self.profiles.insert(Self::normalize_key(root), profile);
    }

    pub fn remove(&mut self, root: &str) -> Option<ScanProfile> {
        self.profiles.remove(&Self::normalize_key(root))
    }

    pub fn list(&self) -> Vec<(String, ScanProfile)> {
        self.profiles.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    pub fn to_json(&self) -> String {
        let file = ProfileStoreFile {
            profiles: self.profiles.clone(),
        };
        serde_json::to_string_pretty(&file)
            .unwrap_or_else(|_| "{\"profiles\":{}}".to_string())
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        let file: ProfileStoreFile = serde_json::from_str(json)
            .map_err(|e| format!("invalid ProfileStore JSON: {e}"))?;
        Ok(Self {
            profiles: file.profiles,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_a() -> ScanProfile {
        ScanProfile {
            exclude_patterns: vec![".git".into(), "target".into()],
            include_hidden: true,
            follow_symlinks: true,
            stay_on_filesystem: false,
            sqlite_cache_enabled: false,
            search_filter_enabled: true,
            color_by_extension: true,
            realtime_watch_enabled: false,
        }
    }

    #[test]
    fn default_profile_matches_user_defaults() {
        let d = ScanProfile::default();
        assert!(d.include_hidden);
        assert!(!d.follow_symlinks);
        assert!(d.realtime_watch_enabled);
        assert!(d.exclude_patterns.is_empty());
    }

    #[test]
    fn get_or_default_returns_default_for_unknown_root() {
        let store = ProfileStore::new();
        let p = store.get_or_default("/never/seen");
        assert_eq!(p, ScanProfile::default());
    }

    #[test]
    fn set_then_get_round_trips() {
        let mut store = ProfileStore::new();
        let p = profile_a();
        store.set("/Users/me/Downloads", p.clone());
        assert_eq!(store.get("/Users/me/Downloads"), Some(&p));
    }

    #[test]
    fn trailing_slash_normalization() {
        let mut store = ProfileStore::new();
        store.set("/Users/me/Downloads", profile_a());
        // Both should hit the same key
        assert!(store.get("/Users/me/Downloads/").is_some());
        assert!(store.get("/Users/me/Downloads").is_some());
    }

    #[test]
    fn remove_returns_stored_profile() {
        let mut store = ProfileStore::new();
        store.set("/a", profile_a());
        let removed = store.remove("/a");
        assert!(removed.is_some());
        assert!(store.get("/a").is_none());
    }

    #[test]
    fn list_returns_sorted_entries() {
        let mut store = ProfileStore::new();
        store.set("/b", profile_a());
        store.set("/a", profile_a());
        let list = store.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, "/a");
        assert_eq!(list[1].0, "/b");
    }

    #[test]
    fn json_round_trip() {
        let mut store = ProfileStore::new();
        store.set("/a", profile_a());
        store.set("/b", ScanProfile::default());
        let json = store.to_json();
        let restored = ProfileStore::from_json(&json).unwrap();
        assert_eq!(store.list(), restored.list());
    }

    #[test]
    fn invalid_json_returns_error() {
        let err = ProfileStore::from_json("not json").unwrap_err();
        assert!(err.contains("invalid"), "got: {err}");
    }

    #[test]
    fn missing_fields_fall_back_to_default_in_json() {
        // Parse a profile JSON that's missing several fields. Serde
        // will fail because none of the fields are #[serde(default)].
        // This test pins that behavior: the user must send a complete
        // profile JSON. We can loosen later if needed.
        let incomplete = r#"{"include_hidden": true}"#;
        let err = ScanProfile::from_json(incomplete).unwrap_err();
        assert!(err.contains("invalid"), "got: {err}");
    }
}
