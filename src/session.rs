//! Versioned persistence for the editor's global tab set and workspace roots.
//!
//! The restore worker bounds raw bytes and decoded paths before retaining them,
//! rejects unstable or replaced inputs as a whole, and acquires the exact
//! existing-or-absent publication authority before returning. The reducer only
//! installs a current worker result; refused and superseded inputs remain
//! write-blocked. Publication uses the authority's conditional replacement and
//! rotating inode custody rather than reopening the destination pathname.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use enzui_core::quarantine::ExpectedSourceIdentity;
use enzui_core::state_authority::{PublishCustody, StateAuthority};

use crate::model::Model;
use crate::workspace::{
    ActiveWorkspace, PersistedTab, PersistedTabs, PersistedWorkspace, TreeRow, WorkspaceState,
};

#[path = "session/bounded_de.rs"]
mod bounded_de;
#[path = "session/bounded_load.rs"]
mod bounded_load;
#[path = "session/restore_worker.rs"]
mod restore_worker;

#[cfg(test)]
use bounded_load::load_with_limits;
pub(crate) use bounded_load::preflight_bounds;
use bounded_load::{account_path, enforce_snapshot_bounds, parse_snapshot, summarize_over_bound};
pub(crate) use restore_worker::{
    PublishOverBoundSummary, SessionDebt, SessionDebtKind, SessionRequest,
};

const FORMAT_VERSION: u32 = 3;

// Maximum session-file size admitted before any allocation, checked against
// opened-descriptor metadata and rechecked while reading. A larger file is one
// whole refusal, never a truncated restore.
pub(crate) const MAX_SESSION_FILE_BYTES: u64 = 1 << 20;

// Maximum retained tabs, whether legacy one-path-per-line entries or JSON tabs.
pub(crate) const MAX_SESSION_TABS: usize = 4096;

// Maximum retained named workspaces.
pub(crate) const MAX_SESSION_WORKSPACES: usize = 512;

// Maximum decoded bytes of any single retained path.
pub(crate) const MAX_SESSION_PATH_BYTES: usize = 4096;

// Maximum aggregate decoded path bytes across every retained tab and workspace.
pub(crate) const MAX_SESSION_AGGREGATE_PATH_BYTES: u64 = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SessionLimits {
    max_file_bytes: u64,
    max_tabs: usize,
    max_workspaces: usize,
    max_path_bytes: usize,
    max_aggregate_path_bytes: u64,
}

impl SessionLimits {
    const PRODUCTION: Self = Self {
        max_file_bytes: MAX_SESSION_FILE_BYTES,
        max_tabs: MAX_SESSION_TABS,
        max_workspaces: MAX_SESSION_WORKSPACES,
        max_path_bytes: MAX_SESSION_PATH_BYTES,
        max_aggregate_path_bytes: MAX_SESSION_AGGREGATE_PATH_BYTES,
    };

    #[cfg(test)]
    const fn new(
        max_file_bytes: u64,
        max_tabs: usize,
        max_workspaces: usize,
        max_path_bytes: usize,
        max_aggregate_path_bytes: u64,
    ) -> Self {
        Self {
            max_file_bytes,
            max_tabs,
            max_workspaces,
            max_path_bytes,
            max_aggregate_path_bytes,
        }
    }
}

// Why a session source was refused, carrying no path or parser payload so a
// disclosure can name the situation without leaking file contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum SessionRefusal {
    NotRegularFile,
    // The open itself was refused, including a final-component symlink.
    OpenRefused,
    // The file exceeds the byte ceiling by metadata or by the bounded read.
    TooLarge,
    TooManyEntries,
    // A path exceeds the per-path or aggregate decoded-byte ceiling.
    PathBudget,
    Malformed,
    // Reading the opened descriptor failed, including non-UTF-8 content.
    ReadFailed,
    // The completed read did not match the byte length measured on the same
    // descriptor before reading, so the source changed during the read.
    Unstable,
    // The opened inode no longer matched the path after the read, so the source
    // was replaced (or became a symlink or non-regular file) during load.
    Replaced,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionSnapshot {
    version: u32,
    #[serde(default)]
    tabs: PersistedTabs,
    #[serde(default)]
    workspaces: Vec<PersistedWorkspace>,
    #[serde(skip)]
    restore_guard: RestoreGuard,
}

