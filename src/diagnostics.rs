//! Diagnostics bundle export.
//!
//! Collects a snapshot of the app's runtime state (version, platform,
//! scan options, perf counters, recent errors) and writes it as a set
//! of plain-text files into a timestamped directory under the current
//! working directory. The user can archive that directory and share it.
//!
//! Design notes:
//!  - No archive dependency: we write a directory of `.txt` files.
//!    The user can `tar -czf` it themselves if they want a single
//!    blob — we don't pre-archive.
//!  - Path redaction: anywhere a `PathBuf` is rendered, the user's
//!    home directory is replaced with `~` to avoid leaking the full
//!    path when sharing a bundle.
//!  - The bundle is intentionally read-only. It must never trigger
//!    a destructive action, even if the underlying state looks like
//!    a stale or invalid one.

use crate::scanner::PerfStats;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const BUNDLE_DIR_PREFIX: &str = "disk-map-diagnostics-";

/// In-memory representation of the bundle. Built from a `&DiskMapApp`
/// or constructed manually in tests.
#[derive(Debug, Clone)]
pub struct DiagnosticsBundle {
    pub app_version: String,
    pub os: &'static str,
    pub arch: &'static str,
    pub generated_at_unix_secs: u64,
    pub scan_root: Option<String>,
    pub status: String,
    pub scan_options: Vec<(String, String)>,
    pub perf_stats: Option<PerfStats>,
    pub recent_errors: Vec<String>,
}

impl DiagnosticsBundle {
    /// Write the bundle to `<dest_dir>/disk-map-diagnostics-<ts>/`,
    /// creating the parent directory if needed. Returns the path to
    /// the created bundle directory.
    pub fn write_to(&self, dest_dir: &Path) -> std::io::Result<PathBuf> {
        fs::create_dir_all(dest_dir)?;
        let ts = self.generated_at_unix_secs;
        let bundle_dir = dest_dir.join(format!("{BUNDLE_DIR_PREFIX}{ts}"));
        fs::create_dir_all(&bundle_dir)?;

        write_file(&bundle_dir.join("manifest.txt",), |f| {
            writeln!(f, "disk-map diagnostics bundle")?;
            writeln!(f, "========================")?;
            writeln!(f, "app_version: {}", self.app_version)?;
            writeln!(f, "platform: {}-{}", self.os, self.arch)?;
            writeln!(
                f,
                "generated_at_unix_secs: {}",
                self.generated_at_unix_secs
            )?;
            writeln!(f, "scan_root: {}", redact_home_opt(&self.scan_root))?;
            writeln!(f, "status: {}", self.status)?;
            Ok(())
        })?;

        write_file(&bundle_dir.join("scan_options.txt",), |f| {
            writeln!(f, "scan options")?;
            writeln!(f, "============")?;
            for (k, v) in &self.scan_options {
                writeln!(f, "{}: {}", k, v)?;
            }
            Ok(())
        })?;

        write_file(&bundle_dir.join("perf.txt",), |f| {
            writeln!(f, "scan perf stats")?;
            writeln!(f, "===============")?;
            match &self.perf_stats {
                Some(s) => {
                    writeln!(f, "messages_sent: {}", s.messages_sent)?;
                    writeln!(f, "batches_sent: {}", s.batches_sent)?;
                    writeln!(f, "entries_seen: {}", s.entries_seen)?;
                    writeln!(f, "nodes_discovered: {}", s.nodes_discovered)?;
                    writeln!(f, "files_scanned: {}", s.files_scanned)?;
                    writeln!(f, "dirs_scanned: {}", s.dirs_scanned)?;
                    writeln!(
                        f,
                        "size_delta_merges: {}",
                        s.size_delta_merges
                    )?;
                    writeln!(
                        f,
                        "ancestor_size_delta_total_ms: {:.3}",
                        s.ancestor_size_delta_total_ms
                    )?;
                    writeln!(f, "parent_stack_hits: {}", s.parent_stack_hits)?;
                    writeln!(
                        f,
                        "parent_lookup_fallbacks: {}",
                        s.parent_lookup_fallbacks
                    )?;
                }
                None => writeln!(f, "(no completed scan yet)")?,
            }
            Ok(())
        })?;

        write_file(&bundle_dir.join("recent_errors.txt",), |f| {
            if self.recent_errors.is_empty() {
                writeln!(f, "(no recent errors recorded)")?;
            } else {
                for (i, line) in self.recent_errors.iter().enumerate() {
                    writeln!(f, "[{:04}] {}", i, line)?;
                }
            }
            Ok(())
        })?;

        Ok(bundle_dir)
    }
}

