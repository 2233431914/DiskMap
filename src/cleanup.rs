use crate::platform::{self, PlatformProtectedPathReason};
use crate::tree::{NodeId, NodeKind};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupCandidate {
    pub node_id: NodeId,
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub item_count: usize,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueAddResult {
    Added,
    AlreadyQueued,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupTargetStatus {
    Ready,
    Missing,
    Inaccessible(String),
    Protected(ProtectedPathReason),
}

#[derive(Debug, Default, Clone)]
pub struct CleanupQueue {
    candidates: Vec<CleanupCandidate>,
}

impl CleanupQueue {
    pub fn candidates(&self) -> &[CleanupCandidate] {
        &self.candidates
    }

    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    pub fn total_size(&self) -> u64 {
        self.candidates.iter().map(|candidate| candidate.size).sum()
    }

    pub fn add(&mut self, candidate: CleanupCandidate) -> QueueAddResult {
        if self
            .candidates
            .iter()
            .any(|existing| existing.path == candidate.path)
        {
            return QueueAddResult::AlreadyQueued;
        }

        self.candidates.push(candidate);
        QueueAddResult::Added
    }

    pub fn get(&self, node_id: NodeId) -> Option<&CleanupCandidate> {
        self.candidates
            .iter()
            .find(|candidate| candidate.node_id == node_id)
    }

    pub fn contains_node(&self, node_id: NodeId) -> bool {
        self.get(node_id).is_some()
    }

    pub fn remove(&mut self, node_id: NodeId) -> Option<CleanupCandidate> {
        let index = self
            .candidates
            .iter()
            .position(|candidate| candidate.node_id == node_id)?;
        Some(self.candidates.remove(index))
    }

    pub fn clear(&mut self) {
        self.candidates.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedPathReason {
    FilesystemRoot,
    HomeRoot,
    SystemLocation,
    MountedVolumeRoot,
    UserDenyList,
}

impl ProtectedPathReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::FilesystemRoot => "filesystem root",
            Self::HomeRoot => "home folder root",
            Self::SystemLocation => "system location",
            Self::MountedVolumeRoot => "mounted volume root",
            Self::UserDenyList => "user protected path",
        }
    }
}

pub fn protected_path_reason(path: &Path) -> Option<ProtectedPathReason> {
    protected_path_reason_with_deny_list(path, &[])
}

pub fn protected_path_reason_with_deny_list(
    path: &Path,
    user_deny_list: &[PathBuf],
) -> Option<ProtectedPathReason> {
    let path_candidates = cleanup_path_candidates(path);
    let deny_candidates: Vec<PathBuf> = user_deny_list
        .iter()
        .flat_map(|denied| cleanup_path_candidates(denied).into_iter())
        .collect();

    path_candidates
        .iter()
        .find_map(|candidate| protected_path_reason_for_candidate(candidate, &deny_candidates))
}

fn protected_path_reason_for_candidate(
    path: &Path,
    user_deny_list: &[PathBuf],
) -> Option<ProtectedPathReason> {
    if path == Path::new("/") {
        return Some(ProtectedPathReason::FilesystemRoot);
    }

    if std::env::var_os("HOME")
        .map(PathBuf::from)
        .is_some_and(|home| {
            cleanup_path_candidates(&home)
                .iter()
                .any(|home| path == home)
        })
    {
        return Some(ProtectedPathReason::HomeRoot);
    }

    if let Some(reason) = platform::protected_path_reason(path) {
        return Some(match reason {
            PlatformProtectedPathReason::SystemLocation => ProtectedPathReason::SystemLocation,
            PlatformProtectedPathReason::MountedVolumeRoot => {
                ProtectedPathReason::MountedVolumeRoot
            }
        });
    }

    if user_deny_list
        .iter()
        .any(|denied| path == denied || path.starts_with(denied))
    {
        return Some(ProtectedPathReason::UserDenyList);
    }

    None
}

pub fn parse_protected_paths(input: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for path in input
        .split([',', ';', '\n'])
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|path| normalize_cleanup_path(&path))
    {
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    }
    paths
}

