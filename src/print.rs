//! Bounded snapshot printing through the desktop Print portal.
//!
//! Prepared PostScript travels in a sealed memfd; superseded generations are
//! discarded before submission.

use std::os::fd::OwnedFd;

use enzui_core::portal::CancelReason;
use enzui_core::portal::PortalOutcome;
use enzui_core::portal::print::{PrintFormat, PrintReceipt, PrintRequest};

const PAGE_WIDTH_PT: f32 = 612.0;
const PAGE_HEIGHT_PT: f32 = 792.0;
const MARGIN_PT: f32 = 54.0;
// Courier at 10 pt with 12 pt leading — a monospace face so the column math
// that drives wrapping is exact and needs no font-metric queries.
const FONT_SIZE_PT: f32 = 10.0;
const LINE_LEADING_PT: f32 = 12.0;
// Courier's advance width is exactly 0.6 em.
const CHAR_ADVANCE_PT: f32 = FONT_SIZE_PT * 0.6;

pub(crate) const TAB_WIDTH: usize = 4;

// Upper bound on emitted pages. A document longer than this is printed
// truncated (with the truncation disclosed), never unbounded.
pub(crate) const MAX_PAGES: usize = 1000;

// Upper bound on emitted PostScript bytes. Reaching it stops the render at the
// last whole page, so a hostile or huge buffer cannot exhaust memory.
pub(crate) const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

fn columns_per_line() -> usize {
    let usable = PAGE_WIDTH_PT - 2.0 * MARGIN_PT;
    ((usable / CHAR_ADVANCE_PT).floor() as usize).max(1)
}

