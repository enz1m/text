//! Application model: tabs, editor state and settings, without rendering.

use std::path::{Path, PathBuf};

use iced::widget::text_editor;

use enzui_core::chunked_text::{TextSnapshot, TextStore};
use enzui_core::document_tabs::{DocumentItem, DocumentKey, DocumentTabs, Orientation};
use enzui_core::editable_text_viewport::{
    EditableTextFrame, EditableTextViewportCache, EditableTextViewportPolicy, MaterializeError,
};
use enzui_core::file_watch::{FileWatchBackend, FileWatcher};

#[path = "model/tab_core.rs"]
mod tab_core;
#[path = "model/tab_edit.rs"]
mod tab_edit;
#[path = "model/tab_install.rs"]
mod tab_install;
#[path = "model/tab_reload.rs"]
mod tab_reload;
#[path = "model/tab_replace.rs"]
mod tab_replace;
#[path = "model/tab_snapshot.rs"]
mod tab_snapshot;
#[path = "model/tab_stats.rs"]
mod tab_stats;
#[path = "model/tab_virtual.rs"]
mod tab_virtual;
#[path = "model/tabs.rs"]
mod tabs;
#[path = "model/virtual_text.rs"]
mod virtual_text;
use virtual_text::{VirtualEditorState, VirtualSelection};

use crate::line_ending::LineEnding;
use crate::search::SearchState;
use crate::settings::Settings;

pub(crate) const DEFAULT_WINDOW_WIDTH: f32 = 1400.0;

// Native position indexes are bytes; character columns are only for the status readout.

// Drive a `Content`'s caret to a native [`Position`](text_editor::Position),
// in constant time.
//
// The line clamps to the live buffer; a clamped line resets the column to 0.
// Callers must pass a column on a char boundary.
pub(crate) fn move_caret_to_position(
    content: &mut text_editor::Content,
    position: text_editor::Position,
) {
    use iced::widget::text_editor::{Cursor, Position};

    let live_last_line = content.line_count().saturating_sub(1);
    let line = position.line.min(live_last_line);
    // The column is not clamped to the line: `Content::line` copies the whole
    // line, a multi-MiB allocation on a single-long-line document.
    let column = if line == position.line {
        position.index
    } else {
        0
    };

    content.move_to(Cursor {
        position: Position {
            line,
            index: column,
        },
        selection: None,
    });
}

pub(crate) fn position_in_flat(text: &str, byte: usize) -> text_editor::Position {
    let byte = byte.min(text.len());
    let prefix = text.get(..byte).unwrap_or("");
    let line = prefix.bytes().filter(|b| *b == b'\n').count();
    let line_start = prefix.rfind('\n').map_or(0, |i| i + 1);
    text_editor::Position {
        line,
        index: byte - line_start,
    }
}

// Incremental statistics over an edited region

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EditCounts {
    chars: isize,
    visible: isize,
    words: isize,
}

// A word start is a non-whitespace character whose predecessor is whitespace
// or the start of the document, so summing word starts over a document counts
// its words exactly the way `split_whitespace` does.
fn word_starts(before: Option<char>, parts: [&str; 2]) -> usize {
    let mut previous_was_whitespace = before.is_none_or(char::is_whitespace);
    let mut starts = 0;
    for ch in parts[0].chars().chain(parts[1].chars()) {
        let whitespace = ch.is_whitespace();
        if !whitespace && previous_was_whitespace {
            starts += 1;
        }
        previous_was_whitespace = whitespace;
    }
    starts
}

fn char_counts(text: &str) -> (isize, isize) {
    let mut chars = 0;
    let mut visible = 0;
    for ch in text.chars() {
        chars += 1;
        if !ch.is_whitespace() {
            visible += 1;
        }
    }
    (chars, visible)
}

// A word start depends only on itself and its predecessor, so an edit can
// change word starts only inside the replaced text and at the character after
// it. Cost is independent of the enclosing word's length.
fn edit_counts(
    before: Option<char>,
    deleted: &str,
    inserted: &str,
    after: Option<char>,
) -> EditCounts {
    let mut following = [0_u8; 4];
    let following: &str = match after {
        Some(ch) => ch.encode_utf8(&mut following),
        None => "",
    };

    let (deleted_chars, deleted_visible) = char_counts(deleted);
    let (inserted_chars, inserted_visible) = char_counts(inserted);

    EditCounts {
        chars: inserted_chars - deleted_chars,
        visible: inserted_visible - deleted_visible,
        words: word_starts(before, [inserted, following]) as isize
            - word_starts(before, [deleted, following]) as isize,
    }
}