pub fn validate_cleanup_target(path: &Path, user_deny_list: &[PathBuf]) -> CleanupTargetStatus {
    let path = normalize_cleanup_path(path);
    if let Some(reason) = protected_path_reason_with_deny_list(&path, user_deny_list) {
        return CleanupTargetStatus::Protected(reason);
    }

    match std::fs::symlink_metadata(&path) {
        Ok(_) => CleanupTargetStatus::Ready,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CleanupTargetStatus::Missing,
        Err(error) => CleanupTargetStatus::Inaccessible(error.to_string()),
    }
}

/// Makes a cleanup operation path absolute without changing how the OS resolves
/// symlinks and parent components when the action is executed.
pub fn normalize_cleanup_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn cleanup_path_candidates(path: &Path) -> Vec<PathBuf> {
    let operation_path = normalize_cleanup_path(path);
    let mut candidates = vec![operation_path.clone()];

    let lexical_path = normalize_path_components(&operation_path);
    if !candidates.contains(&lexical_path) {
        candidates.push(lexical_path);
    }

    if let Some(resolved_path) = resolve_cleanup_path(&operation_path) {
        if !candidates.contains(&resolved_path) {
            candidates.push(resolved_path);
        }
    }

    candidates
}

fn resolve_cleanup_path(path: &Path) -> Option<PathBuf> {
    path.canonicalize().ok().or_else(|| {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        if !metadata.file_type().is_symlink() {
            return None;
        }
        let file_name = path.file_name()?;
        let parent = path.parent()?.canonicalize().ok()?;
        Some(parent.join(file_name))
    })
}

