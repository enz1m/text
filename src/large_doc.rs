//! Metadata-first text admission and bounded editable or read-only document views.
//!
//! Oversized files remain random-access sources. Complete and viewport editors
//! retain distinct buffer types; tab and page tokens reject stale completions.

use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use enzui_core::chunked_text::TextStore;
use enzui_core::editable_text_budget::{
    EditableTextBudget, EditableTextDecision, EditableTextRefusal, ResidentStop,
};
use enzui_core::file_watch::FileIdentity;
use enzui_core::paged_bytes::{
    PagedByteSource, PagedByteState, PagedByteViewport, PreviewKind, classify_bytes,
    classify_source,
};
use enzui_core::paged_text::{
    FilePagedTextSource, NavigateOutcome, PagedTextNavigation, PagedTextOperation, PagedTextPolicy,
    PagedTextSource, PagedTextSourceError, PagedTextState, PagedTextViewport,
};
use enzui_widgets::paged_text_viewer::PagedTextViewportState;

mod capability;

pub(crate) use capability::{
    Capability, UNSEARCHABLE_COUNTER, capability_refused_sentence, large_read_only_supports,
    save_refused_sentence,
};

// Absolute byte ceiling for editable ownership.
//
// Files at or below it are editable when their line shape also fits
// [`editable_budget`]; larger files open in the paged read-only mode.
pub(crate) const LARGE_DOCUMENT_THRESHOLD_BYTES: u64 = 40 * 1024 * 1024;

// Largest document constructed as one native editor buffer.
//
// Larger editable files use the chunked viewport editor, so the reducer only
// constructs a bounded native window.
pub(crate) const EAGER_EDITOR_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;

// Native editor admission bounds both bytes and paragraph count.
pub(crate) const EDITABLE_RESIDENT_BUDGET_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const RESIDENT_BYTES_PER_SOURCE_BYTE: u64 = 2;
pub(crate) const RESIDENT_BYTES_PER_LOGICAL_LINE: u64 = 248;

// The longest single line the editable path will accept.
//
// cosmic-text shapes one logical line as one paragraph regardless of the
// viewport size, so a file with a longer line opens in the paged viewer.
pub(crate) const MAX_EDITABLE_LINE_BYTES: usize = 1 << 20;

// The complete eager-editor admission policy.
//
// The framework scanner keeps no source bytes and exits as soon as refusal is
// certain. `source_threshold` is injectable so boundary tests can stay small.
#[must_use]
pub(crate) fn editable_budget(source_threshold: u64) -> EditableTextBudget {
    EditableTextBudget {
        max_source_bytes: source_threshold,
        max_resident_bytes: EDITABLE_RESIDENT_BUDGET_BYTES,
        resident_bytes_per_source_byte: RESIDENT_BYTES_PER_SOURCE_BYTE,
        resident_bytes_per_logical_line: RESIDENT_BYTES_PER_LOGICAL_LINE,
        max_line_bytes: MAX_EDITABLE_LINE_BYTES as u64,
    }
}

pub(crate) const DEFAULT_VISIBLE_ROWS: usize = 20;

