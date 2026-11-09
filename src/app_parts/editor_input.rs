//! Editor input helpers.

use super::*;

// CRLF paste must not make the native editor disagree with the LF-normalized document.
pub(crate) fn normalize_pasted_line_endings(action: text_editor::Action) -> text_editor::Action {
    use iced::widget::text_editor::Edit;

    match action {
        text_editor::Action::Edit(Edit::Paste(pasted)) if pasted.contains('\r') => {
            text_editor::Action::Edit(Edit::Paste(std::sync::Arc::new(
                crate::line_ending::normalize(&pasted),
            )))
        }
        other => other,
    }
}

// Unselected Tab remains focus traversal; Escape exits the explicit indent mode.
pub(crate) fn editor_indent_binding(
    press: &text_editor::KeyPress,
    capture_tab: bool,
) -> Option<text_editor::Binding<Message>> {
    if !capture_tab {
        return None;
    }
    if !matches!(press.status, text_editor::Status::Focused { .. }) {
        return None;
    }
    if !matches!(press.key, keyboard::Key::Named(keyboard::key::Named::Tab)) {
        return None;
    }
    if press.modifiers.control() || press.modifiers.alt() || press.modifiers.logo() {
        return None;
    }
    let edit = if press.modifiers.shift() {
        text_editor::Edit::Unindent
    } else {
        text_editor::Edit::Indent
    };
    Some(text_editor::Binding::Custom(Message::EditorAction(
        text_editor::Action::Edit(edit),
    )))
}

// Word and line modes can select a range while the anchor equals the caret,
// so any non-`None` selection counts.
pub(crate) fn has_selection_anchor(content: &text_editor::Content) -> bool {
    content.cursor().selection.is_some()
}

pub(crate) fn editor_redo_binding(
    press: &text_editor::KeyPress,
) -> Option<text_editor::Binding<Message>> {
    (matches!(press.status, text_editor::Status::Focused { .. })
        && press.modifiers.control()
        && press.modifiers.shift()
        && !press.modifiers.alt()
        && !press.modifiers.logo()
        && press
            .key
            .to_latin(press.physical_key)
            .is_some_and(|key| key.eq_ignore_ascii_case(&'z')))
    .then_some(text_editor::Binding::Redo)
}
