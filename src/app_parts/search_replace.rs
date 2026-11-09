//! TextApp search and replace.

use super::*;

impl TextApp {
    pub(crate) fn start_queued_search(&mut self) -> Admission {
        if self.search_scan.is_some() {
            return Admission::NothingToDo;
        }
        if let Some(attempt) = self.search_retry.take() {
            if self.search_retry_blocked {
                self.search_retry = Some(attempt);
                return Admission::NothingToDo;
            }
            self.search_queued = None;
            self.dispatch_search_attempt(attempt);
        } else {
            let Some(from) = self.search_queued.take() else {
                return Admission::NothingToDo;
            };
            self.spawn_search(from);
        }
        if self.search_scan.is_some() {
            Admission::Started
        } else {
            // `spawn_search` re-queued itself: the queue refused it.
            Admission::Refused
        }
    }

    pub(crate) fn abort_search_scan(&mut self) {
        // The marker stays until the worker reports back; clearing it would let a
        // reopen or new query start a second concurrent scan.
        if let Some(cancel) = &self.search_scan {
            cancel.cancel();
        }
        // Retiring the generation stops a late completion painting on the
        // successor tab, including on counter exhaustion.
        let _ = self.model.search.next_generation();
        self.model.search.retire_bindings();
        self.search_queued = None;
        self.search_retry = None;
        self.search_attempt = None;
        self.search_retry_blocked = false;
        self.model.search.searching = false;
    }

    pub(crate) fn recompute_search(&mut self) {
        // A paged tab's `text_snapshot` is empty, so a recompute would claim
        // "No matches"; the view shows `UNSEARCHABLE_COUNTER` instead.
        if !self
            .model
            .active_tab()
            .supports(large_doc::Capability::Find)
        {
            return;
        }
        // Exhaustion is durable; its error is already published.
        if self.model.search.exhausted {
            return;
        }
        // Retire the stale match set so no old highlight paints while the fresh
        // scan runs.
        self.model.search.retire_bindings();
        if self.model.search.options.regex
            || self.model.active_tab().document().len() > Self::FOREGROUND_SEARCH_BYTES
        {
            self.spawn_search(None);
            return;
        }
        self.model.recompute_search();
    }

    // No-op when Find has never been used and no scan is in flight.
    pub(crate) fn retire_search_for_active_change(&mut self) {
        let nothing_to_retire = self.search_scan.is_none()
            && self.model.search.binding.is_none()
            && !self.model.search.open;
        if nothing_to_retire {
            return;
        }
        self.abort_search_scan();
        if self.model.search.exhausted
            || !self.model.search.open
            || self.model.search.query.is_empty()
        {
            return;
        }
        self.recompute_search();
    }

    pub(crate) fn caret_byte_offset(&self) -> usize {
        self.model.active_tab().caret_byte()
    }

    pub(crate) fn open_search_at_caret(&mut self) {
        if self.model.search.exhausted {
            return;
        }
        self.model.search.retire_bindings();
        let caret = self.caret_byte_offset();
        if self.model.search.options.regex
            || self.model.active_tab().document().len() > Self::FOREGROUND_SEARCH_BYTES
        {
            self.spawn_search(Some(caret));
            return;
        }
        self.model.recompute_search_starting_at(caret);
    }

    // Refuses through the failure banner unless the match set is bound to the
    // active tab's current `(id, snapshot_version)` and not exhausted;
    // `search::replace_one` also revalidates the range.
    pub(crate) fn replace_current(&mut self) {
        let query = self.model.search.query.clone();
        if query.is_empty() {
            return;
        }
        if self.model.search.exhausted {
            return;
        }
        let (active_id, active_version) = {
            let active = self.model.active_tab();
            (active.id.clone(), active.snapshot_version())
        };
        let Some(m) = self
            .model
            .search
            .current_match_bound(&active_id, active_version)
        else {
            return;
        };
        let replacement = self.model.search.replacement.clone();
        let opts = self.model.search.options;
        let buffer = self.model.active_tab().snapshot();
        match search::replace_one(&buffer, m, &query, &replacement, opts) {
            Ok((new_text, span)) => {
                self.load_active_buffer_mapped(&new_text, &[span]);
                self.recompute_search();
            }
            Err(message) => {
                self.publish_replace_failure(&message);
            }
        }
    }

