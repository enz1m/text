use crate::{Message, TextApp};
use iced::Task;
use iced::widget::text_editor;

impl TextApp {
    // Compatibility drift only; integrated gutter paint owns its scroll state.

    pub(super) fn sync_gutter_scroll(&mut self) -> Task<Message> {
        let metrics = self.model.active_tab().editor_scroll_metrics();
        self.gutter_scroll_offset = metrics.offset.y;
        self.gutter_window_first_line.set(metrics.first_line);
        self.gutter_window_placement.set(
            self.model
                .settings
                .word_wrap
                .then_some(metrics.first_line_top),
        );
        self.gutter_window_size.set(self.model.window_size);
        self.settle_recovered_scroll(&metrics)
    }

    // Waits for real geometry (`viewport.height > 0`), then clamps the offset
    // and drives it once through the `ScrollTo` path. A user editor action
    // clears `pending_scroll` first and supersedes it.
    fn settle_recovered_scroll(&mut self, metrics: &text_editor::ScrollMetrics) -> Task<Message> {
        let Some(target) = self.model.active_tab().pending_scroll else {
            return Task::none();
        };
        // Ignore bootstrap zero geometry; retry on the next message.
        if metrics.viewport.height <= 0.0 {
            return Task::none();
        }
        let max = (metrics.content.height - metrics.viewport.height).max(0.0);
        let clamped = target.clamp(0.0, max);
        // Settled: clear now so a re-entrant sync (after the ScrollTo below) is a
        // no-op and cannot loop.
        self.model.active_tab_mut().pending_scroll = None;
        if (clamped - metrics.offset.y).abs() < 0.5 {
            // Already at the target — nothing to drive.
            return Task::none();
        }
        Task::done(Message::EditorAction(text_editor::Action::ScrollTo {
            offset: iced::Vector::new(0.0, clamped),
        }))
    }
}
