use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusSource {
    System,
    Scan,
    Watch,
    Roots,
    Cleanup,
    Platform,
    Export,
    Analysis,
    #[cfg(test)]
    Persistence,
    #[cfg(test)]
    Rules,
    #[cfg(test)]
    Profile,
    #[cfg(test)]
    View,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusLevel {
    Info,
    Progress,
    Success,
    Warning,
    Error,
    Confirmation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AppStatus {
    source: StatusSource,
    level: StatusLevel,
    text: String,
    watch_failure: Option<String>,
}

impl Default for AppStatus {
    fn default() -> Self {
        Self {
            source: StatusSource::System,
            level: StatusLevel::Info,
            text: "Ready".to_string(),
            watch_failure: None,
        }
    }
}

impl AppStatus {
    pub fn set_primary(
        &mut self,
        source: StatusSource,
        level: StatusLevel,
        text: impl Into<String>,
    ) {
        self.source = source;
        self.level = level;
        self.text = text.into();
    }

    pub fn set_watch_failure(&mut self, error: impl Into<String>) {
        self.watch_failure = Some(error.into());
    }

    pub fn clear_watch_failure(&mut self) {
        self.watch_failure = None;
    }

    pub fn has_watch_failure(&self) -> bool {
        self.watch_failure.is_some()
    }

    #[cfg(test)]
    pub fn source(&self) -> StatusSource {
        self.source
    }

    pub fn level(&self) -> StatusLevel {
        self.level
    }

    #[cfg(test)]
    pub fn primary_text(&self) -> &str {
        &self.text
    }

    pub fn display_text(&self) -> Cow<'_, str> {
        match &self.watch_failure {
            Some(error) => Cow::Owned(format!("{} · Watch failed: {error}", self.text)),
            None => Cow::Borrowed(&self.text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_failure_is_composed_without_replacing_primary_status() {
        let mut status = AppStatus::default();
        status.set_primary(StatusSource::Scan, StatusLevel::Success, "Finished: 1 KiB");
        status.set_watch_failure("backend failed");

        assert_eq!(
            status.display_text(),
            "Finished: 1 KiB · Watch failed: backend failed"
        );
        assert_eq!(status.source(), StatusSource::Scan);
        assert_eq!(status.level(), StatusLevel::Success);
    }

    #[test]
    fn clearing_watch_failure_restores_primary_status() {
        let mut status = AppStatus::default();
        status.set_primary(StatusSource::Scan, StatusLevel::Success, "Finished: 1 KiB");
        status.set_watch_failure("backend failed");

        status.clear_watch_failure();

        assert_eq!(status.display_text(), "Finished: 1 KiB");
    }

    #[test]
    fn replacing_primary_status_preserves_watch_failure() {
        let mut status = AppStatus::default();
        status.set_watch_failure("backend failed");

        status.set_primary(StatusSource::View, StatusLevel::Info, "Layout refreshed");

        assert_eq!(
            status.display_text(),
            "Layout refreshed · Watch failed: backend failed"
        );
    }
}