// Which selection a set of selection statistics describes.
//
// The cursor epoch is needed because a `Word`/`Line` selection covers more
// than its two reported endpoints. A background count is shown or stored only
// for the key it was computed from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SelectionStatsKey {
    snapshot_version: u64,
    cursor_epoch: u64,
    cursor: text_editor::Cursor,
}

// An immutable selection slab handed to the statistics worker.
//
// Cloning the document handle and this byte range is constant-time. The
// selected bytes are neither copied nor scanned on the UI thread; the worker
// materializes only the selected range from the generation-pinned snapshot.
#[derive(Clone)]
pub(crate) struct SelectionStatsJob {
    document: TextSnapshot,
    range: std::ops::Range<usize>,
}

impl SelectionStatsJob {
    pub(crate) fn text(&self) -> String {
        self.document
            .slice_to_string(self.range.clone())
            .expect("selection range came from this snapshot")
    }
}

const EDITABLE_VIEWPORT_LINES: usize = 48;
const EDITABLE_UNDO_OPERATIONS: usize = 256;
const EDITABLE_UNDO_BYTES: usize = 8 * 1024 * 1024;
// cosmic-text's default `Buffer::tab_width`, which the iced editor does not
// override. Indent/Unindent performed against the authoritative store must
// use the same stop width as the same keystroke in an ordinary tab.
const EDITABLE_TAB_WIDTH: usize = 8;
// One Indent/Unindent step rewrites at most this many lines / bytes, so a
// document-wide selection never flattens and re-inserts a large document.
const EDITABLE_INDENT_LINES: usize = 4096;
const EDITABLE_INDENT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct VirtualHistoryEntry {
    document: TextSnapshot,
    caret_byte: usize,
    cost: usize,
}

// Why a bounded editable projection could not be rebuilt truthfully.
//
// An edit can create an overlong line after admission, for example by
// joining two admitted lines. A refusal touches no authoritative byte, and no
// stale projection is presented as the whole document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorFrameRefusal {
    // One complete logical line exceeds the per-paragraph byte limit.
    LineTooLarge {
        line: usize,
        // Byte length of that line, excluding LF.
        bytes: usize,
        // The configured per-line limit.
        limit: usize,
    },
    // Even one visible line without overscan exceeded a byte/line budget.
    Budget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameOutcome {
    Materialized,
    // Nothing was replaced; the previous projection is untouched and the
    // caller must roll the document back or degrade visibly.
    Refused(EditorFrameRefusal),
}

// The persistent, non-dismissible sentence the content pane publishes above
// a tab whose generation counters are exhausted.
//
// Resetting the counter in place would let a stale completion alias a fresh
// identity, so the only escapes are reopening the file or restarting.
pub(crate) const TAB_AUTHORITY_EXHAUSTED_MESSAGE: &str = "Document change authority is exhausted; save and reopen the file, or \
     restart the app, to keep editing.";

// Refusal returned by every admission that would mint a new
// [`Tab::snapshot_version`] or [`Tab::cursor_epoch`] past its ceiling.
//
// Exhaustion is durable per tab: every later mint refuses too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub(crate) struct AuthorityExhausted;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditDeltaOutcome {
    // The exact bytes were applied; the payload is what moved.
    Applied(crate::doc_index::DeltaBytes),
    // The delta did not address the snapshot: the mirror drifted and the
    // caller must pay for a full resync.
    Drifted,
    // The edit would have created a logical line the bounded editable
    // viewport cannot represent. The document was rolled back to its exact
    // pre-edit bytes; the caller must restore the projection and say so.
    RefusedOverlongLine {
        line: usize,
        // Byte length that line would have had.
        bytes: usize,
        // The configured per-line limit.
        limit: usize,
    },
    // The tab's `snapshot_version` counter is at its ceiling. The document
    // is untouched, but the iced editor already mutated `Content`; the caller
    // must restore it with [`Tab::restore_content_from_document`].
    AuthorityExhausted,
}

