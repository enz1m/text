//! TextApp virtual clipboard reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_clipboard_virtual_copy(&mut self) -> Task<Message> {
        let Some(text) = self.model.active_tab().virtual_selected_text() else {
            return Task::none();
        };
        iced::clipboard::write(text).map(|_| Message::NoOp)
    }
    pub(crate) fn reduce_clipboard_virtual_cut(&mut self) -> Task<Message> {
        if self.model.active_tab().is_read_only() {
            return Task::none();
        }
        // An exhausted tab admits no cut; the clipboard write is skipped too so
        // cut cannot degrade to copy.
        if self.refuse_if_authority_exhausted() {
            return Task::none();
        }
        // A cross-line cut can be refused; a refusal writes neither document nor
        // clipboard.
        let text = match self.model.active_tab_mut().cut_virtual_selection() {
            model::VirtualCutOutcome::Cut(text) => text,
            model::VirtualCutOutcome::Nothing => return Task::none(),
            model::VirtualCutOutcome::RefusedOverlongLine { line, bytes, limit } => {
                self.publish_overlong_refusal(line, bytes, limit);
                self.sync_cursor_readout();
                return Task::none();
            }
        };
        self.model.active_tab_mut().dirty = true;
        self.model.active_tab_mut().note_editor_action();
        if self.model.search.open {
            self.recompute_search();
        }
        self.sync_cursor_readout();
        iced::clipboard::write(text).map(|_| Message::NoOp)
    }
}