fn rows_per_page() -> usize {
    let usable = PAGE_HEIGHT_PT - 2.0 * MARGIN_PT;
    ((usable / LINE_LEADING_PT).floor() as usize).max(1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedPostScript {
    // The PostScript program bytes: pure 7-bit ASCII (high bytes are octal
    // escaped), so the file is portable and valid.
    pub bytes: Vec<u8>,
    pub pages: usize,
    // Whether a page or byte bound truncated the document.
    pub truncated: bool,
}

// PostScript's base fonts are single-byte encoded. The policy is explicit and
// lossless within Latin-1: a scalar in `U+0000..=U+00FF` is emitted as that
// byte (rendered through `ISOLatin1Encoding`); every scalar above `U+00FF` is
// replaced with `?` (`0x3F`). Control characters are handled by the caller
// (tab expansion, line breaks) before this point; any that reach here are
// emitted as their byte and octal-escaped, which a PostScript interpreter
// ignores as a non-printing glyph.
fn latin1_byte(c: char) -> u8 {
    let cp = c as u32;
    if cp <= 0x00FF { cp as u8 } else { b'?' }
}

// `(`, `)` and `\` take backslash escapes; printable ASCII is literal; every
// other byte (control or high Latin-1) is emitted as a three-digit octal
// escape, keeping the whole program 7-bit ASCII.
fn escape_ps_byte(byte: u8, out: &mut Vec<u8>) {
    match byte {
        b'(' => out.extend_from_slice(b"\\("),
        b')' => out.extend_from_slice(b"\\)"),
        b'\\' => out.extend_from_slice(b"\\\\"),
        0x20..=0x7E => out.push(byte),
        other => {
            out.push(b'\\');
            out.push(b'0' + ((other >> 6) & 0x07));
            out.push(b'0' + ((other >> 3) & 0x07));
            out.push(b'0' + (other & 0x07));
        }
    }
}

fn push_escaped_run(text: &str, out: &mut Vec<u8>) {
    for c in text.chars() {
        escape_ps_byte(latin1_byte(c), out);
    }
}

fn expand_tabs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut col = 0usize;
    for c in line.chars() {
        if c == '\t' {
            let advance = TAB_WIDTH - (col % TAB_WIDTH);
            for _ in 0..advance {
                out.push(' ');
            }
            col += advance;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

// A line no wider than `cols` yields exactly itself; an empty line yields one
// empty visual row so blank lines in the source print as blank lines.
fn wrap_line(line: &str, cols: usize) -> Vec<String> {
    let expanded = expand_tabs(line);
    let chars: Vec<char> = expanded.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    let mut rows = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + cols).min(chars.len());
        rows.push(chars[start..end].iter().collect());
        start = end;
    }
    rows
}

fn logical_lines(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    normalized.split('\n').map(str::to_owned).collect()
}

fn visual_rows(text: &str, cols: usize) -> Vec<String> {
    logical_lines(text)
        .into_iter()
        .flat_map(|line| wrap_line(&line, cols))
        .collect()
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
}

// Render `text` to a bounded, valid PostScript program titled `title`.
//
// The document is monospaced Courier on US Letter with 0.75-inch margins.
// Rendering stops at [`MAX_PAGES`] or [`MAX_OUTPUT_BYTES`], whichever comes
// first; an empty document still emits exactly one blank page.
#[must_use]
pub(crate) fn render_postscript(text: &str, title: &str) -> RenderedPostScript {
    let cols = columns_per_line();
    let rows = rows_per_page();
    let all_rows = visual_rows(text, cols);

    // At least one page even for the empty document.
    let total_pages_needed = all_rows.len().div_ceil(rows).max(1);
    let page_cap = total_pages_needed.min(MAX_PAGES);

    let mut body = Vec::new();
    let mut emitted_pages = 0usize;
    let mut truncated = total_pages_needed > page_cap;

    'pages: for page_index in 0..page_cap {
        // Refuse to exceed the byte ceiling: stop at the last whole page.
        if body.len() >= MAX_OUTPUT_BYTES {
            truncated = true;
            break;
        }
        let page_number = page_index + 1;
        push_str(&mut body, &format!("%%Page: {page_number} {page_number}\n"));
        // `BodyFont` is the ISO-Latin-1 Courier defined once in the prologue.
        push_str(&mut body, "BodyFont setfont\n");

        let top = PAGE_HEIGHT_PT - MARGIN_PT - FONT_SIZE_PT;
        let start_row = page_index * rows;
        let end_row = (start_row + rows).min(all_rows.len());
        for (row_offset, row) in all_rows[start_row..end_row].iter().enumerate() {
            let baseline = top - (row_offset as f32) * LINE_LEADING_PT;
            push_str(&mut body, &format!("{MARGIN_PT:.1} {baseline:.1} moveto ("));
            push_escaped_run(row, &mut body);
            push_str(&mut body, ") show\n");
        }
        push_str(&mut body, "showpage\n");
        emitted_pages += 1;

        if body.len() >= MAX_OUTPUT_BYTES && page_index + 1 < page_cap {
            truncated = true;
            break 'pages;
        }
    }

    let mut out = Vec::with_capacity(body.len() + 512);
    push_str(&mut out, "%!PS-Adobe-3.0\n");
    push_str(&mut out, "%%Creator: enzUI text-app\n");
    push_str(&mut out, "%%Title: (");
    push_escaped_run(title, &mut out);
    push_str(&mut out, ")\n");
    push_str(
        &mut out,
        &format!("%%BoundingBox: 0 0 {PAGE_WIDTH_PT:.0} {PAGE_HEIGHT_PT:.0}\n"),
    );
    push_str(&mut out, &format!("%%Pages: {emitted_pages}\n"));
    push_str(&mut out, "%%EndComments\n");
    // Courier's Latin-1 encoding preserves the escaped high-byte glyphs.
    push_str(&mut out, "%%BeginProlog\n");
    push_str(
        &mut out,
        &format!(
            "/BodyFont\n  /Courier findfont\n  dup length dict begin\n    \
             {{ 1 index /FID ne {{ def }} {{ pop pop }} ifelse }} forall\n    \
             /Encoding ISOLatin1Encoding def\n    currentdict\n  end\n  \
             /Courier-Latin1 exch definefont\n  {FONT_SIZE_PT:.1} scalefont\ndef\n"
        ),
    );
    push_str(&mut out, "%%EndProlog\n");
    out.extend_from_slice(&body);
    push_str(&mut out, "%%Trailer\n%%EOF\n");

    RenderedPostScript {
        bytes: out,
        pages: emitted_pages.max(1),
        truncated,
    }
}

// The sealed, rewound memfd stays anonymous and owned until the portal consumes it.
pub(crate) fn sealed_document(bytes: &[u8]) -> std::io::Result<OwnedFd> {
    use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
    use std::io::{Seek, SeekFrom, Write};

    let memfd = memfd_create(
        "enzui-text-print",
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )
    .map_err(std::io::Error::from)?;
    let mut file = std::fs::File::from(memfd);
    file.write_all(bytes)?;
    file.flush()?;
    // Rewind so the portal (and any inspector) reads the document from zero.
    file.seek(SeekFrom::Start(0))?;
    // Seal against writes and resizing; reads and seeks stay permitted.
    fcntl_add_seals(
        &file,
        SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL,
    )
    .map_err(std::io::Error::from)?;
    Ok(OwnedFd::from(file))
}

// A one-request-at-a-time seam over the print portal.
//
// The real implementation drives [`enzui_core::portal::print`] through a
// `DbusBackend`; a test fixture drives the same reducer paths without a
// printer. Exactly one request is live at a time (a second [`start`] while one
// is live is refused), mirroring the file chooser's one-dialog rule.
//
// [`start`]: PrintSpool::start
pub(crate) trait PrintSpool {
    fn start(&mut self, request: PrintRequest) -> Result<(), PortalOutcome<PrintReceipt>>;

    fn poll(&mut self) -> Option<PortalOutcome<PrintReceipt>>;

    fn cancel(&mut self, reason: CancelReason);

    fn settle(&mut self);

    fn is_live(&self) -> bool;
}

// The live route: the real XDG Print portal over a lazily built `DbusBackend`.
//
// The backend is constructed on first use (it spawns a tokio runtime, so it is
// not built at boot). A construction failure is reported as
// [`PortalOutcome::Unavailable`] — the portal genuinely cannot be reached.
#[cfg(feature = "portal-dbus")]
#[derive(Default)]
pub(crate) struct DbusPrintSpool {
    backend: Option<enzui_core::portal::dbus_backend::DbusBackend>,
    backend_failed: bool,
    pending: Option<enzui_core::portal::print::PendingPrint>,
    queued: Option<PrintRequest>,
}

#[cfg(feature = "portal-dbus")]
impl DbusPrintSpool {
    fn backend_mut(&mut self) -> Option<&mut enzui_core::portal::dbus_backend::DbusBackend> {
        if self.backend.is_none() && !self.backend_failed {
            match enzui_core::portal::dbus_backend::DbusBackend::new() {
                Ok(backend) => self.backend = Some(backend),
                Err(_) => self.backend_failed = true,
            }
        }
        self.backend.as_mut()
    }
}

#[cfg(feature = "portal-dbus")]
impl PrintSpool for DbusPrintSpool {
    fn start(&mut self, request: PrintRequest) -> Result<(), PortalOutcome<PrintReceipt>> {
        if self.is_live() {
            return Err(PortalOutcome::Failed);
        }
        if self.backend_mut().is_none() {
            return Err(PortalOutcome::Unavailable);
        }
        self.queued = Some(request);
        Ok(())
    }

    fn poll(&mut self) -> Option<PortalOutcome<PrintReceipt>> {
        use enzui_core::portal::backend::PortalBackend;

        let backend = self.backend.as_mut()?;
        // This backend serves only printing; its first completion settles bounded readiness.
        if backend
            .completion_signal()
            .is_some_and(|signal| signal.generation() > 0)
            && let Some(request) = self.queued.take()
        {
            match request.start(backend) {
                Ok(pending) => self.pending = Some(pending),
                Err(outcome) => return Some(outcome),
            }
        }
        self.pending.as_mut()?.poll(backend)
    }

    fn cancel(&mut self, reason: CancelReason) {
        self.queued = None;
        if let Some(pending) = self.pending.as_mut() {
            let _ = pending.cancel(reason);
        }
    }

    fn settle(&mut self) {
        self.queued = None;
        if let (Some(backend), Some(pending)) = (self.backend.as_mut(), self.pending.as_mut()) {
            let _ = pending.settle(backend);
        }
        self.pending = None;
    }

    fn is_live(&self) -> bool {
        self.queued.is_some() || self.pending.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PrintStatus {
    // The spool accepted the whole document for the printer.
    Submitted,
    // The spool accepted the job, but only `pages` (the `MAX_PAGES` cap) were
    // sent because the document exceeded the page or byte bound.
    SubmittedTruncated { pages: usize },
    // The user cancelled the print dialog (or the app/parent cancelled it).
    Cancelled,
    Unavailable,
    UnsupportedCharacters,
    Failed,
}

impl PrintStatus {
    // Success, warning and cancellation toasts dismiss themselves; failures
    // stay until the user dismisses them.
    #[must_use]
    pub(crate) const fn expires(&self) -> bool {
        matches!(
            self,
            Self::Submitted | Self::SubmittedTruncated { .. } | Self::Cancelled
        )
    }
}

#[must_use]
pub(crate) fn outcome_status(outcome: &PortalOutcome<PrintReceipt>) -> PrintStatus {
    match outcome {
        PortalOutcome::Completed(_) => PrintStatus::Submitted,
        PortalOutcome::Cancelled(_) => PrintStatus::Cancelled,
        PortalOutcome::Denied => PrintStatus::Failed,
        PortalOutcome::NotDeclared => PrintStatus::Unavailable,
        PortalOutcome::Unsupported => PrintStatus::Unavailable,
        PortalOutcome::Unavailable => PrintStatus::Unavailable,
        PortalOutcome::ParentUnavailable => PrintStatus::Failed,
        PortalOutcome::Failed => PrintStatus::Failed,
    }
}

impl PrintStatus {
    #[must_use]
    pub(crate) fn banner(&self) -> String {
        match self {
            PrintStatus::Submitted => {
                enzui_core::localization::text("text-sent-to-the-printer-b35f62")
            }
            PrintStatus::SubmittedTruncated { pages } => {
                enzui_core::localized!("text-sent-the-first-pages-pages-to-the-4064f3", "pages" => pages)
            }
            PrintStatus::Cancelled => {
                enzui_core::localization::text("text-printing-was-cancelled-c107b0")
            }
            PrintStatus::Unavailable => enzui_core::localization::text(
                "text-printing-is-unavailable-no-print-service-answered-004122",
            ),
            PrintStatus::UnsupportedCharacters => {
                enzui_core::localization::text("text-20f01746dc32")
            }
            PrintStatus::Failed => enzui_core::localization::text(
                "text-printing-failed-nothing-was-sent-to-the-18a5b6",
            ),
        }
    }
}

// A rendered, sealed print document handed from the off-thread render back to
// the reducer.
#[derive(Debug)]
pub(crate) struct PreparedDocument {
    // The owned, sealed, seekable descriptor positioned at byte zero.
    pub document: OwnedFd,
    pub pages: usize,
    // Whether the document was truncated to a bound.
    pub truncated: bool,
    pub title: String,
}

// Cloned messages share one sealed descriptor; exactly one consumer may take it.
#[derive(Debug, Clone)]
pub(crate) struct PreparedHandoff(std::sync::Arc<std::sync::Mutex<Option<PreparedDocument>>>);

impl PreparedHandoff {
    pub(crate) fn new(document: PreparedDocument) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(Some(document))))
    }

    #[must_use]
    pub(crate) fn take(&self) -> Option<PreparedDocument> {
        self.0.lock().ok().and_then(|mut slot| slot.take())
    }
}

