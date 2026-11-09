//! Tab document snapshots.

use super::*;

impl Tab {
    // Statistics-dialog stats, cached against the buffer version; equal to
    // `crate::doc_stats::document_stats(self.snapshot())`.
    pub(crate) fn document_stats(&self) -> crate::doc_stats::Stats {
        if let Some((version, value)) = self.doc_stats_cache.get()
            && version == self.snapshot_version
        {
            return value;
        }
        let value = crate::doc_stats::document_stats_snapshot(self.document.current());
        self.doc_stats_cache
            .set(Some((self.snapshot_version, value)));
        value
    }

    // Materialize the tab's LF-normalized document text.
    //
    // Frame, search and save paths use [`Tab::document`] or
    // [`Tab::snapshot_handle`]; this serves whole-buffer replaces and tests.
    pub(crate) fn snapshot(&self) -> String {
        #[cfg(test)]
        crate::perf_counters::bump_snapshot_materialization();
        self.document.current().to_string()
    }

    // Borrow the current chunked generation without flattening it.
    pub(crate) fn document(&self) -> &TextSnapshot {
        self.document.current()
    }

    pub(crate) fn snapshot_handle(&self) -> TextSnapshot {
        self.document.snapshot()
    }

    // Install a whole-document replacement and invalidate derived caches.
    //
    // Every whole-buffer install goes through here. When the snapshot version
    // is exhausted the call returns without replacing the document, clearing
    // undo/redo or touching caches.
    pub(crate) fn set_snapshot(&mut self, text: impl Into<String>) {
        let Ok(reserved) = self.reserve_snapshot_version() else {
            // Never replace bytes under a reused ceiling version; callers
            // preflight via [`Tab::authority_exhausted`].
            return;
        };
        self.document = TextStore::from_string(text.into());
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
    }

    // Whether this tab's native editor is a bounded projection of the
    // authoritative chunked document.
    #[must_use]
    pub(crate) fn uses_viewport_editor(&self) -> bool {
        self.virtual_editor.is_some()
    }

    #[must_use]
    pub(crate) fn editor_line_origin(&self) -> usize {
        self.virtual_editor
            .as_ref()
            .map_or(0, |state| state.materialized.start)
    }

    // Caret and anchor in complete-document coordinates. In viewport mode
    // this is app-owned state: the anchor may be thousands of lines outside
    // the native editor window and must survive every recenter.
    #[must_use]
    pub(crate) fn editor_cursor(&self) -> text_editor::Cursor {
        let Some(state) = &self.virtual_editor else {
            return self.content.cursor();
        };
        let position_at = |byte| {
            let (line, column) = self.document.current().line_column_of_byte(byte);
            text_editor::Position {
                line,
                index: column,
            }
        };
        text_editor::Cursor {
            position: position_at(state.selection.active_byte),
            selection: state.selection.anchor_byte.map(position_at),
        }
    }

    // Select the document bytes `start..end` and bring them into view, as
    // Find's Next and Previous do: the anchor at `start`, the caret at `end`,
    // so the editor scrolls to the caret like any other caret move.
    pub(crate) fn reveal_byte_range(&mut self, start: usize, end: usize) {
        let snapshot = self.document.current();
        let end = snapshot.floor_char_boundary(end.min(snapshot.len()));
        let start = snapshot.floor_char_boundary(start.min(end));
        let position_at = |byte| {
            let (line, index) = snapshot.line_column_of_byte(byte);
            text_editor::Position { line, index }
        };
        let (anchor, caret) = (position_at(start), position_at(end));
        let Some(state) = &mut self.virtual_editor else {
            self.content.move_to(text_editor::Cursor {
                position: caret,
                selection: (start != end).then_some(anchor),
            });
            return;
        };
        state.selection = VirtualSelection {
            anchor_byte: (start != end).then_some(start),
            active_byte: end,
        };
        let first = caret.line.saturating_sub(EDITABLE_VIEWPORT_LINES / 2);
        self.materialize_editor_window(first, Some(end));
    }

    pub(crate) fn reveal_document_line(&mut self, line: usize) {
        if self.virtual_editor.is_none() {
            move_caret_to_position(&mut self.content, text_editor::Position { line, index: 0 });
            return;
        }
        let line = line.min(self.document.current().line_count().saturating_sub(1));
        let first = line.saturating_sub(EDITABLE_VIEWPORT_LINES / 2);
        let byte = self.document.current().line_start(line);
        self.materialize_editor_window(first, byte);
    }

    // Rebuild the chunked document by flattening the Content buffer.
    //
    // `O(document)`: the fallback for deltas that did not address the snapshot
    // and for whole-buffer rebuilds. Edits use [`Tab::apply_edit_delta`].
    pub(crate) fn sync_snapshot(&mut self) {
        if self.virtual_editor.is_some() {
            // A bounded projection is never the complete document, so a bad
            // delta rebuilds the projection instead of replacing the store.
            let first = self
                .virtual_editor
                .as_ref()
                .map_or(0, |state| state.first_visible);
            self.materialize_editor_window(first, None);
            return;
        }
        let flattened = self.content.text();
        self.set_snapshot(flattened);
        #[cfg(test)]
        crate::perf_counters::bump_snapshot_flatten();
    }
}
