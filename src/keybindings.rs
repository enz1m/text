//! Editor keyboard shortcuts — one framework catalog for dispatch and help.
//!
//! Every command chord, label, alternate, and live-context rule is declared in
//! [`ALL_BINDINGS`]. Keyboard routing goes through
//! [`enzui_core::dispatch_shortcut_command`], while inline hints and the
//! shortcuts dialog derive from [`enzui_core::shortcut_help`]. This keeps
//! matching, aliases, repeat suppression, and presentation on the framework's
//! catalog model instead of a text-app copy.

use enzui_core::shortcut::{Modifier, Shortcut};
use enzui_core::{ShortcutBinding, dispatch_shortcut_command, shortcut_help};
use enzui_widgets::shortcut_list::ShortcutEntry;
use iced::keyboard::{self, key::Named};
use std::sync::LazyLock;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EditorAction {
    NewTab,
    Open,
    Save,
    SaveAs,
    Print,
    CloseTab,
    Find,
    Replace,
    GotoLine,
    Settings,
    Statistics,
    FindNext,
    FindPrev,
}

// Modal ownership is independent of whether a focused widget captured the key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ShortcutContext {
    pub find_open: bool,
    pub modal_owns_keyboard: bool,
}

impl ShortcutContext {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "ShortcutBinding predicates require fn(&Context) -> bool"
    )]
    #[must_use]
    pub(crate) fn is_global(context: &Self) -> bool {
        !context.modal_owns_keyboard
    }

    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "ShortcutBinding predicates require fn(&Context) -> bool"
    )]
    #[must_use]
    pub(crate) fn find_row_active(context: &Self) -> bool {
        context.find_open && !context.modal_owns_keyboard
    }
}

pub(crate) type EditorBinding = ShortcutBinding<EditorAction, ShortcutContext>;

// A distinct command type prevents catalog dispatch from duplicating editor-owned actions.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct WidgetOwned;

pub(crate) type WidgetOwnedBinding = ShortcutBinding<WidgetOwned, ()>;

fn command(
    id: &'static str,
    action: EditorAction,
    label: &'static str,
    shortcut: Shortcut,
) -> EditorBinding {
    let bare: ShortcutBinding<EditorAction, ()> =
        ShortcutBinding::new_in_context(id, action, shortcut);
    bare.with_description(crate::i18n::domain_text(label))
        .when(ShortcutContext::is_global)
}

fn editor_owned(id: &'static str, label: &'static str, shortcut: Shortcut) -> WidgetOwnedBinding {
    ShortcutBinding::new(id, WidgetOwned, shortcut)
        .with_description(crate::i18n::domain_text(label))
        .with_enabled(false)
}

// Character shortcuts allocate, so the catalog is initialized on first use.
pub(crate) static ALL_BINDINGS: LazyLock<Vec<EditorBinding>> = LazyLock::new(|| {
    vec![
        command(
            "editor.new-tab",
            EditorAction::NewTab,
            "New tab",
            Shortcut::new(Modifier::CTRL, 'n'),
        )
        .with_alternate(Shortcut::new(Modifier::CTRL, 't')),
        command(
            "file.open",
            EditorAction::Open,
            "Open",
            Shortcut::new(Modifier::CTRL, 'o'),
        ),
        command(
            "file.save",
            EditorAction::Save,
            "Save",
            Shortcut::new(Modifier::CTRL, 's'),
        ),
        command(
            "file.save-as",
            EditorAction::SaveAs,
            "Save As",
            Shortcut::new(Modifier::CTRL | Modifier::SHIFT, 's'),
        ),
        command(
            "file.print",
            EditorAction::Print,
            "Print",
            Shortcut::new(Modifier::CTRL, 'p'),
        ),
        command(
            "editor.close-tab",
            EditorAction::CloseTab,
            "Close tab",
            Shortcut::new(Modifier::CTRL, 'w'),
        ),
        command(
            "find.open",
            EditorAction::Find,
            "Find",
            Shortcut::new(Modifier::CTRL, 'f'),
        ),
        command(
            "find.replace",
            EditorAction::Replace,
            "Replace",
            Shortcut::new(Modifier::CTRL, 'h'),
        ),
        command(
            "editor.goto-line",
            EditorAction::GotoLine,
            "Go to line",
            Shortcut::new(Modifier::CTRL, 'g'),
        ),
        command(
            "app.settings",
            EditorAction::Settings,
            "Settings",
            Shortcut::new(Modifier::CTRL, ','),
        ),
        command(
            "document.statistics",
            EditorAction::Statistics,
            "Document Statistics",
            Shortcut::new(Modifier::CTRL | Modifier::SHIFT, 'd'),
        ),
        command(
            "find.next",
            EditorAction::FindNext,
            "Next match",
            Shortcut::new(Modifier::NONE, Named::Enter),
        )
        .with_alternate(Shortcut::new(Modifier::NONE, Named::F3))
        .when(ShortcutContext::find_row_active),
        command(
            "find.previous",
            EditorAction::FindPrev,
            "Previous match",
            Shortcut::new(Modifier::SHIFT, Named::Enter),
        )
        .with_alternate(Shortcut::new(Modifier::SHIFT, Named::F3))
        .when(ShortcutContext::find_row_active),
    ]
});

