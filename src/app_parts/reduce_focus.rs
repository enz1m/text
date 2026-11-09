//! TextApp focus and window-close reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_focus_focus_next(&mut self) -> Task<Message> {
        // A wrapping focus operation: iced's stock one unfocuses at the last
        // control. An open dialog's host owns Tab instead.
        if self.a_dialog_is_open() {
            Task::none()
        } else {
            operate(traverse::<Message>(Traversal::Next))
        }
    }
    pub(crate) fn reduce_focus_focus_previous(&mut self) -> Task<Message> {
        if self.a_dialog_is_open() {
            Task::none()
        } else {
            operate(traverse::<Message>(Traversal::Previous))
        }
    }
    pub(crate) fn reduce_focus_close_requested(&mut self, request: CloseRequest) -> Task<Message> {
        if self.picker.fallback_is_open() {
            return self
                .picker
                .handle(enzui_widgets::file_chooser::FallbackEvent::Cancel);
        }
        self.close_request_window = Some(request.window());
        let dirty = self
            .model
            .tabs
            .iter()
            .filter(|tab| tab.dirty)
            .map(|tab| {
                UnsavedItem::new(
                    tab.id.clone(),
                    crate::safe_text::bounded_inline(&tab.title()),
                )
            })
            .collect::<Vec<_>>();
        let action = self.close_guard.request(request, dirty);
        self.apply_close_guard_action(action)
    }
    pub(crate) fn reduce_focus_window_close_save_all(&mut self) -> Task<Message> {
        let action = self.close_guard.decide(CloseDecision::SaveAll);
        self.apply_close_guard_action(action)
    }
    pub(crate) fn reduce_focus_window_close_discard(&mut self) -> Task<Message> {
        let action = self.close_guard.decide(CloseDecision::Discard);
        if let CloseGuardAction::Close(window) = &action {
            let window = *window;
            let drain = self.close_window(window);
            return Task::batch([drain, self.begin_session_discard(window)]);
        }
        self.apply_close_guard_action(action)
    }
    pub(crate) fn reduce_focus_window_close_cancel(&mut self) -> Task<Message> {
        self.close_drain = None;
        self.close_request_window = None;
        let action = self.close_guard.decide(CloseDecision::Cancel);
        self.apply_close_guard_action(action)
    }
    pub(crate) fn reduce_focus_window(&mut self, command: WindowCommand) -> Task<Message> {
        // The titlebar X must take the same held-close path as a compositor
        // close; `WindowCommand::Close.into_task` would bypass the dirty guard.
        iced::window::latest().then(move |maybe_id| match maybe_id {
            Some(id) => match guarded_window_close_message(command, id) {
                Some(message) => Task::done(message),
                None => command.into_task(id),
            },
            None => Task::none(),
        })
    }
}
