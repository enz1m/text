//! TextApp action mapping and startup helpers.

use super::*;

// The only place chord-resolved actions become messages, so `keybindings`
// stays the single source of truth for labels and dispatch. `Open` uses the
// host's file chooser through the XDG Desktop Portal.
pub(crate) fn message_for_action(action: keybindings::EditorAction) -> Option<Message> {
    use keybindings::EditorAction as A;
    Some(match action {
        A::NewTab => Message::NewTab,
        A::Open => Message::OpenFile,
        A::Save => Message::SaveFile,
        A::SaveAs => Message::SaveFileAs,
        A::Print => Message::PrintDocument,
        A::CloseTab => Message::CloseActiveTab,
        A::Find => Message::OpenFind,
        A::Replace => Message::OpenReplace,
        A::GotoLine => Message::OpenGoto,
        A::Settings => Message::OpenSettings,
        A::Statistics => Message::OpenStats,
        A::FindNext => Message::FindNext,
        A::FindPrev => Message::FindPrev,
    })
}

pub(crate) fn guarded_window_close_message(
    command: WindowCommand,
    id: iced::window::Id,
) -> Option<Message> {
    (command == WindowCommand::Close).then(|| Message::CloseRequested(CloseRequest::for_window(id)))
}

// A launch path suppresses the remembered session and opens only the CLI
// paths as Open Files tabs; otherwise the whole mixed tab set is restored with
// its workspace associations.
pub(crate) fn restore_startup_tabs(
    model: &mut Model,
    saved: &session::SessionSnapshot,
    cli_paths: &[PathBuf],
) -> Vec<model::PendingOpen> {
    if !cli_paths.is_empty() {
        let pending = model.open_paths(cli_paths.iter().cloned());
        if !pending.is_empty() {
            // At least one CLI path opened; those files are this launch's tabs.
            return pending;
        }
        // Every CLI path was invalid, so fall through to session restore; a blank
        // model would otherwise overwrite the saved session on the first sync.
    }
    if model.settings.restore_session {
        // Only path and association are persisted; each tab's mode is
        // reclassified from current metadata by the returned loads.
        model.restore_persisted_tabs(&saved.tabs().tabs, saved.tabs().active_path.as_deref())
    } else {
        Vec::new()
    }
}
