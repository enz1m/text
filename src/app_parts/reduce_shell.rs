//! TextApp resize, theme and recovery reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_shell_resized(&mut self, w: f32, h: f32) -> Task<Message> {
        self.model.window_size = Some((w, h));
        // The paged viewer materializes `visible_rows` plus
        // bounded overscan, so a resize has to re-derive that count or
        // a taller window would show a short viewport (and a shorter
        // one would materialize rows nobody can see).
        self.sync_paged_visible_rows();
        if !self.model.is_narrow() {
            self.model.overflow_menu_open = false;
        }
        // The tab list is anchored on a chip whose very presence
        // depends on the strip width, so a resize can delete its
        // anchor. Close it rather than leave a popover hanging off a
        // control that is no longer drawn.
        self.model.tab_list_open = false;
        Task::none()
    }
    // Every shared-theme-host event (file load, OS
    // bootstrap/change, watch tick/probe) folds through the framework
    // integration. The generation is monotonic; view-side reads pull
    // the freshly-resolved `EnzTheme` from `self.themes`.
    pub(crate) fn reduce_shell_theme(&mut self, event: ThemeEvent) -> Task<Message> {
        let before = self.themes.theme_generation();
        let task = self.themes.update(event);
        debug_assert!(
            self.themes.theme_generation() >= before,
            "theme generations must be monotonic"
        );
        task.map(|event| Message::Theme(Box::new(event)))
    }
    pub(crate) fn reduce_shell_recovery_written(
        &mut self,
        generation: u64,
        result: Result<bool, BlockingTaskError>,
    ) -> Task<Message> {
        self.recovery_write_settled(generation, result)
    }
    pub(crate) fn reduce_shell_recovery_discard_settled(
        &mut self,
        generation: u64,
        result: Result<bool, BlockingTaskError>,
    ) -> Task<Message> {
        self.recovery_discard_settled(generation, result)
    }
}
