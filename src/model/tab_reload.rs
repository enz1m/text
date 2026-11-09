//! Tab caret-preserving reloads.

use super::*;

impl Tab {
    // The caret's byte offset into this tab's buffer.
    //
    // Read from the native `Content` cursor, which a buffer rebuild destroys,
    // rather than from any app-side model field.
    pub(crate) fn caret_byte(&self) -> usize {
        let position = self.editor_cursor().position;
        self.document
            .current()
            .byte_of_line_column(position.line, position.index)
    }

    // Reload a find/replace result while applying only its changed spans to a
    // viewport document's persistent store.
    //
    // `spans` use old-document coordinates and identify each replacement's
    // byte length in `new_text`. Applying them from the end prevents coordinate
    // drift and, unlike installing `new_text` as one source, does not make an
    // undo entry retain another complete large-document allocation.
    pub(crate) fn reload_preserving_caret_spans(
        &mut self,
        new_text: &str,
        caret_byte: usize,
        spans: &[crate::search::ReplaceSpan],
    ) -> VirtualWriteOutcome {
        let changed_bytes = spans.iter().fold(0usize, |total, span| {
            total
                .saturating_add(span.old_len())
                .saturating_add(span.new_len)
        });
        self.reload_preserving_caret_inner(new_text, caret_byte, changed_bytes, Some(spans))
    }

    pub(crate) fn reload_preserving_caret_inner(
        &mut self,
        new_text: &str,
        caret_byte: usize,
        changed_bytes: usize,
        spans: Option<&[crate::search::ReplaceSpan]>,
    ) -> VirtualWriteOutcome {
        let virtualized = self.virtual_editor.is_some();
        // Replace-all can delete newlines and join paragraphs past the bounded
        // projection, and `set_snapshot` is irreversible here, so refuse an
        // overlong line before the store, caret and dirty flag move.
        if let Some((line, bytes, limit)) = self.overlong_line_in(new_text) {
            return VirtualWriteOutcome::RefusedOverlongLine { line, bytes, limit };
        }
        // Reservation first: an exhausted version counter refuses before the
        // document, checkpoint, caret or dirty flag move, and surfaces as a
        // typed `AuthorityExhausted`.
        let Ok(reserved) = self.reserve_snapshot_version() else {
            return VirtualWriteOutcome::AuthorityExhausted;
        };
        if virtualized {
            let checkpoint = VirtualHistoryEntry {
                document: self.document.snapshot(),
                caret_byte: self.caret_byte(),
                cost: changed_bytes.max(1),
            };
            let write = if let Some(spans) = spans {
                let mut delta = 0_isize;
                let mut previous_end = 0;
                let mut replacements = Vec::with_capacity(spans.len());
                for (index, span) in spans.iter().enumerate() {
                    if span.old_start > span.old_end || (index > 0 && span.old_start < previous_end)
                    {
                        return VirtualWriteOutcome::Unchanged;
                    }
                    let new_start = span.old_start.saturating_add_signed(delta);
                    let new_end = new_start.saturating_add(span.new_len);
                    let Some(inserted) = new_text.get(new_start..new_end) else {
                        return VirtualWriteOutcome::Unchanged;
                    };
                    replacements.push((span.old_start..span.old_end, inserted.to_owned()));
                    delta = delta
                        .saturating_add(span.new_len as isize)
                        .saturating_sub(span.old_len() as isize);
                    previous_end = span.old_end;
                }
                let expected_len = self.document.current().len().saturating_add_signed(delta);
                if expected_len != new_text.len() {
                    return VirtualWriteOutcome::Unchanged;
                }
                self.document.replace_many(replacements)
            } else {
                let end = self.document.current().len();
                self.document.replace(0..end, new_text.to_owned())
            };
            if write.is_err() {
                // Reservation dropped: `snapshot_version` stays at pre-call
                // value; no cache flushed.
                return VirtualWriteOutcome::Unchanged;
            }
            self.commit_snapshot_version(reserved);
            self.stats_pending = true;
            self.dirty = true;
            let caret_byte = self
                .document
                .current()
                .floor_char_boundary(caret_byte.min(self.document.current().len()));
            let version = self.snapshot_version;
            if let Some(state) = &mut self.virtual_editor {
                state.cache.invalidate_from(version, 0);
                state.materialized = 0..0;
                state.projected = None;
                state.selection = VirtualSelection {
                    anchor_byte: None,
                    active_byte: caret_byte,
                };
            }
            self.push_virtual_undo(checkpoint);
            let line = self.document.current().line_column_of_byte(caret_byte).0;
            let first = line.saturating_sub(EDITABLE_VIEWPORT_LINES / 2);
            self.materialize_editor_window(first, Some(caret_byte));
            return VirtualWriteOutcome::Applied;
        }
        // Non-virtualized routes re-reserve inside their callee
        // (`apply_edit_delta` or `set_snapshot`), so this reservation is dropped.
        let _ = reserved;
        // An ordinary Replace All goes through `Edit::Replace(Arc<ReplaceBatch>)`
        // so the native editor records one undo transaction and keeps prior
        // history; `Content::with_text` would discard both.
        if let Some(spans) = spans
            && self.perform_replace_batch(new_text, caret_byte, spans)
        {
            return VirtualWriteOutcome::Applied;
        }
        self.reload_preserving_caret_by_snapshot(new_text, caret_byte)
    }

    pub(crate) fn reload_preserving_caret_by_snapshot(
        &mut self,
        new_text: &str,
        caret_byte: usize,
    ) -> VirtualWriteOutcome {
        // One reservation for the whole install, taken before touching content,
        // the dirty flag or the caret, so a refusal leaves them unchanged.
        let Ok(reserved) = self.reserve_snapshot_version() else {
            return VirtualWriteOutcome::AuthorityExhausted;
        };
        // Inline `set_snapshot` so the one reservation covers the whole route.
        self.document = TextStore::from_string(new_text.to_owned());
        self.commit_snapshot_version(reserved);
        if let Some(state) = &mut self.virtual_editor {
            state.cache.clear();
            state.materialized = 0..0;
            state.first_visible = 0;
            state.selection = VirtualSelection::default();
            state.projected = None;
            state.undo.clear();
            state.redo.clear();
            state.undo_bytes = 0;
        }
        self.dirty = true;
        let (line, column) = self.document.current().line_column_of_byte(caret_byte);
        {
            self.content = text_editor::Content::with_text(new_text);
            move_caret_to_position(
                &mut self.content,
                text_editor::Position {
                    line,
                    index: column,
                },
            );
        }
        VirtualWriteOutcome::Applied
    }
}
