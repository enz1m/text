//! TextApp recovery and restore state types.

use super::*;

#[derive(Clone, Debug)]
pub(crate) struct SessionRestorePending {
    pub(crate) cli_opened: bool,
    pub(crate) baseline: session::SessionSnapshot,
    pub(crate) debt: Option<session::SessionDebt>,
}

// The window close waits until the off-thread tombstone durably lands; a stale
// content-write completion cannot clear a pending discard.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RecoveryDiscard {
    // A tombstone write is in flight at `generation`; the window close and the
    // retirement of authority both wait for its durable completion.
    Pending {
        generation: u64,
        window: iced::window::Id,
    },
    // The tombstone write failed. The window stays open for Retry /
    // Close-without-clearing / Cancel; recovery is not retired.
    Failed {
        window: iced::window::Id,
    },
}

pub(crate) struct RecoveryBootReport {
    pub(crate) restored: usize,
    pub(crate) conflicts: Vec<String>,
    pub(crate) deleted: Vec<String>,
}

// Holds only an `Arc`-cloned [`enzui_core::chunked_text::TextSnapshot`]
// and O(1) metadata; [`RecoveryLease::into_record`] materialises the bounded
// content off the reducer thread.
#[derive(Clone)]
pub(crate) struct RecoveryLease {
    pub(crate) id: String,
    pub(crate) path: Option<std::path::PathBuf>,
    pub(crate) fingerprint: Option<recovery::DiskFingerprint>,
    pub(crate) caret: recovery::CaretMeta,
    pub(crate) line_ending: &'static str,
    pub(crate) language_override: Option<String>,
    pub(crate) snapshot: enzui_core::chunked_text::TextSnapshot,
}

impl RecoveryLease {
    pub(crate) fn into_record(self) -> recovery::RecoveryRecord {
        let byte_len = self.snapshot.len() as u64;
        let content = if self.snapshot.len() <= recovery::MAX_RECOVERY_CONTENT_BYTES {
            self.snapshot.to_string()
        } else {
            String::new()
        };
        recovery::RecoveryRecord::from_content(self.id, content, byte_len)
            .with_path(self.path, self.fingerprint)
            .with_caret(self.caret)
            .with_metadata("UTF-8", self.line_ending, self.language_override)
    }
}

pub(crate) fn recovery_lease_for(tab: &Tab) -> RecoveryLease {
    let cursor = tab.content.cursor();
    let caret = recovery::CaretMeta {
        cursor_line: cursor.position.line,
        cursor_column: cursor.position.index,
        selection_line: cursor.selection.map(|p| p.line),
        selection_column: cursor.selection.map(|p| p.index),
        // Every dirty buffer keeps its own viewport offset.
        scroll_offset: tab.editor_scroll_metrics().offset.y,
    };
    let line_ending = match tab.line_ending {
        crate::line_ending::LineEnding::Crlf => "CRLF",
        crate::line_ending::LineEnding::Cr => "CR",
        crate::line_ending::LineEnding::Mixed => "Mixed",
        crate::line_ending::LineEnding::Lf => "LF",
    };
    let fingerprint = tab
        .watcher
        .as_ref()
        .and_then(enzui_core::file_watch::FileWatcher::baseline)
        .map(recovery::DiskFingerprint::from_identity);
    RecoveryLease {
        id: tab.id.clone(),
        path: tab.file_path.clone(),
        fingerprint,
        caret,
        line_ending,
        language_override: tab.language_override.clone(),
        snapshot: tab.snapshot_handle(),
    }
}

// Selection direction is preserved. Each line is clamped into range; a column
// survives only with its line, and cosmic-text clamps it to the line end.
pub(crate) fn restore_caret_selection(
    content: &mut text_editor::Content,
    caret: &recovery::CaretMeta,
) {
    use iced::widget::text_editor::{Cursor, Position};
    let last_line = content.line_count().saturating_sub(1);
    let clamp = |line: usize, column: usize| -> Position {
        let l = line.min(last_line);
        let c = if l == line { column } else { 0 };
        Position { line: l, index: c }
    };
    let position = clamp(caret.cursor_line, caret.cursor_column);
    let selection = match (caret.selection_line, caret.selection_column) {
        (Some(sl), Some(sc)) => {
            let anchor = clamp(sl, sc);
            // A collapsed range is not a selection.
            if anchor.line == position.line && anchor.index == position.index {
                None
            } else {
                Some(anchor)
            }
        }
        _ => None,
    };
    content.move_to(Cursor {
        position,
        selection,
    });
}

pub(crate) fn existing_tab_for(
    model: &Model,
    rec: &recovery::RecoveryRecord,
    claimed: &std::collections::HashSet<usize>,
) -> Option<usize> {
    if let Some(index) = model.tabs.iter().enumerate().find_map(|(index, tab)| {
        (!claimed.contains(&index)
            && tab.id == rec.document_id
            && (tab.file_path == rec.original_path
                || (tab.file_path.is_none() && tab.document().is_empty() && !tab.dirty)))
            .then_some(index)
    }) {
        return Some(index);
    }
    let path = rec.original_path.as_deref()?;
    model.tabs.iter().enumerate().find_map(|(index, tab)| {
        (!claimed.contains(&index) && tab.file_path.as_deref() == Some(path)).then_some(index)
    })
}

pub(crate) fn recovered_tab_id(model: &mut Model, document_id: &str) -> String {
    if !model.tabs.iter().any(|tab| tab.id == document_id) {
        if let Some(sequence) = document_id
            .strip_prefix("tab-")
            .and_then(|value| value.parse::<u64>().ok())
            .and_then(|value| value.checked_add(1))
        {
            model.next_tab_seq = model.next_tab_seq.max(sequence);
        }
        return document_id.to_owned();
    }
    model.fresh_tab_id()
}