// The paging policy text-app runs with (framework defaults: one 64 KiB chunk
// per request, 512 KiB retained, 4 KiB display rows).
#[must_use]
pub(crate) fn policy() -> PagedTextPolicy {
    PagedTextPolicy::default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentClass {
    // Within the byte ceiling; content shape still must pass [`editable_budget`].
    Editable {
        // Byte length observed by `stat`.
        byte_len: u64,
    },
    // Too large to shape eagerly; opened read-only and paged.
    LargeReadOnly {
        // Byte length observed by `stat`.
        byte_len: u64,
    },
}

impl DocumentClass {
    // Whether this class must open read-only.
    #[must_use]
    pub(crate) fn is_large(self) -> bool {
        matches!(self, Self::LargeReadOnly { .. })
    }

    // Observed byte length.
    #[must_use]
    pub(crate) fn byte_len(self) -> u64 {
        match self {
            Self::Editable { byte_len } | Self::LargeReadOnly { byte_len } => byte_len,
        }
    }
}

// Apply the metadata-only byte ceiling against `threshold`.
//
// The boundary is exclusive: a file exactly at the threshold remains an
// eager *candidate*. [`load_document`] then applies the line and resident
// bounds before admitting it to the editor.
#[must_use]
pub(crate) fn classify_len(byte_len: u64, threshold: u64) -> DocumentClass {
    if byte_len > threshold {
        DocumentClass::LargeReadOnly { byte_len }
    } else {
        DocumentClass::Editable { byte_len }
    }
}

// One forced `stat` failure, armed per thread — the deterministic stand-in
// for a denied read, which a privileged test process cannot obtain from Unix
// mode bits. It fires at the exact site where a real `EACCES` from
// `stat(2)` would enter the loader.
#[cfg(test)]
pub(crate) mod fault {
    use std::cell::Cell;
    use std::io;
    thread_local! {
        static ARMED: Cell<Option<io::ErrorKind>> = const { Cell::new(None) };
    }

    pub(crate) fn take() -> Option<io::Error> {
        ARMED.with(|a| {
            a.take()
                .map(|kind| io::Error::new(kind, "injected: permission denied"))
        })
    }
}

// The pathname is read once so the class and the identity that seeds the
// tab's watcher come from the same metadata sample. Callers already on the
// loading worker use this to skip a second `stat(2)` and to detect a
// mid-load replacement of the pathname.
fn stat_and_classify(
    path: &Path,
    threshold: u64,
) -> Result<(DocumentClass, FileIdentity), crate::document_failure::DocumentFailure> {
    #[cfg(test)]
    if let Some(error) = fault::take() {
        return Err(crate::document_failure::DocumentFailure::from_io(
            path, error,
        ));
    }
    let meta = std::fs::metadata(path)
        .map_err(|error| crate::document_failure::DocumentFailure::from_io(path, error))?;
    if !meta.is_file() {
        return Err(crate::document_failure::DocumentFailure::from_detail(
            path,
            "not a regular file",
        ));
    }
    Ok((
        classify_len(meta.len(), threshold),
        FileIdentity::from_metadata(&meta),
    ))
}

// A completed open or reload, shared by every load route.
//
// `identity` seeds the tab's watcher with what the worker actually read, so a
// replacement after load reads as
// [`enzui_core::file_watch::FileWatchOutcome::Replaced`].
#[derive(Clone)]
pub(crate) struct LoadedDocument {
    // Buffer shape (eager editable, viewport editable, or paged read-only).
    pub contents: LoadedContents,
    // Identity of the file the worker actually read from, or `None` when a
    // synthetic test injection carries no on-disk baseline.
    pub identity: Option<FileIdentity>,
    // Whether a decoded snapshot contains replacement characters and must
    // remain read-only. File loads use binary paging instead of lossy snapshots.
    pub decoded_lossily: bool,
}

impl std::fmt::Debug for LoadedDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedDocument")
            .field("contents", &self.contents)
            .field("identity", &self.identity)
            .finish()
    }
}

#[derive(Clone)]
pub(crate) enum LoadedContents {
    Editable {
        contents: String,
        detected_line_ending: crate::line_ending::LineEnding,
    },
    // Complete text owned by the chunked store, with only a viewport window
    // materialized into the native editor. This is selected for every file
    // above [`EAGER_EDITOR_THRESHOLD_BYTES`] and for smaller files whose
    // per-line eager objects would exceed the resident budget.
    EditableViewport {
        // Fully prepared authoritative store, built on the loading worker so
        // the reducer only materializes a bounded visible window.
        document: TextStore,
        detected_line_ending: crate::line_ending::LineEnding,
    },
    // A read-only random-access source plus the length `stat` reported.
    LargeReadOnly {
        // Random-access byte source, opened off the reducer.
        source: PreparedPagedSource,
        // Byte length at open time.
        byte_len: u64,
        // Why eager editing was refused; the banner must tell the truth.
        reason: PagedReason,
    },
}

#[derive(Clone)]
pub(crate) enum PreparedPagedSource {
    // UTF-8 text read in bounded pages.
    Text(Arc<dyn PagedTextSource>),
    // Hexadecimal rows prepared without retaining the original file in memory.
    Bytes(Arc<PagedByteSource>),
}

impl PagedTextSource for PreparedPagedSource {
    fn len(&self) -> Result<u64, PagedTextSourceError> {
        match self {
            Self::Text(source) => source.len(),
            Self::Bytes(source) => source.len(),
        }
    }
    fn read_at(&self, offset: u64, length: usize) -> Result<Vec<u8>, PagedTextSourceError> {
        match self {
            Self::Text(source) => source.read_at(offset, length),
            Self::Bytes(source) => source.read_at(offset, length),
        }
    }
}

