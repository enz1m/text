//! TextApp raw event classification helpers.

use super::*;

// Captured Tab belongs to the focused editor; modified Tab remains tab switching.
#[derive(Clone, Copy)]
pub(crate) enum EscapeTarget {
    Find,
    Goto,
    Popovers,
}

pub(crate) type EventClassifier =
    fn(iced::Event, iced::event::Status, iced::window::Id) -> Option<Message>;

pub(crate) fn escape_message(target: Option<EscapeTarget>) -> Option<Message> {
    use enzui_core::dismissal::{DismissPolicy, on_escape};
    let target = target?;
    let policy = match target {
        EscapeTarget::Popovers => DismissPolicy::popover(),
        EscapeTarget::Find | EscapeTarget::Goto => DismissPolicy::transient(),
    };
    on_escape(policy).closes().then(|| match target {
        EscapeTarget::Find => Message::CloseFind,
        EscapeTarget::Goto => Message::CloseGoto,
        EscapeTarget::Popovers => Message::DismissPopovers,
    })
}

pub(crate) fn classify_event(
    event: iced::Event,
    status: iced::event::Status,
    id: iced::window::Id,
) -> Option<Message> {
    classify_event_for_escape(event, status, id, None, false)
}

// Modal-local navigation and help remain available outside the shell catalog.
pub(crate) fn classify_event_modal_owns_keyboard(
    event: iced::Event,
    status: iced::event::Status,
    id: iced::window::Id,
) -> Option<Message> {
    classify_event_for_escape(event, status, id, None, true)
}

pub(crate) fn classify_event_close_find(
    event: iced::Event,
    status: iced::event::Status,
    id: iced::window::Id,
) -> Option<Message> {
    classify_event_for_escape(event, status, id, Some(EscapeTarget::Find), false)
}

pub(crate) fn classify_event_close_goto(
    event: iced::Event,
    status: iced::event::Status,
    id: iced::window::Id,
) -> Option<Message> {
    classify_event_for_escape(event, status, id, Some(EscapeTarget::Goto), false)
}

pub(crate) fn classify_event_dismiss_popovers(
    event: iced::Event,
    status: iced::event::Status,
    id: iced::window::Id,
) -> Option<Message> {
    classify_event_for_escape(event, status, id, Some(EscapeTarget::Popovers), false)
}

pub(crate) fn classify_event_for_escape(
    event: iced::Event,
    status: iced::event::Status,
    _id: iced::window::Id,
    escape_target: Option<EscapeTarget>,
    modal_owns_keyboard: bool,
) -> Option<Message> {
    match &event {
        iced::Event::Window(iced::window::Event::Opened { .. }) => {
            return Some(Message::Desktop(
                crate::desktop::DesktopEvent::WindowOpened(_id),
            ));
        }
        iced::Event::Window(iced::window::Event::Closed) => {
            return Some(Message::Desktop(
                crate::desktop::DesktopEvent::WindowClosed(_id),
            ));
        }
        iced::Event::Window(iced::window::Event::FileDropped(path)) => {
            return Some(Message::Desktop(crate::desktop::DesktopEvent::Paths(vec![
                path.clone(),
            ])));
        }
        iced::Event::Window(iced::window::Event::Focused) => {
            return Some(Message::WindowFocusChanged(true));
        }
        iced::Event::Window(iced::window::Event::Unfocused) => {
            return Some(Message::WindowFocusChanged(false));
        }
        iced::Event::Window(iced::window::Event::Resized(size)) => {
            return Some(Message::Resized(size.width, size.height));
        }
        iced::Event::Window(iced::window::Event::CloseRequested) => {
            return CloseRequest::from_event(&event, _id).map(Message::CloseRequested);
        }
        iced::Event::Keyboard(
            key_event @ keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                ..
            },
        ) => {
            // Tab chords and Escape are navigation keys, kept out of the
            // `keybindings` command table.
            if modifiers.control()
                && let keyboard::Key::Named(keyboard::key::Named::Tab) = key
            {
                // Ctrl+Tab / Ctrl+Shift+Tab: switch tabs. The shared classifier
                // below declines any Tab carrying Ctrl, so this chord is never
                // rerouted to focus traversal.
                if modifiers.shift() {
                    return Some(Message::PrevTab);
                }
                return Some(Message::NextTab);
            }
            // Bare Tab / Shift+Tab traversal comes from the shared framework
            // classifier.
            if let Some(traversal) = traversal_from_event(&event, status) {
                return Some(match traversal {
                    Traversal::Next => Message::FocusNext,
                    Traversal::Previous => Message::FocusPrevious,
                });
            }
            if let keyboard::Key::Named(keyboard::key::Named::Escape) = key {
                // The subscription selects the rendered topmost surface. Its
                // framework DismissPolicy decides whether Escape closes; with
                // no surface the target is None and no message is published.
                return escape_message(escape_target);
            }

            // F1 and Ctrl+/ open the shortcuts dialog, its only keyboard route.
            // They stay out of the `keybindings` table so its count gate lists
            // editor commands only.
            if matches!(key, keyboard::Key::Named(keyboard::key::Named::F1)) {
                return Some(Message::OpenShortcuts);
            }
            if modifiers.control() && !modifiers.shift() && key.to_latin(*physical_key) == Some('/')
            {
                return Some(Message::OpenShortcuts);
            }
            // Ctrl+M toggles the editor Tab mode. It is a mode key, not an
            // `EditorAction`, so it stays out of the dispatched catalog; the
            // focused editor leaves Ctrl+M `Ignored`, so it reaches here.
            if modifiers.control()
                && !modifiers.shift()
                && !modifiers.alt()
                && !modifiers.logo()
                && key
                    .to_latin(*physical_key)
                    .is_some_and(|c| c.eq_ignore_ascii_case(&'m'))
            {
                return Some(Message::ToggleTabIndentMode);
            }

            // Captured Enter is handled by the find field; only ignored keys reach this route.
            let is_bare_named = matches!(key, keyboard::Key::Named(_)) && !modifiers.control();
            if is_bare_named && !matches!(status, iced::event::Status::Ignored) {
                return None;
            }
            let shortcut_context = keybindings::ShortcutContext {
                find_open: matches!(escape_target, Some(EscapeTarget::Find)),
                modal_owns_keyboard,
            };
            if let Some(action) = keybindings::dispatch(key_event, shortcut_context) {
                return message_for_action(action);
            }
        }
        _ => {}
    }
    None
}
