//! Tab document statistics.

use super::*;

impl Tab {
    // Status-bar `chars / words / lines`, cached against the buffer version.
    pub(crate) fn text_stats(&self) -> crate::stats::TextStats {
        if let Some((version, value)) = self.text_stats_cache.get()
            && version == self.snapshot_version
        {
            return value;
        }
        let value = crate::stats::text_stats_snapshot(self.document.current());
        self.text_stats_cache
            .set(Some((self.snapshot_version, value)));
        value
    }

    // The status-bar statistics, only if they are already known.
    //
    // Never starts a scan on the frame path: large documents are counted
    // off-thread and land through [`Tab::accept_stats`].
    pub(crate) fn text_stats_ready(&self) -> Option<crate::stats::TextStats> {
        self.text_stats_cache
            .get()
            .and_then(|(version, value)| (version == self.snapshot_version).then_some(value))
    }

    pub(crate) fn stats_pending(&self) -> bool {
        self.stats_pending && self.text_stats_ready().is_none()
    }

    pub(crate) fn stats_startable(&self) -> bool {
        self.stats_pending() && !self.stats_in_flight
    }

    pub(crate) fn stats_started(&mut self) {
        self.stats_in_flight = true;
    }

    pub(crate) fn release_stats_claim(&mut self) {
        self.stats_in_flight = false;
    }

    // Install statistics computed off the frame, if they are still current.
    //
    // Generation-tagged: counts that an edit overtook are rejected, and the
    // delta path takes over from the next accepted pass.
    pub(crate) fn accept_stats(
        &mut self,
        generation: u64,
        text: crate::stats::TextStats,
        document: crate::doc_stats::Stats,
    ) -> bool {
        self.stats_in_flight = false;
        if generation != self.snapshot_version {
            // Stale counts: `stats_pending` stays set so the caller starts exactly
            // one replacement pass.
            return false;
        }
        self.text_stats_cache.set(Some((generation, text)));
        self.doc_stats_cache.set(Some((generation, document)));
        self.stats_pending = false;
        true
    }

    // Note that an editor action ran, so the selection range may have changed
    // in a way the cursor pair cannot express.
    //
    // An integer increment, called only where a `text_editor::Action` is
    // performed, never from a repaint. On counter exhaustion the epoch stays
    // put, the tab latches `authority_exhausted`, and the last landed counts
    // stay authoritative.
    pub(crate) fn note_editor_action(&mut self) {
        if self.authority_exhausted {
            return;
        }
        let Some(next) = self.cursor_epoch.checked_add(1) else {
            self.authority_exhausted = true;
            return;
        };
        self.cursor_epoch = next;
    }

    // The generation tag naming the selection this tab has *now*.
    //
    // O(1): reads no selected byte, so repaints and drags may ask freely.
    pub(crate) fn selection_stats_key(&self) -> SelectionStatsKey {
        SelectionStatsKey {
            snapshot_version: self.snapshot_version,
            cursor_epoch: self.cursor_epoch,
            cursor: self.editor_cursor(),
        }
    }

    // Whether this tab holds a selection, without copying a selected byte.
    //
    // Uses `crate::has_selection_anchor`, the same predicate as the Replace
    // scope.
    pub(crate) fn has_selection(&self) -> bool {
        self.virtual_editor.as_ref().map_or_else(
            || crate::has_selection_anchor(&self.content),
            |state| state.selection.anchor_byte.is_some(),
        )
    }

    // Statistics for the current selection, only if they are already known.
    //
    // The view must not call `Content::selection()`, which copies every
    // selected byte. `None` means not counted yet; the previous selection's
    // numbers are never returned.
    pub(crate) fn selection_stats_ready(&self) -> Option<crate::doc_stats::Stats> {
        let key = self.selection_stats_key();
        self.selection_stats_landed
            .and_then(|(landed, value)| (landed == key).then_some(value))
    }

    pub(crate) fn wants_selection_stats(&self) -> bool {
        self.has_selection()
            && !self.selection_stats_in_flight
            && self.selection_stats_ready().is_none()
    }

    // Take an immutable snapshot handle plus the exact selected byte
    // range for a background count, or `None` when there is nothing to start.
    //
    // Uses cosmic-text's expanded Normal, Word or Line bounds without copying
    // text; cloning the persistent root is O(1) and only the worker scans.
    pub(crate) fn take_selection_stats_job(
        &mut self,
    ) -> Option<(SelectionStatsKey, SelectionStatsJob)> {
        if self.selection_stats_in_flight || self.selection_stats_ready().is_some() {
            return None;
        }
        let key = self.selection_stats_key();
        let snapshot = self.document.current();
        let range = if let Some(state) = &self.virtual_editor {
            let anchor = state.selection.anchor_byte?;
            anchor.min(state.selection.active_byte)..anchor.max(state.selection.active_byte)
        } else {
            let positions = self.content.selection_range()?;
            let a = snapshot.byte_of_line_column(positions.start.line, positions.start.index);
            let b = snapshot.byte_of_line_column(positions.end.line, positions.end.index);
            a.min(b)..a.max(b)
        };
        if range.is_empty() {
            return None;
        }
        let job = SelectionStatsJob {
            document: self.snapshot_handle(),
            range,
        };
        self.selection_stats_in_flight = true;
        Some((key, job))
    }

    // Give back the in-flight claim a refused spawn never used.
    //
    // Needed when the bounded worker queue refuses, or the dialog would say
    // "counting" forever.
    pub(crate) fn release_selection_stats_claim(&mut self) {
        self.selection_stats_in_flight = false;
    }

    // Install a background count, if it still describes the selection
    // the user has.
    //
    // Returns `false` for a stale generation and drops the result, so a
    // slower older count cannot overwrite a newer selection's numbers. No
    // replacement is dispatched here; the debounce counts the next settled
    // selection.
    pub(crate) fn accept_selection_stats(
        &mut self,
        key: SelectionStatsKey,
        stats: crate::doc_stats::Stats,
    ) -> bool {
        self.selection_stats_in_flight = false;
        if key != self.selection_stats_key() {
            return false;
        }
        self.selection_stats_landed = Some((key, stats));
        true
    }
}
