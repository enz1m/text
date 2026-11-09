//! Tab edit deltas.

use super::*;

impl Tab {
    // Apply one editor edit delta to the snapshot and its line index in work
    // proportional to the edit, without a document walk or copy.
    //
    // Returns the inserted/deleted byte counts, or `None` when the delta did
    // not address the snapshot; the caller must then fall back to
    // [`Tab::sync_snapshot`].
    pub(crate) fn apply_edit_delta(
        &mut self,
        delta: &iced::widget::text_editor::EditDelta,
    ) -> EditDeltaOutcome {
        let mut bytes = crate::doc_index::DeltaBytes::default();
        let line_origin = self.editor_line_origin();
        let mut first_affected_line = usize::MAX;
        // O(1) pre-edit generation; a refusal below restores it.
        let rollback = self.document.snapshot();
        let mut refusal = None;
        // Reserve the version before mutating: reusing a version for different
        // bytes would let a stale worker completion describe the new document.
        let pre_delta_version = self.snapshot_version;
        let Ok(reserved) = self.reserve_snapshot_version() else {
            return EditDeltaOutcome::AuthorityExhausted;
        };
        // Advance without clearing caches so patched counts bind to the new
        // version; a refusal restores both bytes and version.
        self.snapshot_version = reserved;

        for item in delta.items() {
            // Patch statistics from the whitespace-bounded region around the
            // edit, keeping word counts exact without a document walk.
            let snapshot = self.document.snapshot();
            let global_start_line = item.start.line.saturating_add(line_origin);
            let global_end_line = item.end.line.saturating_add(line_origin);
            first_affected_line = first_affected_line.min(global_start_line);
            let Some(start) =
                snapshot.checked_byte_of_line_column(global_start_line, item.start.index)
            else {
                return self.abandon_edit_delta(rollback, pre_delta_version);
            };
            let (end, deleted, inserted) = match item.kind {
                iced::widget::text_editor::EditKind::Insert => (start, "", item.text.as_str()),
                iced::widget::text_editor::EditKind::Delete => {
                    let Some(end) =
                        snapshot.checked_byte_of_line_column(global_end_line, item.end.index)
                    else {
                        return self.abandon_edit_delta(rollback, pre_delta_version);
                    };
                    if end < start || !snapshot.range_eq(start..end, &item.text) {
                        return self.abandon_edit_delta(rollback, pre_delta_version);
                    }
                    (end, item.text.as_str(), "")
                }
            };

            // Only the changed bytes and one character each side: the enclosing
            // word may be the whole file. Deleted bytes come from `EditItem`.
            let counts = edit_counts(
                snapshot.char_before(start),
                deleted,
                inserted,
                snapshot.char_at(end),
            );
            if self
                .document
                .replace(start..end, inserted.to_owned())
                .is_err()
            {
                return self.abandon_edit_delta(rollback, pre_delta_version);
            }
            self.patch_stats(counts);

            bytes.inserted += inserted.len();
            bytes.deleted += end - start;

            // Only lines this item wrote can exceed the paragraph limit, such as
            // a `Backspace` joining two admitted lines.
            if refusal.is_none() {
                refusal = self.overlong_line_after_edit(start, inserted.len());
            }
        }

        if let Some((line, line_bytes, limit)) = refusal {
            self.restore_pre_delta_document(rollback, pre_delta_version);
            return EditDeltaOutcome::RefusedOverlongLine {
                line,
                bytes: line_bytes,
                limit,
            };
        }

        // `Applied` is the commit. No cache flush: it would discard the patched
        // counts, and caches on the old version self-invalidate.
        let _reserved = reserved;

        if let Some(state) = &mut self.virtual_editor {
            if first_affected_line != usize::MAX {
                state
                    .cache
                    .invalidate_from(self.snapshot_version, first_affected_line);
            }
            state.materialized.end = state
                .materialized
                .start
                .saturating_add(self.content.line_count());
        }

        EditDeltaOutcome::Applied(bytes)
    }

    // Items before the failing one are already written, so the pre-delta
    // generation is restored first; the caller then resynchronizes the native
    // buffer against it.
    pub(crate) fn abandon_edit_delta(
        &mut self,
        rollback: TextSnapshot,
        pre_delta_version: u64,
    ) -> EditDeltaOutcome {
        self.restore_pre_delta_document(rollback, pre_delta_version);
        EditDeltaOutcome::Drifted
    }

    pub(crate) fn restore_pre_delta_document(
        &mut self,
        rollback: TextSnapshot,
        pre_delta_version: u64,
    ) {
        self.document.restore(rollback);
        self.snapshot_version = pre_delta_version;
        self.text_stats_cache.set(None);
        self.doc_stats_cache.set(None);
        self.line_count_cache.set(None);
        self.stats_pending = true;
        if let Some(state) = &mut self.virtual_editor {
            state.cache.clear();
            state.materialized = 0..0;
            state.projected = None;
        }
    }

    // Cost is proportional to the edit. Ordinary buffers have no limit and
    // return `None`.
    pub(crate) fn overlong_line_after_edit(
        &self,
        start: usize,
        inserted_len: usize,
    ) -> Option<(usize, usize, usize)> {
        let limit = self.virtual_editor.as_ref()?.cache.policy().max_line_bytes;
        let snapshot = self.document.current();
        let first = snapshot.line_column_of_byte(start).0;
        let last = snapshot
            .line_column_of_byte(start.saturating_add(inserted_len))
            .0;
        (first..=last).find_map(|line| {
            let begin = snapshot.line_start(line)?;
            let finish = snapshot.line_end(line)?;
            let bytes = finish.saturating_sub(begin);
            (bytes > limit).then_some((line, bytes, limit))
        })
    }

    pub(crate) fn patch_stats(&mut self, counts: EditCounts) {
        let version = self.snapshot_version;
        let lines = self.display_lines();
        let shift = |value: usize, by: isize| value.saturating_add_signed(by);

        if let Some((_, mut stats)) = self.text_stats_cache.get() {
            stats.chars = shift(stats.chars, counts.chars);
            stats.words = shift(stats.words, counts.words);
            stats.lines = lines;
            self.text_stats_cache.set(Some((version, stats)));
        }
        if let Some((_, mut stats)) = self.doc_stats_cache.get() {
            stats.chars = shift(stats.chars, counts.chars);
            stats.chars_no_spaces = shift(stats.chars_no_spaces, counts.visible);
            stats.words = shift(stats.words, counts.words);
            stats.lines = lines;
            stats.bytes = self.document.current().len();
            self.doc_stats_cache.set(Some((version, stats)));
        }
        self.line_count_cache
            .set(Some((version, self.document.current().line_count())));
    }
}