#[derive(Debug, Deserialize)]
struct VersionProbe {
    version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RestoreGuard {
    active_tab_id: String,
    tabs: Vec<(String, u64, bool, bool, bool)>,
    active_workspace: ActiveWorkspace,
    selected_workspace_key: String,
    workspace_rows: Vec<TreeRow>,
}

impl Default for RestoreGuard {
    fn default() -> Self {
        Self {
            active_tab_id: String::new(),
            tabs: Vec::new(),
            active_workspace: ActiveWorkspace::OpenFiles,
            selected_workspace_key: String::new(),
            workspace_rows: Vec::new(),
        }
    }
}

impl SessionSnapshot {
    fn empty() -> Self {
        Self {
            version: FORMAT_VERSION,
            tabs: PersistedTabs::default(),
            workspaces: Vec::new(),
            restore_guard: RestoreGuard::default(),
        }
    }

    pub(crate) fn empty_for_app() -> Self {
        Self::empty()
    }

    pub(crate) fn tabs(&self) -> &PersistedTabs {
        &self.tabs
    }

    pub(crate) fn workspaces(&self) -> &[PersistedWorkspace] {
        &self.workspaces
    }

    fn from_state(model: &Model, workspaces: &WorkspaceState) -> Self {
        Self {
            version: FORMAT_VERSION,
            tabs: PersistedTabs::from_model(model),
            workspaces: workspaces.persisted_workspaces(),
            restore_guard: RestoreGuard::default(),
        }
    }
}

pub(crate) fn persisted_snapshot(model: &Model, workspaces: &WorkspaceState) -> SessionSnapshot {
    #[cfg(test)]
    PERSISTED_SNAPSHOT_CALLS.with(|c| c.set(c.get().wrapping_add(1)));
    SessionSnapshot::from_state(model, workspaces)
}

// Per-thread call counter on the `persisted_snapshot` constructor. Reducer
// callers run on the test thread; blocking-task workers never re-enter this
// constructor.
#[cfg(test)]
thread_local! {
    pub(crate) static PERSISTED_SNAPSHOT_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub(crate) fn projection(model: &Model, workspaces: &WorkspaceState) -> SessionSnapshot {
    let mut snapshot = persisted_snapshot(model, workspaces);
    snapshot.restore_guard = RestoreGuard {
        active_tab_id: model.active_tab_id.clone(),
        tabs: model
            .tabs
            .iter()
            .map(|tab| {
                (
                    tab.id.clone(),
                    tab.snapshot_version(),
                    tab.dirty,
                    tab.loading,
                    tab.document().is_empty(),
                )
            })
            .collect(),
        active_workspace: workspaces.active.clone(),
        selected_workspace_key: workspaces.selected_key.clone(),
        workspace_rows: workspaces.rows().as_ref().to_vec(),
    };
    snapshot
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Missing,
    Restored,
    // The source was refused; it is preserved in place and the store is
    // write-disabled. The reason carries no path or parser payload.
    Refused(SessionRefusal),
    ReadOnlyFutureVersion(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
}

impl FileIdentity {
    fn of(meta: &std::fs::Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileAdmission {
    identity: FileIdentity,
    len: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    links: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl FileAdmission {
    fn of(meta: &std::fs::Metadata) -> Self {
        Self {
            identity: FileIdentity::of(meta),
            len: meta.len(),
            mode: meta.mode(),
            uid: meta.uid(),
            gid: meta.gid(),
            links: meta.nlink(),
            modified_seconds: meta.mtime(),
            modified_nanoseconds: meta.mtime_nsec(),
            changed_seconds: meta.ctime(),
            changed_nanoseconds: meta.ctime_nsec(),
        }
    }

    fn expected_identity(self) -> ExpectedSourceIdentity {
        ExpectedSourceIdentity::from_dev_ino(self.identity.dev, self.identity.ino)
    }
}

fn path_still_matches(path: &Path, admission: FileAdmission) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type().is_file() && FileAdmission::of(&meta) == admission,
        Err(_) => false,
    }
}

struct RestorePayload {
    request: SessionRequest,
    snapshot: SessionSnapshot,
    origin: Origin,
    authority: Option<StateAuthority>,
    admission: Option<FileAdmission>,
    expected: Option<ExpectedSourceIdentity>,
    expected_absent: bool,
    debt_kind: Option<SessionDebtKind>,
}

// Single-consumption result of a bounded restore worker.
//
// Clones share one payload because the retained publication authority is not
// clonable. Only the reducer generation that accepts the result can take it;
// dropping a stale completion drops its authority without installing it.
#[derive(Clone)]
pub(crate) struct SessionRestore {
    payload: Arc<Mutex<Option<RestorePayload>>>,
}

impl std::fmt::Debug for SessionRestore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRestore").finish_non_exhaustive()
    }
}

impl SessionRestore {
    fn new(payload: RestorePayload) -> Self {
        Self {
            payload: Arc::new(Mutex::new(Some(payload))),
        }
    }

    fn take(self) -> Option<RestorePayload> {
        self.payload
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteOutcome {
    Persisted,
    Unchanged,
    DurabilityUncertain,
    Lost,
}

fn classify_publish(result: Result<(), PublishCustody>) -> WriteOutcome {
    match result {
        Ok(()) => WriteOutcome::Persisted,
        Err(PublishCustody::Unchanged) => WriteOutcome::Unchanged,
        Err(PublishCustody::Rotated) => WriteOutcome::DurabilityUncertain,
        Err(PublishCustody::Lost) => WriteOutcome::Lost,
        Err(_) => WriteOutcome::Unchanged,
    }
}

#[derive(Debug)]
pub(crate) struct SessionStore {
    last: SessionSnapshot,
    enabled: bool,
    write_blocked: bool,
    authority: Option<StateAuthority>,
    request: Option<SessionRequest>,
    debt: Option<SessionDebt>,
    pending_retry: Option<SessionSnapshot>,
}

impl SessionStore {
    #[cfg(test)]
    pub(crate) fn open_with_limits(
        path: PathBuf,
        home: Option<&Path>,
        limits: SessionLimits,
    ) -> (SessionSnapshot, Self, Origin) {
        Self::open_with(path, home, limits)
    }

    // Installs only the authority and metadata already acquired by the worker.
    // No filesystem operation occurs on this path.
    pub(crate) fn install(restore: SessionRestore) -> (SessionSnapshot, Self, Origin) {
        let Some(payload) = restore.take() else {
            let snapshot = SessionSnapshot::empty();
            let store = Self::disabled(snapshot.clone(), None, None);
            return (
                snapshot,
                store,
                Origin::Refused(SessionRefusal::OpenRefused),
            );
        };
        let RestorePayload {
            request,
            snapshot,
            origin,
            authority,
            admission,
            expected,
            expected_absent,
            debt_kind,
        } = payload;
        let expected = expected.or_else(|| admission.map(FileAdmission::expected_identity));
        if matches!(&origin, Origin::Restored | Origin::Missing)
            && let Some(authority) = authority
        {
            let store = Self {
                last: snapshot.clone(),
                enabled: true,
                write_blocked: false,
                authority: Some(authority),
                request: Some(request),
                debt: None,
                pending_retry: None,
            };
            return (snapshot, store, origin);
        }
        let kind = debt_kind.unwrap_or(match &origin {
            Origin::Refused(SessionRefusal::Replaced) => SessionDebtKind::Replaced,
            Origin::Refused(refusal) => SessionDebtKind::Refused(*refusal),
            Origin::ReadOnlyFutureVersion(_) => SessionDebtKind::FutureVersion,
            Origin::Missing | Origin::Restored => SessionDebtKind::Replaced,
        });
        let empty = SessionSnapshot::empty();
        let debt = SessionDebt::new(request.clone(), kind, expected, expected_absent);
        (
            empty.clone(),
            Self::disabled(empty, Some(request), Some(debt)),
            origin,
        )
    }

    fn disabled(
        last: SessionSnapshot,
        request: Option<SessionRequest>,
        debt: Option<SessionDebt>,
    ) -> Self {
        Self {
            last,
            enabled: false,
            write_blocked: true,
            authority: None,
            request,
            debt,
            pending_retry: None,
        }
    }

    pub(crate) fn take_debt(&mut self) -> Option<SessionDebt> {
        self.debt.take()
    }

    pub(crate) fn superseded_debt(&self) -> Option<SessionDebt> {
        self.request.clone().map(SessionDebt::superseded_request)
    }

    pub(crate) fn revoke_write_authority(&mut self) {
        self.enabled = false;
        self.write_blocked = true;
        self.authority = None;
        self.debt = None;
        self.pending_retry = None;
    }

    #[cfg(test)]
    pub(crate) fn inert() -> Self {
        Self::disabled(SessionSnapshot::empty(), None, None)
    }

    // A live but write-refusing store with an empty snapshot, for a boot with
    // no safe session path, so boot never falls back to a predictable `/tmp`
    // file.
    pub(crate) fn open_disabled() -> (SessionSnapshot, Self, Origin) {
        let snapshot = SessionSnapshot::empty();
        (
            snapshot.clone(),
            Self::disabled(snapshot, None, None),
            Origin::Missing,
        )
    }

    pub(crate) fn discard_open_tabs(&mut self) -> bool {
        if !self.enabled || self.write_blocked {
            return false;
        }
        if self.pending_retry.is_some() {
            return false;
        }
        let empty = SessionSnapshot::empty();
        if empty == self.last {
            self.enabled = false;
            return true;
        }
        match self.publish(empty) {
            WriteOutcome::Persisted => {
                self.enabled = false;
                true
            }
            WriteOutcome::Unchanged | WriteOutcome::DurabilityUncertain | WriteOutcome::Lost => {
                false
            }
        }
    }

    pub(crate) fn sync_snapshot(&mut self, next: SessionSnapshot) -> bool {
        if !self.enabled {
            return false;
        }
        // Lost or uncertain authority is blocked by `!enabled` or
        // `pending_retry`; an over-bound refusal sets neither, so a fresh
        // projection may proceed and `publish` re-checks bounds.
        if self.pending_retry.is_some() || next == self.last {
            return false;
        }
        matches!(self.publish(next), WriteOutcome::Persisted)
    }

    // Retry only the exact projection retained after an unsettled publication.
    pub(crate) fn retry_pending(&mut self) -> bool {
        let Some(next) = self.pending_retry.clone() else {
            return false;
        };
        matches!(self.publish(next), WriteOutcome::Persisted)
    }

    // Only a durable publication advances `last`; a retryable one keeps the
    // authority and republishes the same projection; lost custody disables the
    // store. Bounds are enforced here so every route, including the direct
    // `SessionStore::sync`, is admitted the same way.
    fn publish(&mut self, next: SessionSnapshot) -> WriteOutcome {
        if let Err(refusal) = enforce_snapshot_bounds(&next, SessionLimits::PRODUCTION) {
            let summary = summarize_over_bound(&next, refusal);
            if let Some(request) = self.request.clone() {
                let expected = self
                    .authority
                    .as_ref()
                    .and_then(StateAuthority::identity)
                    .map(|identity| {
                        ExpectedSourceIdentity::from_dev_ino(identity.device(), identity.inode())
                    });
                self.debt = Some(SessionDebt::new(
                    request,
                    SessionDebtKind::PublishOverBound(summary),
                    expected,
                    expected.is_none(),
                ));
            }
            // Unlike the authority-loss paths, a fresh bounded projection may
            // still recover through `sync_snapshot`.
            self.pending_retry = None;
            return WriteOutcome::Unchanged;
        }
        let Some(authority) = self.authority.as_mut() else {
            self.enabled = false;
            self.write_blocked = true;
            return WriteOutcome::Lost;
        };
        let outcome = classify_publish(
            authority
                .publish_with(|output| write_snapshot_bounded(output, &next))
                .map(|_| ())
                .map_err(|error| error.custody()),
        );
        match outcome {
            WriteOutcome::Persisted => {
                self.last = next;
                self.debt = None;
                self.pending_retry = None;
                self.write_blocked = false;
            }
            WriteOutcome::Unchanged => {
                if let Some(request) = self.request.clone() {
                    let expected = authority.identity().map(|identity| {
                        ExpectedSourceIdentity::from_dev_ino(identity.device(), identity.inode())
                    });
                    self.debt = Some(SessionDebt::new(
                        request,
                        SessionDebtKind::Replaced,
                        expected,
                        expected.is_none(),
                    ));
                }
                self.pending_retry = Some(next);
                self.write_blocked = true;
            }
            WriteOutcome::DurabilityUncertain => {
                if let Some(request) = self.request.clone() {
                    let expected = authority.identity().map(|identity| {
                        ExpectedSourceIdentity::from_dev_ino(identity.device(), identity.inode())
                    });
                    self.debt = Some(SessionDebt::new(
                        request,
                        SessionDebtKind::DurabilityUncertain,
                        expected,
                        false,
                    ));
                }
                self.pending_retry = Some(next);
                self.write_blocked = true;
            }
            WriteOutcome::Lost => {
                if let Some(request) = self.request.clone() {
                    self.debt = Some(SessionDebt::new(
                        request,
                        SessionDebtKind::CustodyLost,
                        None,
                        false,
                    ));
                }
                self.authority = None;
                self.pending_retry = None;
                self.enabled = false;
                self.write_blocked = true;
            }
        }
        outcome
    }
}

struct BoundedWriter<'a> {
    output: &'a mut dyn Write,
    remaining: usize,
}

impl Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "session snapshot exceeds its byte limit",
            ));
        }
        let written = self.output.write(bytes)?;
        self.remaining = self.remaining.saturating_sub(written);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