fn normalize_path_components(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                }
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_queue_deduplicates_by_path() {
        let mut queue = CleanupQueue::default();
        let candidate = CleanupCandidate {
            node_id: 1,
            name: "file.txt".into(),
            path: "/tmp/file.txt".into(),
            size: 10,
            item_count: 1,
            kind: NodeKind::File,
        };

        assert_eq!(queue.add(candidate.clone()), QueueAddResult::Added);
        assert_eq!(queue.add(candidate), QueueAddResult::AlreadyQueued);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.total_size(), 10);
    }

    #[test]
    fn cleanup_queue_removes_by_node_id() {
        let mut queue = CleanupQueue::default();
        queue.add(CleanupCandidate {
            node_id: 1,
            name: "file.txt".into(),
            path: "/tmp/file.txt".into(),
            size: 10,
            item_count: 1,
            kind: NodeKind::File,
        });

        assert!(queue.contains_node(1));
        assert_eq!(
            queue.remove(1).map(|candidate| candidate.name),
            Some("file.txt".into())
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn protected_paths_include_roots_system_locations_and_volume_roots() {
        assert_eq!(
            protected_path_reason(Path::new("/")),
            Some(ProtectedPathReason::FilesystemRoot)
        );

        if cfg!(target_os = "macos") {
            assert_eq!(
                protected_path_reason(Path::new("/System/Library")),
                Some(ProtectedPathReason::SystemLocation)
            );
            assert_eq!(
                protected_path_reason(Path::new("/Volumes/External")),
                Some(ProtectedPathReason::MountedVolumeRoot)
            );
        } else if cfg!(target_os = "linux") {
            assert_eq!(
                protected_path_reason(Path::new("/proc/self")),
                Some(ProtectedPathReason::SystemLocation)
            );
            assert_eq!(
                protected_path_reason(Path::new("/media/user/External")),
                Some(ProtectedPathReason::MountedVolumeRoot)
            );
        }

        assert_eq!(protected_path_reason(Path::new("/tmp/file.txt")), None);
    }

    #[test]
    fn protected_paths_include_user_deny_list_descendants() {
        let denied = vec![PathBuf::from("/Users/me/keep")];

        assert_eq!(
            protected_path_reason_with_deny_list(Path::new("/Users/me/keep/cache"), &denied),
            Some(ProtectedPathReason::UserDenyList)
        );
        assert_eq!(
            protected_path_reason_with_deny_list(Path::new("/Users/me/other"), &denied),
            None
        );
    }

    #[test]
    fn protected_paths_cannot_be_bypassed_with_parent_components() {
        assert_eq!(
            protected_path_reason(Path::new("/tmp/../")),
            Some(ProtectedPathReason::FilesystemRoot)
        );

        let denied = vec![PathBuf::from("/Users/me/keep")];
        assert_eq!(
            protected_path_reason_with_deny_list(
                Path::new("/Users/me/other/../keep/cache"),
                &denied
            ),
            Some(ProtectedPathReason::UserDenyList)
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_to_protected_target_is_blocked_without_rewriting_operation_path() {
        use std::os::unix::fs::symlink;

        let root = std::env::current_dir()
            .expect("test current dir should be available")
            .join("target/test-temp/cleanup-symlink");
        std::fs::create_dir_all(&root).expect("cleanup symlink fixture should be created");
        let link = root.join("root-link");
        symlink(Path::new("/"), &link).expect("cleanup symlink should be created");

        assert_eq!(
            protected_path_reason(&link),
            Some(ProtectedPathReason::FilesystemRoot)
        );
        assert_eq!(normalize_cleanup_path(&link), link);

        let _ = std::fs::remove_file(link);
        let _ = std::fs::remove_dir(root);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_path_preserves_symlink_aware_parent_semantics() {
        use std::os::unix::fs::symlink;

        let root = std::env::current_dir()
            .expect("test current dir should be available")
            .join(format!(
                "target/test-temp/cleanup-parent-symlink-{}",
                std::process::id()
            ));
        let actual_parent = root.join("actual");
        let child = actual_parent.join("child");
        let link = root.join("link");
        let victim = actual_parent.join("victim");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&child).expect("cleanup fixture should be created");
        std::fs::write(&victim, b"keep").expect("protected fixture should be created");
        symlink(&child, &link).expect("cleanup symlink should be created");
        let operation_path = link.join("..").join("victim");

        assert_eq!(normalize_cleanup_path(&operation_path), operation_path);
        assert_eq!(
            validate_cleanup_target(&operation_path, std::slice::from_ref(&actual_parent)),
            CleanupTargetStatus::Protected(ProtectedPathReason::UserDenyList)
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn validate_cleanup_target_accepts_dangling_symlinks() {
        use std::os::unix::fs::symlink;

        let root = std::env::current_dir()
            .expect("test current dir should be available")
            .join("target/test-temp/cleanup-dangling-symlink");
        std::fs::create_dir_all(&root).expect("cleanup symlink fixture should be created");
        let link = root.join("missing-link");
        symlink(root.join("does-not-exist"), &link).expect("dangling symlink should be created");

        assert_eq!(
            validate_cleanup_target(&link, &[]),
            CleanupTargetStatus::Ready
        );

        let _ = std::fs::remove_file(link);
        let _ = std::fs::remove_dir(root);
    }

    #[test]
    fn parse_protected_paths_splits_and_deduplicates() {
        let parsed = parse_protected_paths("/a, /b; /a\n/c");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0], normalize_cleanup_path(Path::new("/a")));
        assert_eq!(parsed[1], normalize_cleanup_path(Path::new("/b")));
        assert_eq!(parsed[2], normalize_cleanup_path(Path::new("/c")));
    }

    #[test]
    fn validate_cleanup_target_reports_ready_missing_protected_and_inaccessible() {
        let unique = format!(
            "disk-map-cleanup-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        );
        let temp_root = std::env::current_dir()
            .expect("test current dir should be available")
            .join("target/test-temp");
        std::fs::create_dir_all(&temp_root).expect("test temp root should be created");
        let dir = temp_root.join(unique);
        std::fs::create_dir(&dir).expect("temp cleanup test dir should be created");

        assert_eq!(
            validate_cleanup_target(&dir, &[]),
            CleanupTargetStatus::Ready
        );
        assert_eq!(
            validate_cleanup_target(Path::new("/"), &[]),
            CleanupTargetStatus::Protected(ProtectedPathReason::FilesystemRoot)
        );

        std::fs::remove_dir(&dir).expect("temp cleanup test dir should be removed");
        assert_eq!(
            validate_cleanup_target(&dir, &[]),
            CleanupTargetStatus::Missing
        );

        let parent_file = temp_root.join(format!(
            "disk-map-cleanup-parent-file-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        std::fs::write(&parent_file, b"file").expect("temp cleanup test file should be written");
        let child = parent_file.join("child");
        assert!(matches!(
            validate_cleanup_target(&child, &[]),
            CleanupTargetStatus::Inaccessible(_)
        ));
        std::fs::remove_file(&parent_file).expect("temp cleanup test file should be removed");
    }
}
