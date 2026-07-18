use std::ffi::OsString;
use std::path::Component;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformProtectedPathReason {
    SystemLocation,
    MountedVolumeRoot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformProtectedPathRule {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub path_pattern: &'static str,
}

#[cfg(target_os = "macos")]
const FULL_DISK_ACCESS_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

#[cfg(target_os = "macos")]
fn full_disk_access_settings_command() -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.arg(FULL_DISK_ACCESS_SETTINGS_URL);
    command
}

#[cfg(target_os = "macos")]
pub fn open_full_disk_access_settings() -> anyhow::Result<()> {
    full_disk_access_settings_command().spawn()?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn reveal_in_file_manager(path: &Path) -> anyhow::Result<()> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn reveal_in_file_manager(path: &Path) -> anyhow::Result<()> {
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    open::that(target)?;
    Ok(())
}

pub fn file_manager_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Finder"
    } else {
        "file manager"
    }
}

pub fn reveal_action_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else {
        "Open Containing Folder"
    }
}

pub fn reveal_action_short_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal"
    } else {
        "Open Folder"
    }
}

pub fn protected_path_reason(path: &Path) -> Option<PlatformProtectedPathReason> {
    let components = path.components().collect::<Vec<_>>();
    let [Component::RootDir, Component::Normal(first), rest @ ..] = components.as_slice() else {
        return None;
    };
    protected_path_reason_from_components(&first.to_string_lossy(), rest)
}

#[cfg(target_os = "macos")]
pub fn default_protected_path_rules() -> &'static [PlatformProtectedPathRule] {
    &[
        PlatformProtectedPathRule {
            id: "protected-applications",
            name: "Applications location",
            description:
                "Top-level /Applications entry. Destructive actions against this are blocked.",
            path_pattern: "Applications",
        },
        PlatformProtectedPathRule {
            id: "protected-library",
            name: "Library location",
            description: "Top-level /Library entry. Destructive actions against this are blocked.",
            path_pattern: "Library",
        },
        PlatformProtectedPathRule {
            id: "protected-system",
            name: "System location",
            description: "Top-level /System entry. Destructive actions against this are blocked.",
            path_pattern: "System",
        },
        PlatformProtectedPathRule {
            id: "protected-bin",
            name: "System binaries",
            description: "Top-level /bin entry. Destructive actions against this are blocked.",
            path_pattern: "bin",
        },
        PlatformProtectedPathRule {
            id: "protected-etc",
            name: "System configuration",
            description: "Top-level /etc entry. Destructive actions against this are blocked.",
            path_pattern: "etc",
        },
        PlatformProtectedPathRule {
            id: "protected-private",
            name: "Private system data",
            description: "Top-level /private entry. Destructive actions against this are blocked.",
            path_pattern: "private",
        },
        PlatformProtectedPathRule {
            id: "protected-sbin",
            name: "System admin binaries",
            description: "Top-level /sbin entry. Destructive actions against this are blocked.",
            path_pattern: "sbin",
        },
        PlatformProtectedPathRule {
            id: "protected-usr",
            name: "System binaries",
            description: "Top-level /usr entry. Destructive actions against this are blocked.",
            path_pattern: "usr",
        },
        PlatformProtectedPathRule {
            id: "protected-var",
            name: "System state",
            description: "Top-level /var entry. Destructive actions against this are blocked.",
            path_pattern: "var",
        },
        PlatformProtectedPathRule {
            id: "protected-volumes",
            name: "Mounted volume root",
            description:
                "Top-level /Volumes mount roots. Destructive actions against these are blocked.",
            path_pattern: "Volumes",
        },
    ]
}