fn write_snapshot_bounded(output: &mut dyn Write, snapshot: &SessionSnapshot) -> io::Result<()> {
    let remaining = usize::try_from(MAX_SESSION_FILE_BYTES).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "session byte limit is not representable",
        )
    })?;
    let mut bounded = BoundedWriter { output, remaining };
    serde_json::to_writer_pretty(&mut bounded, snapshot).map_err(io::Error::other)
}

#[derive(Debug)]
enum LoadError {
    #[cfg(test)]
    Missing,
    Refused(SessionRefusal),
    Future(u32),
}

enum OpenRefusal {
    Missing,
    NotRegular,
    Denied,
}

// Open read-only without following a final symlink and without blocking, then
// require a regular file: a read-only open of a writerless FIFO would block.
fn open_regular_nofollow(path: &Path) -> Result<File, OpenRefusal> {
    use rustix::fs::{Mode, OFlags};
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| {
        if std::io::Error::from(e).kind() == std::io::ErrorKind::NotFound {
            OpenRefusal::Missing
        } else {
            OpenRefusal::Denied
        }
    })?;
    let file = File::from(fd);
    match file.metadata() {
        Ok(m) if m.is_file() => Ok(file),
        Ok(_) => Err(OpenRefusal::NotRegular),
        Err(_) => Err(OpenRefusal::Denied),
    }
}