impl<S: PagedTextSource + 'static> From<Arc<S>> for PreparedPagedSource {
    fn from(source: Arc<S>) -> Self {
        Self::Text(source)
    }
}

impl From<Arc<dyn PagedTextSource>> for PreparedPagedSource {
    fn from(source: Arc<dyn PagedTextSource>) -> Self {
        Self::Text(source)
    }
}

// Why a document uses the bounded read-only viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PagedReason {
    // Arbitrary bytes are shown through a bounded hexadecimal preview.
    Binary,
    // Source metadata exceeded the absolute eager byte ceiling.
    SourceBytes,
    // One logical line exceeded cosmic-text's safe paragraph bound.
    LineBytes { observed: u64 },
}

impl std::fmt::Debug for LoadedContents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Editable { contents, .. } => f
                .debug_struct("Editable")
                .field("bytes", &contents.len())
                .finish(),
            Self::EditableViewport { document, .. } => f
                .debug_struct("EditableViewport")
                .field("bytes", &document.current().len())
                .finish(),
            Self::LargeReadOnly {
                byte_len, reason, ..
            } => f
                .debug_struct("LargeReadOnly")
                .field("byte_len", byte_len)
                .field("reason", reason)
                .finish(),
        }
    }
}

// The one metadata-first open operation. Production submits it to the app's
// bounded blocking owner; the capture settle path may run it before first paint.
//
// Every file has a bounded prefix check; binary bytes use an ASCII preview.
// Text candidates pass source, resident and paragraph budgets before a whole
// read, whose bytes are validated before native editor ownership.
pub(crate) fn load_document(
    path: &Path,
    threshold: u64,
) -> Result<LoadedDocument, crate::document_failure::DocumentFailure> {
    let (class, initial_identity) = stat_and_classify(path, threshold)?;
    let paged_source = FilePagedTextSource::open(path)
        .map_err(|error| crate::document_failure::DocumentFailure::from_paged(path, error))?;
    let classification = classify_source(&paged_source)
        .map_err(|error| crate::document_failure::DocumentFailure::from_paged(path, error))?;
    if classification.kind == PreviewKind::Bytes {
        return load_binary_preview(path, paged_source, class.byte_len(), initial_identity);
    }
    // Identity binds the metadata decision to the descriptor that supplies pages.
    if class.is_large() {
        let identity = confirm_paged_identity(path, &paged_source, initial_identity)?;
        return Ok(LoadedDocument {
            contents: LoadedContents::LargeReadOnly {
                source: Arc::new(paged_source).into(),
                byte_len: class.byte_len(),
                reason: PagedReason::SourceBytes,
            },
            identity: Some(identity),
            decoded_lossily: false,
        });
    }

    // Preflight on the loading worker without retaining contents. Dense and
    // long-line files stop early, so constructing a 40 MiB String is not the
    // price of discovering that cosmic-text would allocate hundreds of MiB.
    let mut file = std::fs::File::open(path)
        .map_err(|error| crate::document_failure::DocumentFailure::from_io(path, error))?;
    // Whole-`FileIdentity` equality (not just inode and length) refuses an
    // mtime-only rewrite before the buffer is admitted.
    let opened_identity = FileIdentity::from_metadata(
        &file
            .metadata()
            .map_err(|error| crate::document_failure::DocumentFailure::from_io(path, error))?,
    );
    if opened_identity != initial_identity {
        return Err(crate::document_failure::DocumentFailure::from_detail(
            path,
            "file changed while it was being opened; try again",
        ));
    }
    // A resident refusal selects the bounded *editable* viewport, which
    // still cannot shape a paragraph over `MAX_EDITABLE_LINE_BYTES`. Stopping
    // at the first dense kilobyte would admit a file whose later 20 MiB line
    // every frame would then refuse, so the scan continues — retaining nothing
    // — until it can also answer the paragraph question.
    let decision = editable_budget(threshold)
        .assess_reader_stopping(
            class.byte_len(),
            BufReader::new(&mut file),
            ResidentStop::ContinueForLineBound,
        )
        .map_err(|error| crate::document_failure::DocumentFailure::from_io(path, error))?;
    let shape = decision.shape();
    let viewport_editable = class.byte_len() > EAGER_EDITOR_THRESHOLD_BYTES
        || matches!(
            &decision,
            EditableTextDecision::Paged {
                reason: EditableTextRefusal::ResidentBytes,
                ..
            }
        );
    if let EditableTextDecision::Paged { reason, .. } = decision
        && reason != EditableTextRefusal::ResidentBytes
    {
        let reason = match reason {
            EditableTextRefusal::SourceBytes => PagedReason::SourceBytes,
            EditableTextRefusal::LineBytes => PagedReason::LineBytes {
                observed: shape.longest_line_bytes,
            },
            EditableTextRefusal::ResidentBytes => unreachable!("handled by viewport editor"),
        };
        drop(file);
        let source = FilePagedTextSource::open(path)
            .map_err(|error| crate::document_failure::DocumentFailure::from_paged(path, error))?;
        let identity = confirm_paged_identity(path, &source, opened_identity)?;
        return Ok(LoadedDocument {
            contents: LoadedContents::LargeReadOnly {
                source: Arc::new(source).into(),
                byte_len: class.byte_len(),
                reason,
            },
            identity: Some(identity),
            decoded_lossily: false,
        });
    }

    file.rewind()
        .map_err(|error| crate::document_failure::DocumentFailure::from_io(path, error))?;
    let expected_bytes = if viewport_editable {
        class.byte_len()
    } else {
        shape.inspected_bytes
    };
    let mut bytes = Vec::with_capacity(expected_bytes as usize);
    file.read_to_end(&mut bytes)
        .map_err(|error| crate::document_failure::DocumentFailure::from_io(path, error))?;
    if bytes.len() as u64 != expected_bytes {
        return Err(crate::document_failure::DocumentFailure::from_detail(
            path,
            "file changed while its editable memory budget was being checked; try again",
        ));
    }
    // Any metadata change during the read means the decoded bytes may not match
    // one on-disk state, so neither they nor the baseline may seed the tab.
    let final_identity = FileIdentity::from_metadata(
        &file
            .metadata()
            .map_err(|error| crate::document_failure::DocumentFailure::from_io(path, error))?,
    );
    if final_identity != opened_identity {
        return Err(crate::document_failure::DocumentFailure::from_detail(
            path,
            "file changed while it was being read; try again",
        ));
    }

    // Detection, CR/LF normalization, and (for a viewport document) persistent
    // summary construction all happen in this loading worker. A completion
    // message never asks the UI reducer to scan the whole file before it can
    // paint the first useful frame.
    // A text-like prefix does not establish that the complete buffer is safe
    // for native shaping, including a truncated UTF-8 scalar at EOF.
    let contents = match String::from_utf8(bytes) {
        Ok(text) if classify_bytes(text.as_bytes(), true) == PreviewKind::Utf8Text => text,
        _ => return load_binary_preview(path, paged_source, class.byte_len(), final_identity),
    };
    let (detected_line_ending, contents) = crate::line_ending::load_normalized_owned(contents);
    let contents_shape = if viewport_editable {
        LoadedContents::EditableViewport {
            document: TextStore::from_string(contents),
            detected_line_ending,
        }
    } else {
        LoadedContents::Editable {
            contents,
            detected_line_ending,
        }
    };
    Ok(LoadedDocument {
        contents: contents_shape,
        identity: Some(final_identity),
        decoded_lossily: false,
    })
}

