//! The large read-only capability policy: one list of commands that need a
//! complete, indexed buffer, shared by the view and the reducer.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Capability {
    Edit,
    Save,
    Find,
    FindStep,
    Replace,
    Goto,
    // Document statistics: characters, words, lines, bytes.
    Stats,
    WordWrap,
    LineEnding,
    Syntax,
}

impl Capability {
    #[must_use]
    pub(crate) fn label(self) -> String {
        match self {
            Self::Edit => enzui_core::localization::text("text-editing-fab453"),
            Self::Save => enzui_core::localization::text("text-saving-096b73"),
            Self::Find => enzui_core::localization::text("text-find-822b2a"),
            Self::FindStep => enzui_core::localization::text("text-find-next-previous-52bd0a"),
            Self::Replace => enzui_core::localization::text("text-replace-95e154"),
            Self::Goto => enzui_core::localization::text("text-go-to-line-77922d"),
            Self::Stats => enzui_core::localization::text("text-document-statistics-c61629"),
            Self::WordWrap => enzui_core::localization::text("text-word-wrap-9e210f"),
            Self::LineEnding => {
                enzui_core::localization::text("text-line-ending-conversion-c480ce")
            }
            Self::Syntax => enzui_core::localization::text("text-syntax-highlighting-a1aa07"),
        }
    }

    // Why a paged document cannot answer for it.
    #[must_use]
    pub(crate) fn reason(self) -> String {
        match self {
            Self::Edit | Self::Replace => {
                enzui_core::localization::text("text-there-is-no-editable-buffer-to-change-f94849")
            }
            Self::Save => {
                enzui_core::localization::text("text-a-paged-view-is-not-a-complete-08bcad")
            }
            Self::Find | Self::FindStep => {
                enzui_core::localization::text("text-the-document-is-not-indexed-so-a-0f588d")
            }
            Self::Goto => {
                enzui_core::localization::text("text-the-document-s-lines-are-not-indexed-ca0828")
            }
            Self::Stats => {
                enzui_core::localization::text("text-counting-would-have-to-read-the-whole-b9477e")
            }
            Self::WordWrap | Self::Syntax => enzui_core::localization::text(
                "text-the-paged-viewer-draws-unwrapped-unhighlighted-rows-982289",
            ),
            Self::LineEnding => {
                enzui_core::localization::text("text-nothing-can-be-written-back-af40c0")
            }
        }
    }
}

// Whether a `LargeReadOnly` document supports `capability`.
//
// It supports none; the exhaustive `match` forces a decision for each new
// variant.
#[must_use]
pub(crate) fn large_read_only_supports(capability: Capability) -> bool {
    match capability {
        Capability::Edit
        | Capability::Save
        | Capability::Find
        | Capability::FindStep
        | Capability::Replace
        | Capability::Goto
        | Capability::Stats
        | Capability::WordWrap
        | Capability::LineEnding
        | Capability::Syntax => false,
    }
}

// The sentence a refused control publishes on every route.
#[must_use]
pub(crate) fn capability_refused_sentence(capability: Capability, title: &str) -> String {
    let name = crate::safe_text::bounded_inline(title);
    enzui_core::localized!("text-p0-is-unavailable-name-is-open-read-1830d6", "p0" => capability.label(), "name" => name, "p1" => capability.reason())
}

// The find bar's counter cell on a read-only tab.
//
// Used instead of `SearchState::counter_label`, since nothing was searched.
pub(crate) const UNSEARCHABLE_COUNTER: &str = "Not searchable — large read-only document";

// Sentence shown when a write is refused because the tab is read-only.
#[must_use]
pub(crate) fn save_refused_sentence(path: &Path) -> String {
    let fallback = enzui_core::localization::text("text-this-item-545362");
    let name = crate::safe_text::bounded_inline(
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&fallback),
    );
    enzui_core::localized!("text-name-is-open-read-only-as-a-7fcb51", "name" => name)
}