// Cap bytes against opened-descriptor metadata before allocating, then read one
// byte past the cap. The completed read must equal the length measured on the
// same descriptor, so a same-inode append or truncation during the read is
// refused (`Unstable`), not only growth past the cap (`TooLarge`). Returns the
// opened inode identity so a later publication can bind to it.

// Synchronization seams used only by tests to inject a concurrent substitution
// or growth at a precise point; inert in production.
#[cfg(test)]
fn seam_after_metadata() {
    test_seam::run_after_metadata();
}
#[cfg(not(test))]
fn seam_after_metadata() {}

#[cfg(test)]
fn seam_after_read() {
    test_seam::run_after_read();
}
#[cfg(not(test))]
fn seam_after_read() {}

#[cfg(test)]
pub(crate) fn set_restore_after_read_seam(f: impl Fn() + 'static) -> test_seam::Reset {
    test_seam::set_after_read(f);
    test_seam::Reset
}

// Resolve legacy one-path-per-line entries, refusing the whole file rather than
// retaining a prefix once any per-path, aggregate, or entry cap is crossed.
fn parse_legacy_bounded(
    text: &str,
    home: Option<&Path>,
    limits: SessionLimits,
) -> Result<Vec<PathBuf>, SessionRefusal> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut aggregate: u64 = 0;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.len() > limits.max_path_bytes {
            return Err(SessionRefusal::PathBudget);
        }
        let candidate = PathBuf::from(line);
        let resolved = if candidate.is_absolute() {
            candidate
        } else if let Some(h) = home {
            h.join(&candidate)
        } else {
            // No HOME to anchor a relative path: skip rather than fall back to
            // the process CWD.
            continue;
        };
        aggregate = account_path(&resolved, aggregate, limits)?;
        if out.len() >= limits.max_tabs {
            return Err(SessionRefusal::TooManyEntries);
        }
        out.push(resolved);
    }
    Ok(out)
}

