//! Find and replace engine, independent of rendering.

use regex::{Regex, RegexBuilder};

#[path = "search/replace.rs"]
mod replace;
pub(crate) use replace::{
    ReplaceSpan, map_offset, replace_all_spans, replace_all_spans_with_limits, replace_one,
};
#[cfg(test)]
#[path = "search/tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Match {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ReplaceScope {
    // Replace in the current document only (the default).
    #[default]
    CurrentDocument,
    // Replace within the current selection only.
    Selection,
    AllOpenTabs,
}

impl ReplaceScope {
    pub(crate) const ALL: [ReplaceScope; 3] = [
        ReplaceScope::CurrentDocument,
        ReplaceScope::Selection,
        ReplaceScope::AllOpenTabs,
    ];

    pub(crate) fn label(self) -> String {
        match self {
            ReplaceScope::CurrentDocument => {
                enzui_core::localization::text("text-current-document-4a7b73")
            }
            ReplaceScope::Selection => enzui_core::localization::text("text-selection-ca4067"),
            ReplaceScope::AllOpenTabs => {
                enzui_core::localization::text("text-all-open-tabs-ecdbf7")
            }
        }
    }
}

// The tab identity and snapshot version a match set was computed against.
//
// Matches are byte offsets valid only for that buffer, so every consumer
// checks the binding against the live active tab before painting or writing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MatchBinding {
    pub tab_id: String,
    // The `Tab::snapshot_version` the scan read from.
    pub snapshot_version: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SearchState {
    pub open: bool,
    pub replace_open: bool,
    pub query: String,
    pub replacement: String,
    pub options: SearchOptions,
    pub matches: Vec<Match>,
    pub current: usize,
    pub error: String,
    pub replace_scope: ReplaceScope,
    // The generation of the newest requested match set; a background result
    // with any other generation is dropped.
    //
    // Never wraps: an exhausted counter refuses further scans and replaces
    // (see [`Self::next_generation`]).
    pub generation: u64,
    // The generation counter is at its ceiling and no further scan can be
    // dispatched or accepted. Cleared only by an app restart.
    pub exhausted: bool,
    // The tab and snapshot version the currently retained `matches` were
    // computed against; `None` when no valid match set is retained.
    pub binding: Option<MatchBinding>,
    // A background scan for [`generation`](Self::generation) is in flight.
    pub searching: bool,
    pub total: usize,
}

// The bounded, user-visible error published when the search generation
// counter reaches its ceiling.
//
// Unreachable in real use; it exists so exhaustion refuses rather than
// reusing a generation.
pub(crate) const GENERATION_EXHAUSTED_MESSAGE: &str =
    "Search authority is exhausted; restart the app to search again.";

impl SearchState {
    // Claim the next generation for a scan that is about to start, and
    // abandon whatever is in flight.
    //
    // Returns `None` once the counter reaches its ceiling: the state is marked
    // exhausted, matches and binding are cleared, and
    // [`GENERATION_EXHAUSTED_MESSAGE`] is published. The counter never wraps.
    pub(crate) fn next_generation(&mut self) -> Option<u64> {
        if self.exhausted {
            return None;
        }
        match self.generation.checked_add(1) {
            Some(next) => {
                self.generation = next;
                Some(next)
            }
            None => {
                self.exhausted = true;
                self.matches = Vec::new();
                self.binding = None;
                self.current = 0;
                self.total = 0;
                self.searching = false;
                crate::i18n::domain_text(GENERATION_EXHAUSTED_MESSAGE).clone_into(&mut self.error);
                None
            }
        }
    }

    // Clear the retained match set, the binding, the counter and any
    // published error.
    pub(crate) fn release_results(&mut self) {
        self.matches = Vec::new();
        self.current = 0;
        self.total = 0;
        self.error = String::new();
        self.binding = None;
    }

    // Retire the matches and their binding, keeping query, options, open
    // state and any in-flight error.
    //
    // Call on every change to the bound document, so stale highlights vanish
    // on the same frame and no replace uses retired offsets.
    pub(crate) fn retire_bindings(&mut self) {
        self.matches = Vec::new();
        self.current = 0;
        self.total = 0;
        self.binding = None;
    }