fn load_binary_preview(
    path: &Path,
    source: FilePagedTextSource,
    byte_len: u64,
    identity: FileIdentity,
) -> Result<LoadedDocument, crate::document_failure::DocumentFailure> {
    let identity = confirm_paged_identity(path, &source, identity)?;
    let source = PagedByteSource::new(Arc::new(source))
        .map_err(|error| crate::document_failure::DocumentFailure::from_paged(path, error))?;
    Ok(LoadedDocument {
        contents: LoadedContents::LargeReadOnly {
            source: PreparedPagedSource::Bytes(Arc::new(source)),
            byte_len,
            reason: PagedReason::Binary,
        },
        identity: Some(identity),
        decoded_lossily: false,
    })
}

// The source does not expose its descriptor, so a second pathname `stat(2)`
// plus the source's fd-visible `len()` stand in for fd identity.
fn confirm_paged_identity(
    path: &Path,
    source: &FilePagedTextSource,
    initial: FileIdentity,
) -> Result<FileIdentity, crate::document_failure::DocumentFailure> {
    let source_len = source
        .len()
        .map_err(|error| crate::document_failure::DocumentFailure::from_paged(path, error))?;
    let post_meta = std::fs::metadata(path)
        .map_err(|error| crate::document_failure::DocumentFailure::from_io(path, error))?;
    let post_identity = FileIdentity::from_metadata(&post_meta);
    if post_identity != initial || source_len != initial.len {
        return Err(crate::document_failure::DocumentFailure::from_detail(
            path,
            "file changed while it was being opened; try again",
        ));
    }
    Ok(post_identity)
}