// A typed reason the off-thread render could not produce a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PrintRenderError {
    UnsupportedCharacters,
    // The owned descriptor could not be created or sealed.
    Descriptor(String),
}

// Render `text` to bounded PostScript and place it in an owned sealed
// descriptor. Runs off the reducer; the empty document renders one blank page.
//
// # Errors
//
// Returns [`PrintRenderError::Descriptor`] if the sealed memfd cannot be built.
pub(crate) fn prepare_document(
    text: &str,
    title: &str,
) -> Result<PreparedHandoff, PrintRenderError> {
    if text.chars().any(|character| u32::from(character) > 255) {
        return Err(PrintRenderError::UnsupportedCharacters);
    }
    let rendered = render_postscript(text, title);
    let document = sealed_document(&rendered.bytes)
        .map_err(|e| PrintRenderError::Descriptor(e.to_string()))?;
    Ok(PreparedHandoff::new(PreparedDocument {
        document,
        pages: rendered.pages,
        truncated: rendered.truncated,
        title: title.to_owned(),
    }))
}

#[derive(Debug, Clone)]
pub(crate) struct PrintJob {
    // This job's monotonic id; a prepared document at a different generation
    // is stale and refused.
    pub generation: u64,
    // Whether the spool has accepted a live request for this job.
    pub live: bool,
    pub pages: usize,
    // Whether the prepared document hit the page/byte bound. A completed
    // truncated job settles as [`PrintStatus::SubmittedTruncated`], never as a
    // plain full submission.
    pub truncated: bool,
}