#[cfg(target_os = "linux")]
pub fn default_protected_path_rules() -> &'static [PlatformProtectedPathRule] {
    &[
        PlatformProtectedPathRule {
            id: "protected-bin",
            name: "System binaries",
            description: "Top-level /bin entry. Destructive actions against this are blocked.",
            path_pattern: "bin",
        },
        PlatformProtectedPathRule {
            id: "protected-boot",
            name: "Boot files",
            description: "Top-level /boot entry. Destructive actions against this are blocked.",
            path_pattern: "boot",
        },
        PlatformProtectedPathRule {
            id: "protected-dev",
            name: "Device filesystem",
            description: "Top-level /dev entry. Destructive actions against this are blocked.",
            path_pattern: "dev",
        },
        PlatformProtectedPathRule {
            id: "protected-etc",
            name: "System configuration",
            description: "Top-level /etc entry. Destructive actions against this are blocked.",
            path_pattern: "etc",
        },
        PlatformProtectedPathRule {
            id: "protected-lib",
            name: "System libraries",
            description: "Top-level /lib entry. Destructive actions against this are blocked.",
            path_pattern: "lib",
        },
        PlatformProtectedPathRule {
            id: "protected-lib64",
            name: "System libraries",
            description: "Top-level /lib64 entry. Destructive actions against this are blocked.",
            path_pattern: "lib64",
        },
        PlatformProtectedPathRule {
            id: "protected-opt",
            name: "Optional system packages",
            description: "Top-level /opt entry. Destructive actions against this are blocked.",
            path_pattern: "opt",
        },
        PlatformProtectedPathRule {
            id: "protected-proc",
            name: "proc filesystem",
            description: "Top-level /proc entry. Destructive actions against this are blocked.",
            path_pattern: "proc",
        },
        PlatformProtectedPathRule {
            id: "protected-root",
            name: "Root user home",
            description: "Top-level /root entry. Destructive actions against this are blocked.",
            path_pattern: "root",
        },
        PlatformProtectedPathRule {
            id: "protected-run",
            name: "Runtime state",
            description: "Top-level /run entry. Destructive actions against this are blocked.",
            path_pattern: "run",
        },
        PlatformProtectedPathRule {
            id: "protected-sbin",
            name: "System admin binaries",
            description: "Top-level /sbin entry. Destructive actions against this are blocked.",
            path_pattern: "sbin",
        },
        PlatformProtectedPathRule {
            id: "protected-srv",
            name: "Service data",
            description: "Top-level /srv entry. Destructive actions against this are blocked.",
            path_pattern: "srv",
        },
        PlatformProtectedPathRule {
            id: "protected-sys",
            name: "sys filesystem",
            description: "Top-level /sys entry. Destructive actions against this are blocked.",
            path_pattern: "sys",
        },
        PlatformProtectedPathRule {
            id: "protected-usr",
            name: "System binaries",
            description: "Top-level /usr entry. Destructive actions against this are blocked.",
            path_pattern: "usr",
        },
        PlatformProtectedPathRule {
            id: "protected-var",
            name: "System state",
            description: "Top-level /var entry. Destructive actions against this are blocked.",
            path_pattern: "var",
        },
        PlatformProtectedPathRule {
            id: "protected-mnt",
            name: "Mounted volume root",
            description: "Mount roots under /mnt. Destructive actions against these are blocked.",
            path_pattern: "mnt",
        },
        PlatformProtectedPathRule {
            id: "protected-media",
            name: "Mounted media root",
            description: "Mount roots under /media. Destructive actions against these are blocked.",
            path_pattern: "media",
        },
        PlatformProtectedPathRule {
            id: "protected-run-media",
            name: "Mounted media root",
            description:
                "Mount roots under /run/media. Destructive actions against these are blocked.",
            path_pattern: "run/media",
        },
    ]
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn default_protected_path_rules() -> &'static [PlatformProtectedPathRule] {
    &[
        PlatformProtectedPathRule {
            id: "protected-usr",
            name: "System binaries",
            description: "Top-level /usr entry. Destructive actions against this are blocked.",
            path_pattern: "usr",
        },
        PlatformProtectedPathRule {
            id: "protected-var",
            name: "System state",
            description: "Top-level /var entry. Destructive actions against this are blocked.",
            path_pattern: "var",
        },
    ]
}