// App-level per-tab load generation.
//
// Bumped by [`LoadTokenSource::next`] for every open, reopen, replacement and
// refresh; a tab stores the token it is currently waiting on. A completion
// whose token is not the tab's current token is dropped, which is what stops a
// stale read from resurrecting a closed tab or clobbering newer content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct LoadToken(u64);

#[derive(Debug, Clone, Default)]
pub(crate) struct LoadTokenSource {
    next: u64,
    exhausted: bool,
}

impl LoadTokenSource {
    // Issue the next token, or `None` once the counter is exhausted.
    //
    // Tokens never wrap: a reused value would let a stale completion match a
    // fresh load, so callers refuse the action instead.
    #[must_use]
    pub(crate) fn next(&mut self) -> Option<LoadToken> {
        if self.exhausted {
            return None;
        }
        match self.next.checked_add(1) {
            Some(next) => {
                self.next = next;
                Some(LoadToken(next))
            }
            None => {
                self.exhausted = true;
                None
            }
        }
    }
}

// One open read-only paged document belonging to a tab.
pub(crate) struct LargeDocument {
    pub path: PathBuf,
    pub byte_len: u64,
    paging: DocumentPaging,
    // The viewer's last resolved allocation, published during layout so
    // navigation and tail anchoring use the rows the frame painted.
    viewport_state: PagedTextViewportState,
    // Why eager editing was refused. The banner says which bound applied.
    reason: PagedReason,
}

enum DocumentPaging {
    Text {
        state: PagedTextState,
        viewport: PagedTextViewport,
    },
    Bytes {
        state: PagedByteState,
        viewport: PagedByteViewport,
    },
}

impl LargeDocument {
    // Open `source`, recording why the document is paged.
    //
    // The returned operation must be executed off the UI thread; its result
    // is applied with [`apply_page`](Self::apply_page).
    pub(crate) fn open_because(
        path: PathBuf,
        source: impl Into<PreparedPagedSource>,
        byte_len: u64,
        visible_rows: usize,
        reason: PagedReason,
    ) -> Result<(Self, PagedTextOperation), String> {
        let (paging, operation) = match source.into() {
            PreparedPagedSource::Text(source) => {
                let mut state = PagedTextState::new(policy());
                let operation = state.open(source).map_err(open_failure_sentence)?;
                (
                    DocumentPaging::Text {
                        state,
                        viewport: PagedTextViewport::new(visible_rows.max(1)),
                    },
                    operation,
                )
            }
            PreparedPagedSource::Bytes(source) => {
                let mut state = PagedByteState::new();
                let operation = state.open(source).map_err(open_failure_sentence)?;
                (
                    DocumentPaging::Bytes {
                        state,
                        viewport: PagedByteViewport::new(visible_rows.max(1)),
                    },
                    operation,
                )
            }
        };
        Ok((
            Self {
                path,
                byte_len,
                paging,
                viewport_state: PagedTextViewportState::new(),
                reason,
            },
            operation,
        ))
    }

    #[must_use]
    pub(crate) fn state(&self) -> &PagedTextState {
        match &self.paging {
            DocumentPaging::Text { state, .. } => state,
            DocumentPaging::Bytes { state, .. } => state.text_state(),
        }
    }

    pub(crate) fn is_binary_preview(&self) -> bool {
        self.reason == PagedReason::Binary
    }

    // Immutable viewport, for the view path.
    #[must_use]
    pub(crate) fn viewport(&self) -> &PagedTextViewport {
        match &self.paging {
            DocumentPaging::Text { viewport, .. } => viewport,
            DocumentPaging::Bytes { viewport, .. } => viewport.text_viewport(),
        }
    }

    pub(crate) fn set_visible_rows(&mut self, rows: usize) {
        match &mut self.paging {
            DocumentPaging::Text { viewport, .. } => viewport.visible_rows = rows.max(1),
            DocumentPaging::Bytes { viewport, .. } => viewport.visible_rows(rows),
        }
    }