    // Whether the retained matches were computed against
    // `(tab_id, snapshot_version)`.
    //
    // A mismatch means the matches are retired and the caller must refuse.
    pub(crate) fn binding_matches(&self, tab_id: &str, snapshot_version: u64) -> bool {
        self.binding
            .as_ref()
            .is_some_and(|b| b.tab_id == tab_id && b.snapshot_version == snapshot_version)
    }

    // The current match, or `None` unless the retained set is bound to the
    // given `(tab_id, snapshot_version)`.
    pub(crate) fn current_match_bound(&self, tab_id: &str, snapshot_version: u64) -> Option<Match> {
        if !self.binding_matches(tab_id, snapshot_version) {
            return None;
        }
        self.current_match()
    }

    pub(crate) fn rebind(&mut self, binding: MatchBinding) {
        self.binding = Some(binding);
    }

    // Accept a background scan's result, or reject it as stale.
    //
    // `binding` is the `(tab_id, snapshot_version)` the scan was launched
    // against; it is stored on acceptance.
    pub(crate) fn accept(
        &mut self,
        generation: u64,
        result: Result<Scan, String>,
        from: Option<usize>,
        binding: MatchBinding,
    ) -> bool {
        if generation != self.generation || self.exhausted {
            return false;
        }
        self.searching = false;
        match result {
            Ok(scan) => {
                self.error.clear();
                self.current = match from {
                    Some(caret) => scan
                        .matches
                        .iter()
                        .position(|m| m.start >= caret)
                        .unwrap_or(0),
                    None => self.current.min(scan.matches.len().saturating_sub(1)),
                };
                self.total = scan.total;
                self.matches = scan.matches;
                self.binding = Some(binding);
            }
            Err(message) => {
                self.error = message;
                self.matches.clear();
                self.total = 0;
                self.current = 0;
                self.binding = None;
            }
        }
        true
    }
}