    // - `CurrentDocument`: replace in the active buffer.
    // - `Selection`: replace only within the selected text range. The text
    //   outside the selection is preserved byte-for-byte.
    // - `AllOpenTabs`: replace in every open tab's buffer. Each tab gets its
    //   own `load_buffer` call, so undo is per-tab (iced `text_editor` has no
    //   cross-buffer undo group).
    pub(crate) fn replace_all(&mut self) {
        let query = self.model.search.query.clone();
        if query.is_empty() {
            return;
        }
        if self.model.search.exhausted {
            return;
        }
        let scope = self.model.search.replace_scope;
        // Single-buffer scopes require the active-tab binding; AllOpenTabs
        // preflights each tab itself.
        if !matches!(scope, search::ReplaceScope::AllOpenTabs) {
            let (active_id, active_version) = {
                let active = self.model.active_tab();
                (active.id.clone(), active.snapshot_version())
            };
            if !self
                .model
                .search
                .binding_matches(&active_id, active_version)
            {
                return;
            }
        }
        let replacement = self.model.search.replacement.clone();
        let opts = self.model.search.options;

        match scope {
            search::ReplaceScope::CurrentDocument => {
                self.replace_all_current_document(&query, &replacement, opts);
            }
            search::ReplaceScope::Selection => {
                self.replace_all_in_selection(&query, &replacement, opts);
            }
            search::ReplaceScope::AllOpenTabs => {
                self.replace_all_in_all_tabs(&query, &replacement, opts);
            }
        }
    }

    pub(crate) fn publish_replace_failure(&mut self, message: &str) {
        self.picker.failure = Some(
            enzui_core::localized!("text-replace-refused-message-22ceb4", "message" => crate::i18n::domain_text(message)),
        );
    }

    pub(crate) fn replace_all_current_document(
        &mut self,
        query: &str,
        replacement: &str,
        opts: search::SearchOptions,
    ) {
        let buffer = self.model.active_tab().snapshot();
        match search::replace_all_spans(&buffer, query, replacement, opts) {
            Ok((new_text, spans)) if !spans.is_empty() => {
                self.load_active_buffer_mapped(&new_text, &spans);
                self.recompute_search();
            }
            Ok(_) => {}
            Err(message) => self.publish_replace_failure(&message),
        }
    }