// The outcome of a document write the app performs itself, without a cosmic
// delta: a selection replacement, a cut, or a whole-document reload.
//
// Each write is atomic: on refusal the bytes, native projection, dirty flag
// and undo/redo history are unchanged, and the caller publishes the sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub(crate) enum VirtualWriteOutcome {
    // Bytes moved; the projection describes the new document.
    Applied,
    Unchanged,
    // Refused. Nothing changed anywhere; the caller must say so.
    RefusedOverlongLine {
        line: usize,
        // Byte length that line would have had.
        bytes: usize,
        // The configured per-line limit.
        limit: usize,
    },
    // A line-oriented edit (Indent/Unindent) whose selection spans more of
    // the document than one bounded step may rewrite. Nothing changed.
    RefusedIndentScope {
        lines: usize,
        // Bytes the operation would have rewritten.
        bytes: usize,
        // The per-step line limit.
        line_limit: usize,
        // The per-step byte limit.
        byte_limit: usize,
    },
    // The tab's `snapshot_version` counter is at its ceiling. Nothing
    // changed.
    AuthorityExhausted,
}

impl VirtualWriteOutcome {
    // Whether authoritative bytes moved.
    #[must_use]
    pub(crate) fn changed(self) -> bool {
        matches!(self, Self::Applied)
    }

    // The refusal to publish, if this write was refused.
    #[must_use]
    pub(crate) fn refusal(self) -> Option<(usize, usize, usize)> {
        match self {
            Self::RefusedOverlongLine { line, bytes, limit } => Some((line, bytes, limit)),
            _ => None,
        }
    }

    // The one visible sentence for any refused write, so no route can refuse
    // silently just because a new refusal kind was added.
    #[must_use]
    pub(crate) fn refusal_sentence(self) -> Option<String> {
        match self {
            Self::RefusedOverlongLine { line, bytes, limit } => {
                Some(overlong_line_refused_sentence(line, bytes, limit))
            }
            Self::RefusedIndentScope {
                lines,
                bytes,
                line_limit,
                byte_limit,
            } => Some(indent_scope_refused_sentence(
                lines, bytes, line_limit, byte_limit,
            )),
            // Same sentence as the persistent banner, so the assertive live
            // region announces the refused gesture.
            Self::AuthorityExhausted => {
                Some(crate::i18n::domain_text(TAB_AUTHORITY_EXHAUSTED_MESSAGE))
            }
            Self::Applied | Self::Unchanged => None,
        }
    }
}

// A cut is a clipboard write *and* a document write, so it reports both. A
// refused cut copies nothing: putting the text on the clipboard while the
// document still holds it would make "cut" mean "copy" without saying so.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub(crate) enum VirtualCutOutcome {
    // The selection was removed; these bytes go to the clipboard.
    Cut(String),
    Nothing,
    // Refused: nothing copied, nothing written.
    RefusedOverlongLine {
        line: usize,
        // Byte length that line would have had.
        bytes: usize,
        // The configured per-line limit.
        limit: usize,
    },
}

// The one visible sentence for a refused over-limit edit. It says
// what was refused, why, and that the file is unchanged — a silent no-op
// would leave a keyboard user believing the edit landed.
#[must_use]
pub(crate) fn overlong_line_refused_sentence(line: usize, bytes: usize, limit: usize) -> String {
    enzui_core::localized!("text-edit-not-applied-it-would-make-line-6f2452", "p0" => line.saturating_add(1), "p1" => bytes.div_ceil(1024), "p2" => limit / 1024)
}

// The one visible sentence for an Indent/Unindent refused because its
// selection is larger than one bounded step may rewrite. Silence here
// would look exactly like an indent that did nothing.
#[must_use]
pub(crate) fn indent_scope_refused_sentence(
    lines: usize,
    bytes: usize,
    line_limit: usize,
    byte_limit: usize,
) -> String {
    enzui_core::localized!("text-indent-not-applied-the-selection-covers-lines-cec85a", "lines" => lines, "p0" => bytes.div_ceil(1024), "line_limit" => line_limit, "p1" => byte_limit / 1024)
}

