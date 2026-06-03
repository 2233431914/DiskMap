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
    if path == Path::new("/") {
        return Some(ProtectedPathReason::FilesystemRoot);
    }

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if path == home {
            return Some(ProtectedPathReason::HomeRoot);
        }
    }

    let components = path.components().collect::<Vec<_>>();
    let [Component::RootDir, Component::Normal(first), rest @ ..] = components.as_slice() else {
        return None;
    };
    let first = first.to_string_lossy();

    if first == "Volumes" && rest.len() <= 1 {
        return Some(ProtectedPathReason::MountedVolumeRoot);
    }

    if matches!(
        first.as_ref(),
        "Applications" | "Library" | "System" | "bin" | "etc" | "private" | "sbin" | "usr" | "var"
    ) {
        return Some(ProtectedPathReason::SystemLocation);
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
    {
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    }
    paths
}

pub fn validate_cleanup_target(path: &Path, user_deny_list: &[PathBuf]) -> CleanupTargetStatus {
    if let Some(reason) = protected_path_reason_with_deny_list(path, user_deny_list) {
        return CleanupTargetStatus::Protected(reason);
    }

    match path.try_exists() {
        Ok(true) => CleanupTargetStatus::Ready,
        Ok(false) => CleanupTargetStatus::Missing,
        Err(error) => CleanupTargetStatus::Inaccessible(error.to_string()),
    }
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
        assert_eq!(
            protected_path_reason(Path::new("/System/Library")),
            Some(ProtectedPathReason::SystemLocation)
        );
        assert_eq!(
            protected_path_reason(Path::new("/Volumes/External")),
            Some(ProtectedPathReason::MountedVolumeRoot)
        );
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
    fn parse_protected_paths_splits_and_deduplicates() {
        assert_eq!(
            parse_protected_paths("/a, /b; /a\n/c"),
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c")
            ]
        );
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