// One-shot, thread-local hooks that let a test substitute or grow the source at a
// precise point during a single bounded load. Each hook runs at most once; a
// `Reset` guard clears both when a test returns.
#[cfg(test)]
mod test_seam {
    use std::cell::RefCell;

    type Hook = Box<dyn Fn()>;

    thread_local! {
        static AFTER_METADATA: RefCell<Option<Hook>> = const { RefCell::new(None) };
        static AFTER_READ: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub(super) fn set_after_metadata(f: impl Fn() + 'static) {
        AFTER_METADATA.with(|h| *h.borrow_mut() = Some(Box::new(f)));
    }

    pub(super) fn set_after_read(f: impl Fn() + 'static) {
        AFTER_READ.with(|h| *h.borrow_mut() = Some(Box::new(f)));
    }

    pub(super) fn run_after_metadata() {
        if let Some(f) = AFTER_METADATA.with(|h| h.borrow_mut().take()) {
            f();
        }
    }

    pub(super) fn run_after_read() {
        if let Some(f) = AFTER_READ.with(|h| h.borrow_mut().take()) {
            f();
        }
    }

    pub(crate) struct Reset;

    impl Drop for Reset {
        fn drop(&mut self) {
            AFTER_METADATA.with(|h| *h.borrow_mut() = None);
            AFTER_READ.with(|h| *h.borrow_mut() = None);
        }
    }
}

#[cfg(test)]
#[path = "session/migration_and_save_tests.rs"]
mod migration_and_save_tests;

#[cfg(test)]
#[path = "session/bounded_restore_tests.rs"]
mod bounded_restore_tests;

#[cfg(test)]
fn read_bounded(path: &Path, max_bytes: u64) -> Result<(String, FileIdentity), LoadError> {
    let file = match open_regular_nofollow(path) {
        Ok(f) => f,
        Err(OpenRefusal::Missing) => return Err(LoadError::Missing),
        Err(OpenRefusal::NotRegular) => {
            return Err(LoadError::Refused(SessionRefusal::NotRegularFile));
        }
        Err(OpenRefusal::Denied) => return Err(LoadError::Refused(SessionRefusal::OpenRefused)),
    };
    let (stat_len, identity) = match file.metadata() {
        Ok(m) if m.is_file() => (m.len(), FileIdentity::of(&m)),
        Ok(_) => return Err(LoadError::Refused(SessionRefusal::NotRegularFile)),
        Err(_) => return Err(LoadError::Refused(SessionRefusal::ReadFailed)),
    };
    if stat_len > max_bytes {
        return Err(LoadError::Refused(SessionRefusal::TooLarge));
    }
    seam_after_metadata();
    let mut text = String::new();
    let mut bounded = (&file).take(max_bytes.saturating_add(1));
    bounded
        .read_to_string(&mut text)
        .map_err(|_| LoadError::Refused(SessionRefusal::ReadFailed))?;
    if text.len() as u64 > max_bytes {
        return Err(LoadError::Refused(SessionRefusal::TooLarge));
    }
    // A length change under the open is a same-inode append or truncation;
    // refuse rather than grant write custody over a moving target.
    if text.len() as u64 != stat_len {
        return Err(LoadError::Refused(SessionRefusal::Unstable));
    }
    Ok((text, identity))
}

#[cfg(test)]
impl SessionStore {
    pub(crate) fn read(path: PathBuf, home: Option<&Path>) -> SessionRestore {
        SessionRequest::for_path(path, home).execute()
    }

    fn open_with(
        path: PathBuf,
        home: Option<&Path>,
        limits: SessionLimits,
    ) -> (SessionSnapshot, Self, Origin) {
        Self::install(
            SessionRequest::for_path(path, home)
                .with_limits(limits)
                .execute(),
        )
    }

    pub(crate) fn open(path: PathBuf, home: Option<&Path>) -> (SessionSnapshot, Self, Origin) {
        Self::open_with(path, home, SessionLimits::PRODUCTION)
    }
}

#[cfg(test)]
fn path_still_names(path: &Path, identity: FileIdentity) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type().is_file() && FileIdentity::of(&meta) == identity,
        Err(_) => false,
    }
}