impl SearchState {
    // The matches that start inside `lines`, as native caret positions, with
    // each one's index into [`SearchState::matches`].
    //
    // Found by binary search over the sorted matches; positions use byte
    // columns. A match that starts above the window but reaches into it is
    // included.
    pub(crate) fn matches_in_lines<'a>(
        &'a self,
        text: &'a enzui_core::chunked_text::TextSnapshot,
        lines: std::ops::Range<usize>,
    ) -> impl Iterator<
        Item = (
            usize,
            iced::widget::text_editor::Position,
            iced::widget::text_editor::Position,
        ),
    > + 'a {
        let from = text.line_start(lines.start).unwrap_or(text.len());
        let to = text.line_start(lines.end).unwrap_or(text.len());
        // The first match that could still reach `from` is found by scanning
        // back from the first match starting at or after it; matches do not
        // overlap, so at most one earlier match can cover the boundary.
        let first = self.matches.partition_point(|m| m.start < from);
        let first = first.saturating_sub(1);

        self.matches
            .iter()
            .enumerate()
            .skip(first)
            .take_while(move |(_, m)| m.start <= to)
            .filter(move |(_, m)| m.end >= from)
            .map(move |(i, m)| {
                (
                    i,
                    {
                        let (line, column) = text.line_column_of_byte(m.start);
                        iced::widget::text_editor::Position {
                            line,
                            index: column,
                        }
                    },
                    {
                        let (line, column) = text.line_column_of_byte(m.end);
                        iced::widget::text_editor::Position {
                            line,
                            index: column,
                        }
                    },
                )
            })
    }

    // Recompute from a persistent snapshot without flattening it.
    //
    // Intended for bounded foreground literal/whole-word searches; regex
    // searches belong on the blocking worker, which needs a contiguous haystack.
    pub(crate) fn recompute_snapshot(&mut self, document: &enzui_core::chunked_text::TextSnapshot) {
        self.error.clear();
        let result = find_all_snapshot_cancellable(
            document,
            &self.query,
            self.options,
            &crate::worker::Cancel::default(),
        );
        match result {
            Cancellable::Done(Ok(scan)) => {
                self.total = scan.total;
                self.matches = scan.matches;
                self.current = self.current.min(self.matches.len().saturating_sub(1));
            }
            Cancellable::Done(Err(message)) => {
                self.error = message;
                self.matches.clear();
                self.total = 0;
                self.current = 0;
            }
            Cancellable::Cancelled => unreachable!("fresh local token cannot be cancelled"),
        }
    }

    // Recompute from a persistent snapshot and select from `caret`.
    pub(crate) fn recompute_snapshot_starting_at(
        &mut self,
        document: &enzui_core::chunked_text::TextSnapshot,
        caret: usize,
    ) {
        self.recompute_snapshot(document);
        self.select_from(caret);
    }

    fn select_from(&mut self, caret: usize) {
        if self.matches.is_empty() {
            return;
        }
        self.current = self
            .matches
            .iter()
            .position(|m| m.start >= caret)
            .unwrap_or(0);
    }

    pub(crate) fn next(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current = (self.current + 1) % self.matches.len();
    }

    pub(crate) fn prev(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        if self.current == 0 {
            self.current = self.matches.len() - 1;
        } else {
            self.current -= 1;
        }
    }

    pub(crate) fn counter_label(&self) -> String {
        if !self.error.is_empty() {
            return enzui_core::localized!("text-error-p0-8904a2", "p0" => self.error);
        }
        if self.query.is_empty() {
            return String::new();
        }
        if self.matches.is_empty() {
            // An unfinished scan must not report "No matches".
            if self.searching {
                return enzui_core::localization::text("text-searching-c31723");
            }
            return enzui_core::localization::text("text-no-matches-2df01a");
        }
        if self.total > self.matches.len() {
            return enzui_core::localized!("text-p0-of-p1-shown-p2-matches-5a1e2b", "p0" => self.current + 1, "p1" => self.matches.len(), "p2" => self.total);
        }
        enzui_core::localized!("text-p0-of-p1-7667c1", "p0" => self.current + 1, "p1" => self.matches.len())
    }

    pub(crate) fn current_match(&self) -> Option<Match> {
        self.matches.get(self.current).copied()
    }
}

fn compile_regex(query: &str, opts: SearchOptions) -> Result<Regex, String> {
    // `RegexBuilder` allocates an NFA proportional to the pattern length.
    if query.len() > MAX_QUERY_BYTES {
        return Err(enzui_core::localization::text("text-search-e79364d9dd"));
    }
    let base = if opts.regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    let pattern = if opts.whole_word {
        format!(r"\b(?:{base})\b")
    } else {
        base
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!opts.case_sensitive)
        .multi_line(true)
        .build()
        .map_err(|error| match error {
            regex::Error::CompiledTooBig(_) => {
                enzui_core::localization::text("text-search-c058b554cf")
            }
            _ => enzui_core::localization::text("text-search-6111038a9e"),
        })
}

// Matches that are kept, plus how many there are.
//
// The scan keeps a bounded prefix and counts the rest, so the two numbers can
// differ; the find bar reports both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Scan {
    // The matches that were kept: at most [`MAX_RETAINED_MATCHES`].
    pub matches: Vec<Match>,
    pub total: usize,
}

// The most matches a search keeps for navigation and highlighting.
//
// About 3 MB of offsets; past it the count stays exact and the bar says the
// list is a prefix.
pub(crate) const MAX_RETAINED_MATCHES: usize = 200_000;

// The largest query the engine will compile.
//
// A longer pattern is rejected at [`compile_regex`] before any scan or
// replace allocation.
pub(crate) const MAX_QUERY_BYTES: usize = 1 << 16;

// The largest replacement string the engine will apply.
//
// [`replace_all_spans`] refuses a replacement past this bound before it
// writes the first output byte; [`replace_one`] refuses at the same point.
pub(crate) const MAX_REPLACEMENT_BYTES: usize = 1 << 16;