    // The seam the paged viewer publishes its measured allocation into.
    //
    // Pass it to
    // [`EnzPagedTextViewer::viewport_state`](enzui_widgets::paged_text_viewer::EnzPagedTextViewer::viewport_state)
    // so painted rows and navigation rows cannot diverge.
    #[must_use]
    pub(crate) fn viewport_state(&self) -> &PagedTextViewportState {
        &self.viewport_state
    }

    // The row window the viewer last resolved from its own allocation,
    // or the current viewport count before the first layout pass has
    // published anything. This is the authority the reducer tail applies to
    // [`set_visible_rows`](Self::set_visible_rows); the value may lag the
    // current frame by one layout pass, which the policy's overscan covers.
    #[must_use]
    pub(crate) fn published_visible_rows(&self) -> usize {
        self.viewport_state
            .visible_rows_or(self.viewport().visible_rows)
    }

    // Apply a page result. Returns `false` when the framework rejected it as
    // stale (closed, refreshed, or superseded); nothing is cached then.
    //
    // A typed failure latches the document (see [`Self::load_error`]) and
    // issues no follow-up, since re-requesting the same anchor would loop.
    pub(crate) fn apply_page(
        &mut self,
        result: enzui_core::paged_text::PagedTextResult,
    ) -> Option<PagedTextOperation> {
        let was_latched = self.state().failure().is_some();
        let applied = match &mut self.paging {
            DocumentPaging::Text { state, .. } => state.apply(result),
            DocumentPaging::Bytes { state, .. } => state.apply(result),
        };
        if !applied {
            return None;
        }
        // The full payload is logged once per latch, escaped by `{:?}` into
        // `\u{..}` form so a hostile payload cannot forge a log record.
        if !was_latched && let Some(diagnostic) = self.load_error_diagnostic() {
            eprintln!("[text-app] paged document read failed: {diagnostic:?}");
        }
        self.resolve()
    }

    // Move the viewport and return the next bounded read, if the movement
    // crossed into an uncached page.
    pub(crate) fn navigate(&mut self, movement: PagedTextNavigation) -> Option<PagedTextOperation> {
        let chunk_size = self.chunk_size();
        match &mut self.paging {
            DocumentPaging::Text { state, viewport } => {
                viewport.navigate(movement, state.source_len(), chunk_size);
            }
            DocumentPaging::Bytes { state, viewport } => viewport.navigate(movement, state),
        }
        self.resolve()
    }

    pub(crate) fn byte_preview(&self) -> Option<(&PagedByteState, &PagedByteViewport)> {
        match &self.paging {
            DocumentPaging::Bytes { state, viewport } => Some((state, viewport)),
            DocumentPaging::Text { .. } => None,
        }
    }

    #[must_use]
    pub(crate) fn chunk_size(&self) -> usize {
        let len = self.state().source_len();
        let policy = self.state().policy();
        if usize::try_from(len).is_ok_and(|len| len <= policy.eager_cutoff) {
            usize::try_from(len.max(1)).unwrap_or(policy.read_chunk_size)
        } else {
            policy.read_chunk_size
        }
    }

    // Resolve the viewport against the cache, requesting the page it needs.
    //
    // Returns `None` while a load failure is latched: a failed read caches
    // nothing, so re-requesting the same anchor would loop forever. Only an
    // explicit refresh clears the latch.
    pub(crate) fn resolve(&mut self) -> Option<PagedTextOperation> {
        if self.state().failure().is_some() {
            return None;
        }
        match &mut self.paging {
            DocumentPaging::Text { state, viewport } => match viewport.resolve(state) {
                NavigateOutcome::Ready => None,
                NavigateOutcome::NeedsFetch(anchor) => state.request(anchor),
            },
            DocumentPaging::Bytes { state, viewport } => match viewport.resolve(state) {
                NavigateOutcome::Ready => None,
                NavigateOutcome::NeedsFetch(_) => state.request(viewport),
            },
        }
    }

    // The typed load failure this document is latched on, rendered as one
    // sentence. `None` while the document is healthy.
    //
    // The bytes are never made lossy to produce it: an `InvalidUtf8` failure
    // reports the offset, not the bytes.
    #[must_use]
    pub(crate) fn load_error(&self) -> Option<String> {
        let failure = self.state().failure()?;
        let name = crate::safe_text::bounded_path(&self.path);
        Some(
            enzui_core::localized!("text-could-not-read-part-of-p0-p1-2d9453",
                "p0" => name,
                "p1" => open_failure_sentence(failure.clone()),
            ),
        )
    }