    // The spliced whole-document output is also bounded by
    // `MAX_REPLACE_OUTPUT_BYTES` with checked arithmetic; overflow refuses
    // through the failure banner.
    pub(crate) fn replace_all_in_selection(
        &mut self,
        query: &str,
        replacement: &str,
        opts: search::SearchOptions,
    ) {
        let cursor = self.model.active_tab().editor_cursor();
        let Some(sel_pos) = cursor.selection else {
            self.replace_all_current_document(query, replacement, opts);
            return;
        };

        let buffer = self.model.active_tab().snapshot();

        let pos_a = cursor.position;
        let pos_b = sel_pos;
        // `Position::column` is a byte index within its line.
        let document = self.model.active_tab().document();
        let off_a = document.byte_of_line_column(pos_a.line, pos_a.index);
        let off_b = document.byte_of_line_column(pos_b.line, pos_b.index);
        let (start, end) = if off_a <= off_b {
            (off_a, off_b)
        } else {
            (off_b, off_a)
        };

        // A stale caret after a reload can name an invalid or mid-codepoint byte;
        // refuse rather than panic on the slice.
        if start > end {
            self.publish_replace_failure("selection range is inverted");
            return;
        }
        if end > buffer.len() {
            self.publish_replace_failure("selection extends past the buffer");
            return;
        }
        if !buffer.is_char_boundary(start) || !buffer.is_char_boundary(end) {
            self.publish_replace_failure("selection endpoint is not on a UTF-8 boundary");
            return;
        }

        let (Some(prefix), Some(selected), Some(suffix)) = (
            buffer.get(..start),
            buffer.get(start..end),
            buffer.get(end..),
        ) else {
            self.publish_replace_failure("selection endpoint is not on a UTF-8 boundary");
            return;
        };
        let (replaced, spans) = match search::replace_all_spans(selected, query, replacement, opts)
        {
            Ok((_, spans)) if spans.is_empty() => return,
            Ok(pair) => pair,
            Err(message) => {
                self.publish_replace_failure(&message);
                return;
            }
        };

        // Bound the spliced total with checked arithmetic before allocating.
        let Some(selected_len) = end.checked_sub(start) else {
            self.publish_replace_failure("selection range is inverted");
            return;
        };
        let Some(outside_len) = buffer.len().checked_sub(selected_len) else {
            self.publish_replace_failure("selection extends past the buffer");
            return;
        };
        let Some(final_len) = search::checked_output_len(outside_len, replaced.len()) else {
            self.publish_replace_failure("replacement would exceed the output size limit");
            return;
        };

        // Spans are in selection coordinates; rebase without wrapping.
        let mut rebased: Vec<search::ReplaceSpan> = Vec::with_capacity(spans.len());
        for s in &spans {
            let (Some(old_start), Some(old_end)) =
                (s.old_start.checked_add(start), s.old_end.checked_add(start))
            else {
                self.publish_replace_failure("span offsets overflowed while rebasing");
                return;
            };
            rebased.push(search::ReplaceSpan {
                old_start,
                old_end,
                new_len: s.new_len,
            });
        }

        let mut new_text = String::with_capacity(final_len);
        new_text.push_str(prefix);
        new_text.push_str(&replaced);
        new_text.push_str(suffix);

        self.load_active_buffer_mapped(&new_text, &rebased);
        self.recompute_search();
    }

