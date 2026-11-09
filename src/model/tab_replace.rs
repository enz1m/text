//! Tab replace batches.

use super::*;

impl Tab {
    // Returns `true` when the batch was submitted and applied to the
    // authoritative mirror. A malformed span plan returns `false` without
    // submitting, so the caller can fall back to a snapshot install.
    pub(crate) fn perform_replace_batch(
        &mut self,
        new_text: &str,
        caret_byte: usize,
        spans: &[crate::search::ReplaceSpan],
    ) -> bool {
        use std::sync::Arc;

        let doc = self.document.current();
        let doc_len = doc.len();
        let mut replacements = Vec::with_capacity(spans.len());
        let mut length_delta: isize = 0;
        let mut previous_end = 0usize;
        for (index, span) in spans.iter().enumerate() {
            if span.old_start > span.old_end
                || span.old_end > doc_len
                || (index > 0 && span.old_start < previous_end)
            {
                return false;
            }
            let new_start = span.old_start.saturating_add_signed(length_delta);
            let new_end = new_start.saturating_add(span.new_len);
            let Some(inserted) = new_text.get(new_start..new_end) else {
                return false;
            };
            let (start_line, start_col) = doc.line_column_of_byte(span.old_start);
            let (end_line, end_col) = doc.line_column_of_byte(span.old_end);
            replacements.push(text_editor::Replacement::new(
                text_editor::Position {
                    line: start_line,
                    index: start_col,
                }..text_editor::Position {
                    line: end_line,
                    index: end_col,
                },
                inserted,
            ));
            length_delta = length_delta
                .saturating_add(span.new_len as isize)
                .saturating_sub(span.old_len() as isize);
            previous_end = span.old_end;
        }
        if doc_len.saturating_add_signed(length_delta) != new_text.len() {
            return false;
        }
        let cursor_after = position_in_flat(new_text, caret_byte);
        let batch = text_editor::ReplaceBatch {
            replacements,
            cursor_after: text_editor::Cursor {
                position: cursor_after,
                selection: None,
            },
        };
        let action = text_editor::Action::Edit(text_editor::Edit::Replace(Arc::new(batch)));
        let Some(delta) = self.content.perform(action) else {
            // iced rejected the batch; `Content` is unchanged.
            return false;
        };
        match self.apply_edit_delta(&delta) {
            EditDeltaOutcome::Applied(_) => {
                self.dirty = true;
                self.stats_pending = true;
                true
            }
            EditDeltaOutcome::Drifted => {
                // The editor applied the batch but the mirror could not follow:
                // rebuild the mirror and mark dirty. History still has one entry.
                self.sync_snapshot();
                self.dirty = true;
                self.stats_pending = true;
                true
            }
            EditDeltaOutcome::RefusedOverlongLine { .. } => {
                // Unreachable after the preflight in
                // `reload_preserving_caret_inner`; recover with a snapshot install.
                self.sync_snapshot();
                self.dirty = true;
                self.stats_pending = true;
                true
            }
            EditDeltaOutcome::AuthorityExhausted => {
                // Version counter exhausted before any byte moved: rebuild
                // `Content` from the untouched document; `dirty` and
                // `stats_pending` stay as found.
                self.restore_content_from_document();
                false
            }
        }
    }

    pub(crate) fn overlong_line_in(&self, text: &str) -> Option<(usize, usize, usize)> {
        let limit = self.virtual_editor.as_ref()?.cache.policy().max_line_bytes;
        text.split('\n').enumerate().find_map(|(line, content)| {
            (content.len() > limit).then_some((line, content.len(), limit))
        })
    }

    // All tabs must pass UTF-8, span, length, and paragraph checks before any replacement commits.
    pub(crate) fn preflight_replace_spans(
        &self,
        new_text: String,
        spans: Vec<crate::search::ReplaceSpan>,
    ) -> Result<PreparedReplace, String> {
        let doc = self.document.current();
        let doc_len = doc.len();
        let mut delta: isize = 0;
        let mut previous_end = 0usize;
        for (index, span) in spans.iter().enumerate() {
            if span.old_start >= span.old_end {
                return Err("replace plan has an empty or reversed span".to_owned());
            }
            if span.old_end > doc_len {
                return Err("replace plan span extends past the buffer".to_owned());
            }
            if index > 0 && span.old_start < previous_end {
                return Err("replace plan spans overlap or are unordered".to_owned());
            }
            if !doc.is_char_boundary(span.old_start) || !doc.is_char_boundary(span.old_end) {
                return Err("replace plan span is not on a UTF-8 boundary".to_owned());
            }
            let Ok(new_start) = usize::try_from(span.old_start as isize + delta) else {
                return Err("replace plan offset went below zero".to_owned());
            };
            let Some(new_end) = new_start.checked_add(span.new_len) else {
                return Err("replace plan overflowed while rebasing".to_owned());
            };
            if new_text.get(new_start..new_end).is_none() {
                return Err("replace plan target is not on a UTF-8 boundary".to_owned());
            }
            delta = delta
                .checked_add(span.new_len as isize)
                .and_then(|d| d.checked_sub(span.old_len() as isize))
                .ok_or_else(|| "replace plan length delta overflowed".to_owned())?;
            previous_end = span.old_end;
        }
        let Some(expected_len) = doc_len.checked_add_signed(delta) else {
            return Err("replace plan final length underflowed".to_owned());
        };
        if expected_len != new_text.len() {
            return Err("replace plan length disagrees with the produced text".to_owned());
        }
        if let Some((line, bytes, limit)) = self.overlong_line_in(&new_text) {
            return Err(overlong_line_refused_sentence(line, bytes, limit));
        }
        Ok(PreparedReplace {
            snapshot_version: self.snapshot_version,
            tab_id: self.id.clone(),
            new_text,
            spans,
        })
    }

    // Consume a [`PreparedReplace`] and commit it as one undoable transaction.
    //
    // After an exact preflight this returns [`VirtualWriteOutcome::Applied`];
    // any other outcome is an invariant break the caller `debug_assert!`s.
    pub(crate) fn commit_prepared_replace(
        &mut self,
        prepared: PreparedReplace,
    ) -> VirtualWriteOutcome {
        let PreparedReplace {
            snapshot_version,
            tab_id: _,
            new_text,
            spans,
        } = prepared;
        debug_assert_eq!(
            snapshot_version, self.snapshot_version,
            "commit_prepared_replace ran against a tab whose snapshot moved after preflight",
        );
        let target = crate::search::map_offset(&spans, self.caret_byte());
        self.reload_preserving_caret_spans(&new_text, target, &spans)
    }
}
