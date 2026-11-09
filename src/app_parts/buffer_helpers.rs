//! TextApp buffer, cursor and presentation helpers.

use super::*;

impl TextApp {
    pub(crate) fn goto_line(&mut self, line: usize) {
        let total = self.model.active_tab().total_lines();
        let clamped = line.clamp(1, total.max(1));
        // `Motion::Down` advances one visual row once cosmic-text has shaped a
        // wrapped editor, so counting Down motions undershoots any logical line
        // below a wrapped one. Resolve the logical line through the tab's line
        // index (no document scan, no document copy) and set the caret directly.
        let tab = self.model.active_tab_mut();
        tab.reveal_document_line(clamped - 1);
        self.sync_cursor_readout();
    }

    // Every path that moves or rebuilds the caret ends here, so the readout
    // cannot drift from the native cursor.
    pub(crate) fn sync_cursor_readout(&mut self) {
        let tab = self.model.active_tab();
        let line = tab.editor_cursor().position.line + 1;
        // The native column is a byte offset; the status bar's character column
        // is derived here from the current line only.
        let column = tab.document().character_column(tab.caret_byte());
        self.model.cursor_line = line;
        self.model.cursor_col = column;
    }

    // The caret is mapped through `spans` (old-buffer byte coordinates) by
    // [`search::map_offset`], shifting it by the length delta of replacements
    // before it so it stays on the same text. Jumping to the last replacement
    // or keeping the raw byte offset would both move the user.
    pub(crate) fn load_active_buffer_mapped(
        &mut self,
        new_text: &str,
        spans: &[search::ReplaceSpan],
    ) {
        let tab = self.model.active_tab_mut();
        let target = search::map_offset(spans, tab.caret_byte());
        // A viewport replace is undoable. Apply its spans piece-wise so
        // history generations share unchanged source chunks rather than each
        // retaining this complete replacement allocation.
        let outcome = tab.reload_preserving_caret_spans(new_text, target, spans);
        if let Some((line, bytes, limit)) = outcome.refusal() {
            // The buffer is untouched; say so rather than leaving the
            // user with a find/replace that silently did nothing.
            self.publish_overlong_refusal(line, bytes, limit);
        }
        self.sync_cursor_readout();
    }

    pub(crate) fn publish_overlong_refusal(&mut self, line: usize, bytes: usize, limit: usize) {
        self.picker.failure = Some(model::overlong_line_refused_sentence(line, bytes, limit));
    }

    pub(crate) fn palette(&self) -> Palette {
        self.themes.theme().palette.clone()
    }

    pub(crate) fn spacing(&self) -> SpacingScale {
        self.themes.theme().spacing
    }

    pub(crate) fn typography(&self) -> TypographyScale {
        self.themes.theme().typography
    }

    // The modal host owns Tab; suppress the shell focus ring while a dialog is open.
    pub(crate) fn a_dialog_is_open(&self) -> bool {
        self.picker.fallback_is_open()
            || matches!(self.recovery_discard, Some(RecoveryDiscard::Failed { .. }))
            || self.model.pending_close.is_some()
            || self.pending_workspace_remove.is_some()
            || self.close_guard.is_pending()
            || self.model.stats_open
            || self.model.settings_open
            || self.model.shortcuts_open
    }

    pub(crate) fn sidebar_presentation(&self) -> SidebarPresentation {
        let width = self
            .model
            .window_size
            .map_or(crate::model::DEFAULT_WINDOW_WIDTH, |(w, _)| w);
        Self::sidebar_presentation_for(self.nav, width)
    }

    pub(crate) fn sidebar_presentation_for(
        nav: NavSplitState,
        window_width: f32,
    ) -> SidebarPresentation {
        enzui_widgets::app_shell::resolve_shell_geometry(
            ShellLayout::TitlebarSidebarContent,
            nav,
            shell::shell_tokens().with_flush_defaults(ShellLayout::TitlebarSidebarContent),
            Default::default(),
            window_width,
        )
        .sidebar
    }

    pub(crate) fn event_classifier(&self) -> EventClassifier {
        // Modal and drawer hosts own their own Escape routing. For app-owned
        // transient surfaces, select the topmost rendered one; no open surface
        // uses the base classifier, whose Escape result is None.
        if self.picker.fallback_is_open() {
            |event, status, id| match event {
                iced::Event::Keyboard(_) => None,
                other => classify_event(other, status, id),
            }
        } else if self.a_dialog_is_open() {
            classify_event_modal_owns_keyboard
        } else if self.model.language_menu_open
            || self.model.overflow_menu_open
            || self.model.tab_list_open
        {
            classify_event_dismiss_popovers
        } else if self.model.goto_open {
            classify_event_close_goto
        } else if self.model.search.open {
            classify_event_close_find
        } else {
            classify_event
        }
    }
}