    // Preflight every tab against shared budgets before committing any replacement.
    pub(crate) fn replace_all_in_all_tabs(
        &mut self,
        query: &str,
        replacement: &str,
        opts: search::SearchOptions,
    ) {
        let skipped: Vec<String> = self
            .model
            .tabs
            .iter()
            .filter(|tab| !tab.supports(large_doc::Capability::Replace))
            .map(|tab| crate::safe_text::bounded_inline(&tab.title()))
            .collect();

        struct Candidate {
            index: usize,
            prepared: crate::model::PreparedReplace,
        }

        let mut candidates: Vec<Candidate> = Vec::new();
        let mut refusal: Option<String> = None;
        let mut retained_bytes: usize = 0;
        let mut retained_spans: usize = 0;
        for i in 0..self.model.tabs.len() {
            let tab = &self.model.tabs[i];
            if !tab.supports(large_doc::Capability::Replace) {
                continue;
            }
            // Underflow cannot happen given the checked admissions below; it
            // refuses defensively.
            let Some(remaining_bytes) =
                search::MAX_REPLACE_BATCH_OUTPUT_BYTES.checked_sub(retained_bytes)
            else {
                let title = crate::safe_text::bounded_inline(&tab.title());
                refusal = Some(
                    enzui_core::localized!("text-replace-all-batch-has-already-retained-the-12500a", "p0" => search::MAX_REPLACE_BATCH_OUTPUT_BYTES, "title" => title),
                );
                break;
            };
            let Some(remaining_matches) =
                search::MAX_REPLACE_BATCH_MATCHES.checked_sub(retained_spans)
            else {
                let title = crate::safe_text::bounded_inline(&tab.title());
                refusal = Some(
                    enzui_core::localized!("text-replace-all-batch-has-already-retained-the-93f072", "p0" => search::MAX_REPLACE_BATCH_MATCHES, "title" => title),
                );
                break;
            };
            let buffer = tab.snapshot();
            let (new_text, spans) = match search::replace_all_spans_with_limits(
                &buffer,
                query,
                replacement,
                opts,
                search::ReplaceLimits {
                    output_bytes: remaining_bytes,
                    matches: remaining_matches,
                },
            ) {
                Ok((_, spans)) if spans.is_empty() => continue,
                Ok(pair) => pair,
                Err(message) => {
                    let title = crate::safe_text::bounded_inline(&tab.title());
                    refusal = Some(
                        enzui_core::localized!("text-message-in-title-no-tab-was-changed-625f9f", "message" => crate::i18n::domain_text(&message), "title" => title),
                    );
                    break;
                }
            };
            // The engine already enforced these ceilings; the checked sums refuse
            // any drift before the token is retained.
            let Some(next_bytes) = retained_bytes.checked_add(new_text.len()) else {
                let title = crate::safe_text::bounded_inline(&tab.title());
                refusal = Some(
                    enzui_core::localized!("text-replace-all-batch-output-overflowed-at-title-7f7541", "title" => title),
                );
                break;
            };
            if next_bytes > search::MAX_REPLACE_BATCH_OUTPUT_BYTES {
                let title = crate::safe_text::bounded_inline(&tab.title());
                refusal = Some(
                    enzui_core::localized!("text-replace-all-batch-would-exceed-the-aggregate-367338", "p0" => search::MAX_REPLACE_BATCH_OUTPUT_BYTES, "title" => title),
                );
                break;
            }
            let Some(next_spans) = retained_spans.checked_add(spans.len()) else {
                let title = crate::safe_text::bounded_inline(&tab.title());
                refusal = Some(
                    enzui_core::localized!("text-replace-all-batch-span-count-overflowed-at-9d9d1d", "title" => title),
                );
                break;
            };
            if next_spans > search::MAX_REPLACE_BATCH_MATCHES {
                let title = crate::safe_text::bounded_inline(&tab.title());
                refusal = Some(
                    enzui_core::localized!("text-replace-all-batch-would-exceed-the-aggregate-78e7ab", "p0" => search::MAX_REPLACE_BATCH_MATCHES, "title" => title),
                );
                break;
            }
            match tab.preflight_replace_spans(new_text, spans) {
                Ok(prepared) => {
                    retained_bytes = next_bytes;
                    retained_spans = next_spans;
                    candidates.push(Candidate { index: i, prepared });
                }
                Err(message) => {
                    let title = crate::safe_text::bounded_inline(&tab.title());
                    refusal = Some(
                        enzui_core::localized!("text-message-in-title-no-tab-was-changed-625f9f", "message" => crate::i18n::domain_text(&message), "title" => title),
                    );
                    break;
                }
            }
        }

        if let Some(message) = refusal {
            self.picker.failure = Some(
                enzui_core::localized!("text-replace-in-all-open-tabs-refused-message-dc0dc9", "message" => crate::i18n::domain_text(&message)),
            );
            return;
        }

        if candidates.is_empty() {
            if !skipped.is_empty() {
                self.picker.failure = Some(
                    enzui_core::localized!("text-replace-in-all-open-tabs-skipped-p0-0d5b52", "p0" => skipped.join(", ")),
                );
            }
            return;
        }

        // Cannot fail on the single-threaded reducer; refuses the batch
        // defensively.
        for candidate in &candidates {
            let live = self.model.tabs.get(candidate.index).filter(|tab| {
                tab.id == candidate.prepared.tab_id
                    && tab.snapshot_version() == candidate.prepared.snapshot_version
            });
            if live.is_none() {
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-replace-in-all-open-tabs-refused-tab-35adc3",
                ));
                return;
            }
        }

        // Preflight was exact, so commit cannot refuse; `debug_assert` traps a
        // broken invariant in tests.
        for candidate in candidates {
            let Candidate { index, prepared } = candidate;
            let outcome = self.model.tabs[index].commit_prepared_replace(prepared);
            debug_assert!(
                outcome.refusal().is_none(),
                "preflight admitted a Replace All candidate the commit then refused: {:?}",
                outcome.refusal(),
            );
        }

        if !skipped.is_empty() {
            self.picker.failure = Some(
                enzui_core::localized!("text-replace-in-all-open-tabs-skipped-p0-0d5b52", "p0" => skipped.join(", ")),
            );
        }
        self.recompute_search();
        self.sync_cursor_readout();
    }
}