fn indent_edit(line: &str, indent: bool) -> (std::ops::Range<usize>, usize) {
    if indent {
        let Some((index, count)) = line
            .char_indices()
            .enumerate()
            .find_map(|(count, (index, ch))| (!ch.is_whitespace()).then_some((index, count)))
        else {
            // cosmic-text inserts nothing on an all-whitespace line.
            return (0..0, 0);
        };
        let required = EDITABLE_TAB_WIDTH - (count % EDITABLE_TAB_WIDTH);
        (index..index, required)
    } else {
        let mut after_whitespace = line.len();
        let mut last_indent = 0;
        for (count, (index, ch)) in line.char_indices().enumerate() {
            if !ch.is_whitespace() {
                after_whitespace = index;
                break;
            }
            if count % EDITABLE_TAB_WIDTH == 0 {
                last_indent = index;
            }
        }
        if last_indent == after_whitespace {
            return (0..0, 0);
        }
        (last_indent..after_whitespace, 0)
    }
}

// Comparing a previously shifted endpoint with the next edit's old coordinate
// applies too few later shifts (notably during Unindent). Keeping `byte`
// immutable and accumulating signed deltas makes anchor and active symmetric,
// including an endpoint inside removed indentation.
fn map_line_edit_endpoint(byte: usize, edits: &[(std::ops::Range<usize>, usize)]) -> usize {
    let mut delta = 0_isize;
    for (removed, inserted) in edits {
        if byte < removed.start {
            break;
        }
        if removed.is_empty() {
            // Indentation is inserted immediately before an endpoint at the
            // first non-whitespace byte, matching cosmic's selection mapping.
            delta = delta.saturating_add(*inserted as isize);
        } else if byte < removed.end {
            // The endpoint was inside whitespace that disappeared.
            return removed.start.saturating_add_signed(delta);
        } else {
            delta = delta
                .saturating_add(*inserted as isize)
                .saturating_sub(removed.len() as isize);
        }
    }
    byte.saturating_add_signed(delta)
}

pub(crate) struct Tab {
    pub id: String,
    pub content: text_editor::Content,
    document: TextStore,
    // Present only when dense-line residency would make a full cosmic buffer
    // exceed its measured budget. In this mode `content` contains only this
    // cache's visible+overscan materialization.
    virtual_editor: Option<VirtualEditorState>,
    pub dirty: bool,
    pub(crate) preview: crate::markdown_preview::MarkdownPreview,
    pub file_path: Option<PathBuf>,
    // The workspace root this tab was opened under, or `None` for the Open
    // Files context. Tabs are one global set; switching workspace never
    // rewrites this.
    pub workspace_root: Option<PathBuf>,
    pub line_ending: LineEnding,
    pub detected_line_ending: LineEnding,
    pub language_override: Option<String>,
    // The buffer version, bumped on every document change; derived scans are
    // cached against it in `Cell`s because the view path only has `&Tab`.
    snapshot_version: u64,
    stats_pending: bool,
    stats_in_flight: bool,
    // A background save of this tab is in flight; the buffer stays editable.
    // One writer per tab, so an older generation never renames after a newer.
    pub saving: bool,
    pub save_queued: Option<Option<std::path::PathBuf>>,
    line_count_cache: std::cell::Cell<Option<(u64, usize)>>,
    text_stats_cache: std::cell::Cell<Option<(u64, crate::stats::TextStats)>>,
    doc_stats_cache: std::cell::Cell<Option<(u64, crate::doc_stats::Stats)>>,
    // How many editor actions this tab has performed. Keys selection
    // statistics, because a `Word`/`Line` selection can change range while
    // the reported cursor pair stays identical. A repaint never bumps it.
    cursor_epoch: u64,
    // The last selection statistics a worker landed, with their key. The view
    // never scans the selection; a mismatched key renders as "counting".
    selection_stats_landed: Option<(SelectionStatsKey, crate::doc_stats::Stats)>,
    // One selection count is out with a worker. Only the debounce settle
    // tick (`Message::SelectionStatsSettleTick`) starts a replacement, so a
    // drag never starts a count per pointer event.
    selection_stats_in_flight: bool,
    language_cache:
        std::cell::RefCell<Option<(u64, Option<String>, enzui_widgets::syntax::Language)>>,
    pub watcher: Option<FileWatcher>,
    // An external change was detected on disk; cleared on refresh or dismiss.
    pub externally_changed: bool,
    pub loading: bool,
    // The file is not valid UTF-8 and shows what decodes; the tab is
    // read-only so a save cannot change bytes the user never edited.
    pub decoded_lossily: bool,
    // The tab that was active when this one opened, returned to if the open
    // fails, so a failure never lands the user on an unrelated document.
    pub opened_from: Option<String>,
    // The tab's buffer mode. Every save/edit/replace path branches on this, so
    // a `LargeReadOnly` paged window is never mistaken for the whole file.
    pub buffer: crate::large_doc::DocumentBuffer,
    // The load generation this tab awaits; a completion with any other token
    // is stale and dropped.
    pub load_token: crate::large_doc::LoadToken,
    // A recovered viewport offset to reinstate once the editor has real
    // geometry; any user editor action supersedes it.
    pub pending_scroll: Option<f32>,
    // `snapshot_version` or `cursor_epoch` reached its ceiling; edits and
    // installs are refused from then on. Cleared only by a restart.
    authority_exhausted: bool,
}