#[cfg(target_os = "macos")]
fn protected_path_reason_from_components(
    first: &str,
    rest: &[Component<'_>],
) -> Option<PlatformProtectedPathReason> {
    if first == "Volumes" && rest.len() <= 1 {
        return Some(PlatformProtectedPathReason::MountedVolumeRoot);
    }

    if matches!(
        first,
        "Applications" | "Library" | "System" | "bin" | "etc" | "private" | "sbin" | "usr" | "var"
    ) {
        return Some(PlatformProtectedPathReason::SystemLocation);
    }

    None
}

#[cfg(target_os = "linux")]
fn protected_path_reason_from_components(
    first: &str,
    rest: &[Component<'_>],
) -> Option<PlatformProtectedPathReason> {
    if first == "mnt" && rest.len() <= 1 {
        return Some(PlatformProtectedPathReason::MountedVolumeRoot);
    }
    if first == "media" && rest.len() <= 2 {
        return Some(PlatformProtectedPathReason::MountedVolumeRoot);
    }
    if first == "run"
        && matches!(rest.first(), Some(Component::Normal(component)) if component.to_string_lossy() == "media")
    {
        if rest.len() <= 3 {
            return Some(PlatformProtectedPathReason::MountedVolumeRoot);
        }
        return None;
    }

    if matches!(
        first,
        "bin"
            | "boot"
            | "dev"
            | "etc"
            | "lib"
            | "lib64"
            | "opt"
            | "proc"
            | "root"
            | "run"
            | "sbin"
            | "srv"
            | "sys"
            | "usr"
            | "var"
    ) {
        return Some(PlatformProtectedPathReason::SystemLocation);
    }

    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn protected_path_reason_from_components(
    first: &str,
    _rest: &[Component<'_>],
) -> Option<PlatformProtectedPathReason> {
    if matches!(first, "bin" | "etc" | "sbin" | "usr" | "var") {
        return Some(PlatformProtectedPathReason::SystemLocation);
    }

    None
}

pub fn open_path(path: &Path) -> anyhow::Result<()> {
    open::that(path)?;
    Ok(())
}

pub fn app_data_dir(app_id: &str) -> PathBuf {
    app_data_dir_from_env(
        app_id,
        std::env::var_os("HOME"),
        std::env::var_os("XDG_DATA_HOME"),
    )
}

#[cfg(target_os = "macos")]
fn app_data_dir_from_env(
    app_id: &str,
    home: Option<OsString>,
    _xdg_data_home: Option<OsString>,
) -> PathBuf {
    if let Some(home) = home {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join(app_id);
    }
    PathBuf::from("/tmp").join(app_id)
}

#[cfg(not(target_os = "macos"))]
fn app_data_dir_from_env(
    app_id: &str,
    home: Option<OsString>,
    xdg_data_home: Option<OsString>,
) -> PathBuf {
    if let Some(xdg_data_home) = absolute_env_path(xdg_data_home) {
        return xdg_data_home.join(app_id);
    }
    if let Some(home) = home {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join(app_id);
    }
    PathBuf::from("/tmp").join(app_id)
}

#[cfg(not(target_os = "macos"))]
fn absolute_env_path(value: Option<OsString>) -> Option<PathBuf> {
    let path = PathBuf::from(value?);
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return None;
    }
    Some(path)
}

#[cfg(test)]
pub fn move_to_trash(path: &Path) -> anyhow::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(all(not(test), target_os = "macos"))]
pub fn move_to_trash(path: &Path) -> anyhow::Result<()> {
    use trash::macos::{DeleteMethod, TrashContextExtMacos};

    let mut context = trash::TrashContext::new();
    context.set_delete_method(DeleteMethod::NsFileManager);
    context.delete(path)?;
    Ok(())
}

#[cfg(all(not(test), not(target_os = "macos")))]
pub fn move_to_trash(path: &Path) -> anyhow::Result<()> {
    trash::delete(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn full_disk_access_command_targets_privacy_all_files_settings() {
        let command = full_disk_access_settings_command();

        assert_eq!(
            (
                command.get_program(),
                command.get_args().collect::<Vec<_>>()
            ),
            (
                std::ffi::OsStr::new("open"),
                vec![std::ffi::OsStr::new(FULL_DISK_ACCESS_SETTINGS_URL)]
            )
        );
    }

    #[test]
    fn file_manager_name_is_platform_specific() {
        if cfg!(target_os = "macos") {
            assert_eq!(file_manager_name(), "Finder");
            assert_eq!(reveal_action_label(), "Reveal in Finder");
            assert_eq!(reveal_action_short_label(), "Reveal");
        } else {
            assert_eq!(file_manager_name(), "file manager");
            assert_eq!(reveal_action_label(), "Open Containing Folder");
            assert_eq!(reveal_action_short_label(), "Open Folder");
        }
    }

    #[test]
    fn app_data_dir_uses_platform_conventions() {
        let home = Some(OsString::from("/home/user"));
        let xdg = Some(OsString::from("/custom/data"));

        let path = app_data_dir_from_env("disk-map", home.clone(), xdg);

        if cfg!(target_os = "macos") {
            assert_eq!(
                path,
                PathBuf::from("/home/user")
                    .join("Library")
                    .join("Application Support")
                    .join("disk-map")
            );
        } else {
            assert_eq!(path, PathBuf::from("/custom/data").join("disk-map"));
            assert_eq!(
                app_data_dir_from_env("disk-map", home, None),
                PathBuf::from("/home/user")
                    .join(".local")
                    .join("share")
                    .join("disk-map")
            );
        }
    }

    #[test]
    fn app_data_dir_ignores_invalid_xdg_data_home() {
        if cfg!(not(target_os = "macos")) {
            let home = Some(OsString::from("/home/user"));
            let fallback = PathBuf::from("/home/user")
                .join(".local")
                .join("share")
                .join("disk-map");

            assert_eq!(
                app_data_dir_from_env("disk-map", home.clone(), Some(OsString::new())),
                fallback
            );
            assert_eq!(
                app_data_dir_from_env("disk-map", home, Some(OsString::from("relative/path"))),
                fallback
            );
        }
    }

    #[test]
    fn app_data_dir_falls_back_to_tmp_without_home() {
        assert_eq!(
            app_data_dir_from_env("disk-map", None, None),
            PathBuf::from("/tmp").join("disk-map")
        );
    }

    #[test]
    fn protected_paths_are_platform_specific() {
        if cfg!(target_os = "macos") {
            assert_eq!(
                protected_path_reason(Path::new("/Volumes/External")),
                Some(PlatformProtectedPathReason::MountedVolumeRoot)
            );
            assert_eq!(
                protected_path_reason(Path::new("/System/Library")),
                Some(PlatformProtectedPathReason::SystemLocation)
            );
        } else if cfg!(target_os = "linux") {
            assert_eq!(
                protected_path_reason(Path::new("/media/user/External")),
                Some(PlatformProtectedPathReason::MountedVolumeRoot)
            );
            assert_eq!(
                protected_path_reason(Path::new("/media/user/External/file")),
                None
            );
            assert_eq!(
                protected_path_reason(Path::new("/mnt/External")),
                Some(PlatformProtectedPathReason::MountedVolumeRoot)
            );
            assert_eq!(protected_path_reason(Path::new("/mnt/External/file")), None);
            assert_eq!(
                protected_path_reason(Path::new("/run/media/user/External")),
                Some(PlatformProtectedPathReason::MountedVolumeRoot)
            );
            assert_eq!(
                protected_path_reason(Path::new("/run/media/user/External/file")),
                None
            );
            assert_eq!(
                protected_path_reason(Path::new("/proc/self")),
                Some(PlatformProtectedPathReason::SystemLocation)
            );
            assert_eq!(
                protected_path_reason(Path::new("/run/lock")),
                Some(PlatformProtectedPathReason::SystemLocation)
            );
            assert_eq!(protected_path_reason(Path::new("/Applications")), None);
        }
    }
}