// The most bytes [`replace_all_spans`] will emit into its new buffer.
//
// Checked before the output grows past the bound, so a refusal leaves the
// document byte-identical. Covers the 40 MiB editable ceiling plus expansion.
pub(crate) const MAX_REPLACE_OUTPUT_BYTES: usize = 128 * 1024 * 1024;

// The length of a replacement's whole-document result, or `None` when it
// would pass [`MAX_REPLACE_OUTPUT_BYTES`] or overflow.
#[must_use]
pub(crate) fn checked_output_len(outside_len: usize, replaced_len: usize) -> Option<usize> {
    outside_len
        .checked_add(replaced_len)
        .filter(|total| *total <= MAX_REPLACE_OUTPUT_BYTES)
}

// The most bytes one `AllOpenTabs` batch may retain across all tabs'
// candidates, equal to the per-document cap.
//
// The batch driver passes the remaining budget as `limits.output_bytes` to
// [`replace_all_spans_with_limits`].
pub(crate) const MAX_REPLACE_BATCH_OUTPUT_BYTES: usize = MAX_REPLACE_OUTPUT_BYTES;

// The most [`ReplaceSpan`]s one `AllOpenTabs` batch may retain across all
// tabs' candidates, equal to the per-document cap.
//
// The batch driver passes the remaining budget as `limits.matches` to
// [`replace_all_spans_with_limits`].
pub(crate) const MAX_REPLACE_BATCH_MATCHES: usize = MAX_RETAINED_MATCHES;

// Ceilings that bound a single Replace All operation.
//
// [`replace_all_spans_with_limits`] clamps both fields to
// [`MAX_REPLACE_OUTPUT_BYTES`] and [`MAX_RETAINED_MATCHES`]; smaller values
// apply exactly.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReplaceLimits {
    // Output-byte ceiling for this call.
    pub output_bytes: usize,
    // Retained-span ceiling for this call.
    pub matches: usize,
}

impl ReplaceLimits {
    pub(crate) const PER_DOCUMENT: Self = Self {
        output_bytes: MAX_REPLACE_OUTPUT_BYTES,
        matches: MAX_RETAINED_MATCHES,
    };
}

#[derive(Debug)]
pub(crate) enum Cancellable<T> {
    Done(T),
    Cancelled,
}

pub(crate) const CANCEL_CHUNK_BYTES: usize = 1 << 20;

// `find_all` that stops when `cancel` says so.
//
// The scan checks `cancel` between bounded chunks. Chunking is exact: matches
// that straddle a chunk edge are found once. Literal and whole-word searches
// never flatten the document; regex searches do, so run them on the blocking
// worker.
pub(crate) fn find_all_snapshot_cancellable(
    document: &enzui_core::chunked_text::TextSnapshot,
    query: &str,
    opts: SearchOptions,
    cancel: &crate::worker::Cancel,
) -> Cancellable<Result<Scan, String>> {
    find_all_snapshot_cancellable_observed(document, query, opts, cancel, |_| {})
}

fn find_all_snapshot_cancellable_observed(
    document: &enzui_core::chunked_text::TextSnapshot,
    query: &str,
    opts: SearchOptions,
    cancel: &crate::worker::Cancel,
    mut observe_window: impl FnMut(usize),
) -> Cancellable<Result<Scan, String>> {
    if query.is_empty() {
        return Cancellable::Done(Ok(Scan::default()));
    }
    if opts.regex {
        if cancel.is_cancelled() {
            return Cancellable::Cancelled;
        }
        // The regex crate requires one contiguous haystack.
        let text = document.to_string();
        return find_all_cancellable(&text, query, opts, cancel);
    }
    let re = match compile_regex(query, opts) {
        Ok(re) => re,
        Err(error) => return Cancellable::Done(Err(error)),
    };
    let mut matches = Vec::new();
    let mut total = 0_usize;
    let reach = query.len().saturating_mul(4).saturating_add(4);
    let mut chunk_start = 0;
    while chunk_start < document.len() {
        if cancel.is_cancelled() {
            return Cancellable::Cancelled;
        }
        let chunk_end = document.ceil_char_boundary(
            chunk_start
                .saturating_add(CANCEL_CHUNK_BYTES)
                .min(document.len()),
        );
        let scan_start = document.floor_char_boundary(chunk_start.saturating_sub(4));
        let scan_end =
            document.ceil_char_boundary(chunk_end.saturating_add(reach).min(document.len()));
        let window = document
            .slice_to_string(scan_start..scan_end)
            .expect("boundaries were normalized");
        observe_window(window.len());
        for found in re.find_iter(&window) {
            let start = scan_start + found.start();
            if start < chunk_start {
                continue;
            }
            if start >= chunk_end {
                break;
            }
            total += 1;
            if matches.len() < MAX_RETAINED_MATCHES && found.start() != found.end() {
                matches.push(Match {
                    start,
                    end: scan_start + found.end(),
                });
            }
        }
        chunk_start = chunk_end;
    }
    Cancellable::Done(Ok(Scan { matches, total }))
}

