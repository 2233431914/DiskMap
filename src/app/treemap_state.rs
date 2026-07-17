use super::LAYOUT_REFRESH_INTERVAL;
use crate::tree::{NodeId, TreeStore};
use crate::treemap::{layout_treemap, LayoutScratch, SearchState, TreemapLayoutParams, VisualNode};
use egui::Rect;
use std::time::{Duration, Instant};

pub(super) struct TreemapLayoutRequest<'a> {
    pub root: NodeId,
    pub canvas_rect: Rect,
    pub max_depth: usize,
    pub search_state: &'a SearchState,
    pub filter_to_search: bool,
    pub scanning: bool,
}

pub(super) struct TreemapViewState {
    visuals: Vec<VisualNode>,
    scratch: LayoutScratch,
    canvas_rect: Option<Rect>,
    dirty: bool,
    last_refresh: Instant,
}

impl Default for TreemapViewState {
    fn default() -> Self {
        Self {
            visuals: Vec::new(),
            scratch: LayoutScratch::default(),
            canvas_rect: None,
            dirty: true,
            last_refresh: Instant::now(),
        }
    }
}

impl TreemapViewState {
    pub fn clear(&mut self) {
        self.visuals.clear();
        self.canvas_rect = None;
        self.dirty = true;
    }

    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub fn invalidate_now(&mut self) {
        self.dirty = true;
        self.last_refresh = Instant::now()
            .checked_sub(LAYOUT_REFRESH_INTERVAL)
            .unwrap_or_else(Instant::now);
    }

    pub fn visuals(&self) -> &[VisualNode] {
        &self.visuals
    }

    pub fn layout_if_due(
        &mut self,
        tree: &mut TreeStore,
        request: TreemapLayoutRequest<'_>,
    ) -> Option<Duration> {
        if self.canvas_rect != Some(request.canvas_rect) {
            self.canvas_rect = Some(request.canvas_rect);
            self.invalidate_now();
        }
        if !self.dirty
            || (request.scanning && self.last_refresh.elapsed() < LAYOUT_REFRESH_INTERVAL)
        {
            return None;
        }

        let started_at = Instant::now();
        layout_treemap(
            tree,
            TreemapLayoutParams {
                root: request.root,
                canvas_rect: request.canvas_rect,
                max_depth: request.max_depth,
                search_state: request.search_state,
                filter_to_search: request.filter_to_search,
                out: &mut self.visuals,
                scratch: &mut self.scratch,
            },
        );
        self.dirty = false;
        self.last_refresh = Instant::now();
        Some(started_at.elapsed())
    }

    #[cfg(test)]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    #[cfg(test)]
    pub fn mark_clean_for_test(&mut self) {
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidate_now_marks_clean_state_dirty() {
        let mut state = TreemapViewState::default();
        state.mark_clean_for_test();

        state.invalidate_now();

        assert!(state.is_dirty());
    }

    #[test]
    fn clear_drops_cached_canvas_and_marks_layout_dirty() {
        let mut state = TreemapViewState {
            canvas_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(100.0, 100.0),
            )),
            ..Default::default()
        };
        state.mark_clean_for_test();

        state.clear();

        assert!(state.visuals().is_empty());
        assert!(state.canvas_rect.is_none());
        assert!(state.is_dirty());
    }
}