// App-side print controller: the spool seam plus the single tracked job and
// the last visible status.
pub(crate) struct PrintState {
    spool: Box<dyn PrintSpool>,
    job: Option<PrintJob>,
    status: Option<PrintStatus>,
    generation: u64,
    parent: Option<crate::portal_parent::ParentLease>,
}

impl std::fmt::Debug for PrintState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrintState")
            .field("job", &self.job)
            .field("status", &self.status)
            .field("generation", &self.generation)
            .field("live", &self.spool.is_live())
            .finish()
    }
}

impl Default for PrintState {
    fn default() -> Self {
        Self::with_spool(default_spool())
    }
}

fn default_spool() -> Box<dyn PrintSpool> {
    #[cfg(feature = "portal-dbus")]
    {
        Box::new(DbusPrintSpool::default())
    }
    #[cfg(not(feature = "portal-dbus"))]
    {
        Box::new(UnavailableSpool)
    }
}

// A spool that reports the portal as unavailable — the honest state when
// `portal-dbus` is not compiled in.
#[cfg(not(feature = "portal-dbus"))]
struct UnavailableSpool;

#[cfg(not(feature = "portal-dbus"))]
impl PrintSpool for UnavailableSpool {
    fn start(&mut self, _request: PrintRequest) -> Result<(), PortalOutcome<PrintReceipt>> {
        Err(PortalOutcome::Unavailable)
    }
    fn poll(&mut self) -> Option<PortalOutcome<PrintReceipt>> {
        None
    }
    fn cancel(&mut self, _reason: CancelReason) {}
    fn settle(&mut self) {}
    fn is_live(&self) -> bool {
        false
    }
}