    // The unmodified diagnostic for the latched failure, for logs.
    //
    // Kept separate from [`load_error`](Self::load_error) so the payload a
    // banner must never show is still available to structured logging.
    #[must_use]
    pub(crate) fn load_error_diagnostic(&self) -> Option<String> {
        self.state().failure().map(open_failure_diagnostic)
    }

    pub(crate) fn close(&mut self) {
        match &mut self.paging {
            DocumentPaging::Text { state, .. } => state.close(),
            DocumentPaging::Bytes { state, .. } => state.close(),
        }
    }
}

// The buffer mode of a tab. The read-only variant owns paging state.
#[derive(Default)]
pub(crate) enum DocumentBuffer {
    #[default]
    Editable,
    // Read-only paged window onto a file too large to shape eagerly.
    LargeReadOnly(Box<LargeDocument>),
}

impl DocumentBuffer {
    // Whether this tab refuses edits and writes.
    #[must_use]
    pub(crate) fn is_read_only(&self) -> bool {
        matches!(self, Self::LargeReadOnly(_))
    }

    // The paged document, when this tab is in large read-only mode.
    #[must_use]
    pub(crate) fn large(&self) -> Option<&LargeDocument> {
        match self {
            Self::LargeReadOnly(doc) => Some(doc),
            Self::Editable => None,
        }
    }

    // Whether this tab's mode supports `capability` — the single question the
    // view and the reducer both ask (see [`large_read_only_supports`]).
    #[must_use]
    pub(crate) fn supports(&self, capability: Capability) -> bool {
        match self {
            Self::Editable => true,
            Self::LargeReadOnly(_) => large_read_only_supports(capability),
        }
    }

    // Mutable paged document, when this tab is in large read-only mode.
    pub(crate) fn large_mut(&mut self) -> Option<&mut LargeDocument> {
        match self {
            Self::LargeReadOnly(doc) => Some(doc),
            Self::Editable => None,
        }
    }
}

impl std::fmt::Debug for DocumentBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Editable => f.write_str(&enzui_core::localization::text("text-editable-149344")),
            Self::LargeReadOnly(doc) => f
                .debug_struct("LargeReadOnly")
                .field("path", &doc.path)
                .field("byte_len", &doc.byte_len)
                .field("reason", &doc.reason)
                .finish(),
        }
    }
}

// Human byte size for the read-only explanation ("40.0 MB").
#[must_use]
pub(crate) fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1000.0;
    let b = bytes as f64;
    if b < KB {
        return enzui_core::localized!("text-bytes-bytes-31800b", "bytes" => bytes);
    }
    for (unit, scale) in [("kB", KB), ("MB", KB * KB), ("GB", KB * KB * KB)] {
        if b < scale * KB {
            return enzui_core::localized!("text-p0-unit-be511b", "p0" => crate::i18n::number(format!("{:.1}", b / scale)), "unit" => unit);
        }
    }
    enzui_core::localized!("text-p0-tb-64dbdd", "p0" => crate::i18n::number(format!("{:.1}", b / (KB * KB * KB * KB))))
}

// Render a typed paged-text failure as a sentence, without turning invalid
// bytes into text.
//
// A source failure renders the framework's classified summary
// ([`PagedTextSourceError::user_summary`](PagedTextSourceError::user_summary)),
// never the raw payload, which can carry errno text, control characters or
// paths. [`open_failure_diagnostic`] keeps the payload for logging.
#[must_use]
pub(crate) fn open_failure_sentence(
    failure: enzui_core::paged_text::PagedTextOpenFailure,
) -> String {
    use enzui_core::paged_text::PagedTextOpenFailure as F;
    match failure {
        F::Source(error) => error.user_summary(),
        F::InvalidUtf8 { offset } => {
            enzui_core::localized!("text-not-valid-utf-8-text-malformed-byte-605219", "offset" => offset)
        }
    }
}

// The unmodified diagnostic behind a typed failure — for structured logging
// or a crash report, never for a label.
#[must_use]
pub(crate) fn open_failure_diagnostic(
    failure: &enzui_core::paged_text::PagedTextOpenFailure,
) -> String {
    use enzui_core::paged_text::PagedTextOpenFailure as F;
    match failure {
        F::Source(error) => error.diagnostic().to_owned(),
        F::InvalidUtf8 { offset } => {
            format!("malformed UTF-8 at byte offset {offset}")
        }
    }
}