impl Tab {
    pub(crate) fn total_lines(&self) -> usize {
        if let Some((version, value)) = self.line_count_cache.get()
            && version == self.snapshot_version
        {
            return value;
        }
        let value = self.document.current().line_count();
        self.line_count_cache
            .set(Some((self.snapshot_version, value)));
        value
    }

    fn display_lines(&self) -> usize {
        self.document.current().line_count()
    }
}
// A Replace All plan that [`Tab::preflight_replace_spans`] proved admissible,
// consumed once by [`Tab::commit_prepared_replace`].
//
// The all-tabs driver bounds the bytes and spans retained across all tokens
// by [`crate::search::MAX_REPLACE_BATCH_OUTPUT_BYTES`] and
// [`crate::search::MAX_REPLACE_BATCH_MATCHES`].
#[derive(Debug)]
pub(crate) struct PreparedReplace {
    pub tab_id: String,
    // The snapshot version the preflight ran against.
    pub snapshot_version: u64,
    // The exact bytes the commit will install.
    pub new_text: String,
    pub spans: Vec<crate::search::ReplaceSpan>,
}

impl PreparedReplace {}

pub(crate) struct Model {
    pub tabs: Vec<Tab>,
    pub active_tab_id: String,
    pub next_tab_seq: u64,
    pub settings: Settings,
    pub search: SearchState,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub goto_open: bool,
    pub goto_query: String,
    pub stats_open: bool,
    pub settings_open: bool,
    pub shortcuts_open: bool,
    pub language_menu_open: bool,
    // Mobile titlebar overflow "…" menu open.
    pub overflow_menu_open: bool,
    // The titlebar tab list (open-document switcher on the strip's overflow
    // chevron) is open.
    pub tab_list_open: bool,
    // Pending unsaved-changes close confirmation, holding the tab id to close
    // once confirmed.
    pub pending_close: Option<String>,
    // Last observed window size, from `Resized`. `None` until the first real
    // configure event, so a stale first frame is not photographed at the wrong
    // size.
    pub window_size: Option<(f32, f32)>,
    pub doc_tabs: DocumentTabs,
    // The size at or below which a document opens as an editable buffer.
    // Defaults to [`crate::large_doc::LARGE_DOCUMENT_THRESHOLD_BYTES`].
    pub large_threshold: u64,
    // The monotonic source of per-tab load generations.
    pub load_tokens: crate::large_doc::LoadTokenSource,
}

impl Default for Model {
    fn default() -> Self {
        let initial = Tab::new("tab-1".to_owned());
        let mut model = Self {
            tabs: vec![initial],
            active_tab_id: "tab-1".to_owned(),
            next_tab_seq: 2,
            settings: Settings::default(),
            search: SearchState::default(),
            cursor_line: 1,
            cursor_col: 1,
            goto_open: false,
            goto_query: String::new(),
            stats_open: false,
            settings_open: false,
            shortcuts_open: false,
            language_menu_open: false,
            overflow_menu_open: false,
            tab_list_open: false,
            pending_close: None,
            window_size: None,
            doc_tabs: DocumentTabs::new(),
            large_threshold: crate::large_doc::LARGE_DOCUMENT_THRESHOLD_BYTES,
            load_tokens: crate::large_doc::LoadTokenSource::default(),
        };
        model.sync_doc_tabs();
        model
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingOpen {
    pub tab_id: String,
    pub path: PathBuf,
    pub workspace: Option<PathBuf>,
    // Generation the completion must still match.
    pub token: crate::large_doc::LoadToken,
}