pub(crate) fn find_all_cancellable(
    haystack: &str,
    query: &str,
    opts: SearchOptions,
    cancel: &crate::worker::Cancel,
) -> Cancellable<Result<Scan, String>> {
    if query.is_empty() {
        return Cancellable::Done(Ok(Scan::default()));
    }
    let re = match compile_regex(query, opts) {
        Ok(re) => re,
        Err(error) => return Cancellable::Done(Err(error)),
    };

    let mut out: Vec<Match> = Vec::new();
    let mut total = 0_usize;
    let mut highest_start: Option<usize> = None;
    let mut push = |start: usize, end: usize| {
        if start == end {
            return;
        }
        if highest_start.is_some_and(|last| last >= start) {
            return;
        }
        highest_start = Some(start);
        total += 1;
        if out.len() < MAX_RETAINED_MATCHES {
            out.push(Match { start, end });
        }
    };

    // Literal whole-word search is chunk-safe: every window includes one real
    // scalar of context on both sides, so a slice edge cannot fake `\b`.
    if opts.regex {
        let mut checked_up_to = 0;
        for m in re.find_iter(haystack) {
            if m.start() >= checked_up_to {
                if cancel.is_cancelled() {
                    return Cancellable::Cancelled;
                }
                checked_up_to = m.start() + CANCEL_CHUNK_BYTES;
            }
            push(m.start(), m.end());
        }
        return Cancellable::Done(Ok(Scan {
            matches: out,
            total,
        }));
    }

    // A case-folded match can be longer than the query, so reach past the chunk
    // by an upper bound on match length and accept only matches starting inside
    // this chunk; each match is found once, with real context on both sides.
    let reach = query.len().saturating_mul(4).saturating_add(4);
    let mut chunk_start = 0;
    while chunk_start < haystack.len() {
        if cancel.is_cancelled() {
            return Cancellable::Cancelled;
        }
        let mut chunk_end = chunk_start
            .saturating_add(CANCEL_CHUNK_BYTES)
            .min(haystack.len());
        while chunk_end < haystack.len() && !haystack.is_char_boundary(chunk_end) {
            chunk_end += 1;
        }
        let mut scan_end = chunk_end.saturating_add(reach).min(haystack.len());
        while scan_end < haystack.len() && !haystack.is_char_boundary(scan_end) {
            scan_end += 1;
        }
        let mut scan_start = chunk_start;
        while scan_start > 0 {
            scan_start -= 1;
            if haystack.is_char_boundary(scan_start) {
                break;
            }
        }

        let Some(scan) = haystack.get(scan_start..scan_end) else {
            return Cancellable::Done(Err(enzui_core::localization::text(
                "text-search-38272f397b",
            )));
        };
        for m in re.find_iter(scan) {
            let start = scan_start + m.start();
            if start < chunk_start {
                // Owned by the previous chunk, which already reported it.
                continue;
            }
            if start >= chunk_end {
                // Owned by the next chunk, which will.
                break;
            }
            push(start, scan_start + m.end());
        }

        chunk_start = chunk_end;
    }

    Cancellable::Done(Ok(Scan {
        matches: out,
        total,
    }))
}