impl PrintState {
    // Build a controller over a specific spool (tests inject a fixture).
    #[must_use]
    pub(crate) fn with_spool(spool: Box<dyn PrintSpool>) -> Self {
        Self {
            spool,
            job: None,
            status: None,
            generation: 0,
            parent: None,
        }
    }

    #[must_use]
    pub(crate) fn status(&self) -> Option<&PrintStatus> {
        self.status.as_ref()
    }

    // Identifies the shown outcome, so its dismissal timer restarts when a
    // later job or a different outcome replaces it.
    #[must_use]
    pub(crate) fn status_key(&self) -> Option<(u64, std::mem::Discriminant<PrintStatus>)> {
        self.status
            .as_ref()
            .map(|status| (self.generation, std::mem::discriminant(status)))
    }

    // Whether a print is in progress (a job is tracked or the spool is live).
    #[must_use]
    pub(crate) fn is_busy(&self) -> bool {
        self.job.is_some() || self.spool.is_live()
    }

    // Whether the user has admitted a live print, so a window close must
    // settle it. False when nothing was ever started.
    #[must_use]
    pub(crate) fn has_admitted_inflight(&self) -> bool {
        self.job.as_ref().is_some_and(|j| j.live) || self.spool.is_live()
    }

    // An admitted spool cannot be superseded; generation exhaustion leaves the job intact.
    pub(crate) fn begin(&mut self) -> Option<u64> {
        if self.has_admitted_inflight() {
            return None;
        }
        let generation = self.generation.checked_add(1)?;
        self.generation = generation;
        self.status = None;
        self.job = Some(PrintJob {
            generation,
            live: false,
            pages: 0,
            truncated: false,
        });
        Some(generation)
    }