// Editor-owned shortcuts share help metadata; Escape releases Tab-indent mode for focus traversal.
pub(crate) static EDITOR_OWNED_BINDINGS: LazyLock<Vec<WidgetOwnedBinding>> = LazyLock::new(|| {
    vec![
        editor_owned("editor.undo", "Undo", Shortcut::new(Modifier::CTRL, 'z')),
        editor_owned("editor.redo", "Redo", Shortcut::new(Modifier::CTRL, 'y'))
            .with_alternate(Shortcut::new(Modifier::CTRL | Modifier::SHIFT, 'z')),
        editor_owned(
            "editor.select-all",
            "Select all",
            Shortcut::new(Modifier::CTRL, 'a'),
        ),
        editor_owned("editor.copy", "Copy", Shortcut::new(Modifier::CTRL, 'c')),
        editor_owned("editor.cut", "Cut", Shortcut::new(Modifier::CTRL, 'x')),
        editor_owned("editor.paste", "Paste", Shortcut::new(Modifier::CTRL, 'v')),
        editor_owned(
            "editor.indent",
            "Indent selection",
            Shortcut::new(Modifier::NONE, Named::Tab),
        ),
        editor_owned(
            "editor.unindent",
            "Unindent selection",
            Shortcut::new(Modifier::SHIFT, Named::Tab),
        ),
        editor_owned(
            "editor.tab-mode",
            "Toggle Tab indentation",
            Shortcut::new(Modifier::CTRL, 'm'),
        ),
        editor_owned(
            "editor.focus-out",
            "Move focus out of editor",
            Shortcut::new(Modifier::NONE, Named::Escape),
        ),
    ]
});

pub(crate) fn bindings() -> &'static [EditorBinding] {
    ALL_BINDINGS.as_slice()
}

pub(crate) fn editor_owned_bindings() -> &'static [WidgetOwnedBinding] {
    EDITOR_OWNED_BINDINGS.as_slice()
}

#[must_use]
pub(crate) fn dispatch(event: &keyboard::Event, context: ShortcutContext) -> Option<EditorAction> {
    dispatch_shortcut_command(bindings(), event, &context)
}

fn dialog_rows_from<Command, Context>(
    bindings: &[ShortcutBinding<Command, Context>],
) -> Vec<ShortcutEntry> {
    shortcut_help(bindings.iter())
        .into_iter()
        .map(|row| {
            let chords = row
                .shortcuts()
                .map(Shortcut::localized_label)
                .collect::<Vec<_>>()
                .join(", ");
            ShortcutEntry::new(row.section(), chords, row.description()).stable_key(row.id())
        })
        .collect()
}

#[must_use]
pub(crate) fn shortcut_dialog_entries() -> Vec<ShortcutEntry> {
    let mut entries = dialog_rows_from(bindings());
    entries.extend(dialog_rows_from(editor_owned_bindings()));
    entries
}

#[must_use]
pub(crate) fn binding(action: EditorAction) -> Option<&'static EditorBinding> {
    bindings()
        .iter()
        .find(|binding| binding.command() == &action)
}

#[must_use]
pub(crate) fn chord_for(action: EditorAction) -> String {
    binding(action)
        .map(|binding| {
            binding
                .shortcuts()
                .map(Shortcut::localized_label)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

#[must_use]
pub(crate) fn label_with_chord(action: EditorAction) -> String {
    binding(action)
        .map(|binding| enzui_core::localized!("text-0841818d307c", "label" => binding.description(), "shortcut" => chord_for(action)))
        .unwrap_or_default()
}