fn write_file<F>(path: &Path, write: F) -> std::io::Result<()>
where
    F: FnOnce(&mut fs::File) -> std::io::Result<()>,
{
    let mut f = fs::File::create(path)?;
    write(&mut f)
}

/// Replace the user's home directory prefix with `~` so the bundle
/// does not leak the full path when shared.
pub fn redact_home(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if let Ok(stripped) = path.strip_prefix(&home) {
            let s = stripped.to_string_lossy();
            return format!("~/{}", s.trim_start_matches('/'));
        }
    }
    path.display().to_string()
}

fn redact_home_opt(opt: &Option<String>) -> String {
    match opt {
        Some(s) => redact_home(Path::new(s)),
        None => "(none)".to_string(),
    }
}

/// Current Unix time in seconds. Returns 0 if the system clock is
/// before the Unix epoch (which it never is in practice, but we handle
/// the error rather than crash).
pub fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let p = std::env::temp_dir()
            .join(format!("disk-map-diagnostics-test-{pid}-{nanos}-{n}"));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn write_to_creates_four_text_files() {
        let dest = unique_temp_dir();
        let bundle = DiagnosticsBundle {
            app_version: "0.1.0-test".into(),
            os: "macos",
            arch: "aarch64",
            generated_at_unix_secs: 1_700_000_000,
            scan_root: Some("/tmp/scan".into()),
            status: "Ready".into(),
            scan_options: vec![("include_hidden".into(), "true".into())],
            perf_stats: None,
            recent_errors: vec!["scanner: timeout".into()],
        };
        let bundle_dir = bundle.write_to(&dest).unwrap();
        assert!(bundle_dir.is_dir());
        for name in ["manifest.txt", "scan_options.txt", "perf.txt", "recent_errors.txt"] {
            let p = bundle_dir.join(name);
            assert!(p.is_file(), "expected {name} to exist");
            let text = fs::read_to_string(&p).unwrap();
            assert!(!text.is_empty(), "{name} should not be empty");
        }
    }

    #[test]
    fn redact_home_replaces_home_prefix() {
        let home = std::env::var("HOME").expect("HOME should be set in test env");
        let p = Path::new(&home).join("Projects/DiskMap");
        let redacted = redact_home(&p);
        assert!(redacted.starts_with("~/"));
        assert!(!redacted.contains(&home));
    }

    #[test]
    fn redact_home_passes_through_non_home_paths() {
        let p = Path::new("/etc/hosts");
        let redacted = redact_home(p);
        assert_eq!(redacted, "/etc/hosts");
    }

    #[test]
    fn perf_stats_section_omits_when_no_scan_completed() {
        let dest = unique_temp_dir();
        let bundle = DiagnosticsBundle {
            app_version: "0.1.0-test".into(),
            os: "macos",
            arch: "aarch64",
            generated_at_unix_secs: 1_700_000_000,
            scan_root: None,
            status: "Ready".into(),
            scan_options: vec![],
            perf_stats: None,
            recent_errors: vec![],
        };
        let bundle_dir = bundle.write_to(&dest).unwrap();
        let text = fs::read_to_string(bundle_dir.join("perf.txt")).unwrap();
        assert!(text.contains("(no completed scan yet)"));
    }

    #[test]
    fn recent_errors_section_omits_when_empty() {
        let dest = unique_temp_dir();
        let bundle = DiagnosticsBundle {
            app_version: "0.1.0-test".into(),
            os: "macos",
            arch: "aarch64",
            generated_at_unix_secs: 1_700_000_000,
            scan_root: None,
            status: "Ready".into(),
            scan_options: vec![],
            perf_stats: None,
            recent_errors: vec![],
        };
        let bundle_dir = bundle.write_to(&dest).unwrap();
        let text = fs::read_to_string(bundle_dir.join("recent_errors.txt")).unwrap();
        assert!(text.contains("(no recent errors recorded)"));
    }
}