    // The current job's generation, if a job is tracked.
    #[must_use]
    pub(crate) fn current_generation(&self) -> Option<u64> {
        self.job.as_ref().map(|j| j.generation)
    }

    pub(crate) fn submit_parented(
        &mut self,
        generation: u64,
        document: PreparedDocument,
        parent: crate::portal_parent::ParentLease,
    ) {
        if self.current_generation() != Some(generation) || self.has_admitted_inflight() {
            return;
        }
        if !parent.is_alive() {
            self.render_failed(generation, None);
            return;
        }
        let request = build_request(document.document, &document.title, parent.handle());
        match self.spool.start(request) {
            Ok(()) => {
                let job = self.job.as_mut().expect("current print job");
                job.live = true;
                job.pages = document.pages;
                job.truncated = document.truncated;
                self.parent = Some(parent);
            }
            Err(outcome) => {
                self.status = Some(outcome_status(&outcome));
                self.job = None;
            }
        }
    }

    pub(crate) fn poll(&mut self) {
        if self
            .parent
            .as_ref()
            .is_some_and(|parent| !parent.is_alive())
        {
            self.abort_for_close();
            self.status = Some(PrintStatus::Cancelled);
            return;
        }
        if !self.spool.is_live() {
            return;
        }
        if let Some(outcome) = self.spool.poll() {
            self.spool.settle();
            let status = match &outcome {
                PortalOutcome::Completed(_) if self.job.as_ref().is_some_and(|j| j.truncated) => {
                    let pages = self.job.as_ref().map_or(0, |j| j.pages);
                    PrintStatus::SubmittedTruncated { pages }
                }
                other => outcome_status(other),
            };
            self.status = Some(status);
            self.job = None;
            self.parent = None;
        }
    }

    pub(crate) fn abort_for_close(&mut self) {
        self.abort(CancelReason::ParentClosed);
    }

    fn abort(&mut self, reason: CancelReason) {
        if self.has_admitted_inflight() {
            self.spool.cancel(reason);
            self.spool.settle();
        }
        self.job = None;
        self.parent = None;
    }

    pub(crate) fn render_failed(&mut self, generation: u64, error: Option<&PrintRenderError>) {
        if self.current_generation() == Some(generation) && !self.has_admitted_inflight() {
            self.status = Some(
                if matches!(error, Some(PrintRenderError::UnsupportedCharacters)) {
                    PrintStatus::UnsupportedCharacters
                } else {
                    PrintStatus::Failed
                },
            );
            self.job = None;
        }
    }

    pub(crate) fn dismiss_status(&mut self) {
        self.status = None;
    }
}

#[must_use]
fn build_request(
    document: OwnedFd,
    title: &str,
    parent: enzui_core::portal::parent::ParentHandle,
) -> PrintRequest {
    PrintRequest::with_dialog(document, PrintFormat::PostScript)
        .title(title.to_owned())
        .parent(parent)
}

impl Drop for PrintState {
    fn drop(&mut self) {
        self.abort(CancelReason::Application);
    }
}
