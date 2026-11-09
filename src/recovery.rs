//! Crash recovery for unsaved editable buffers.
//!
//! Reads and conditional writes retain inode-bound authority; stale generations
//! and external replacements cannot overwrite the journal. Malformed or oversized
//! records are quarantined within the limits declared below. Future schemas remain
//! untouched. Disk fingerprint conflicts require an explicit user choice before
//! writing the original path.

use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use enzui_core::file_watch::FileIdentity;
use enzui_core::quarantine::{
    CustodyIdentity, ExpectedSourceIdentity, QuarantineOutcome, quarantine_sibling_expected,
};
use enzui_core::state_authority::{CustodySnapshot, PublishCustody, StateAuthority};

// On-disk schema version.
pub(crate) const SCHEMA_VERSION: u32 = 1;

pub(crate) const MAX_RECOVERY_CONTENT_BYTES: usize = 8 * 1024 * 1024;

// Largest number of records retained (newest by `updated_at_ms` kept).
pub(crate) const MAX_RECOVERY_RECORDS: usize = 64;

// Largest complete recovery journal admitted or published.
pub(crate) const MAX_RECOVERY_FILE_BYTES: usize = 64 * 1024 * 1024;

const MAX_DOCUMENT_ID_BYTES: usize = 128;
const MAX_RECOVERY_PATH_BYTES: usize = 4096;
const MAX_METADATA_STRING_BYTES: usize = 128;

fn default_schema() -> u32 {
    SCHEMA_VERSION
}
fn utf8() -> String {
    "UTF-8".to_owned()
}
fn lf() -> String {
    "LF".to_owned()
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

// The last-known disk identity of a path-backed buffer — the serializable twin
// of [`FileIdentity`] (`(dev, ino) + mtime + len`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DiskFingerprint {
    #[serde(default)]
    pub dev: u64,
    #[serde(default)]
    pub ino: u64,
    #[serde(default)]
    pub mtime_secs: i64,
    #[serde(default)]
    pub mtime_nsec: u32,
    #[serde(default)]
    pub len: u64,
}

impl DiskFingerprint {
    #[must_use]
    pub(crate) fn from_identity(id: FileIdentity) -> Self {
        Self {
            dev: id.inode.0,
            ino: id.inode.1,
            mtime_secs: id.mtime_secs,
            mtime_nsec: id.mtime_nsec,
            len: id.len,
        }
    }

    #[must_use]
    pub(crate) fn same_disk_state(&self, other: &DiskFingerprint) -> bool {
        self == other
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct CaretMeta {
    #[serde(default)]
    pub cursor_line: usize,
    #[serde(default)]
    pub cursor_column: usize,
    #[serde(default)]
    pub selection_line: Option<usize>,
    #[serde(default)]
    pub selection_column: Option<usize>,
    #[serde(default)]
    pub scroll_offset: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct RecoveryRecord {
    #[serde(default = "default_schema")]
    pub schema: u32,
    // Stable identity of the document across launches (the tab id).
    #[serde(default)]
    pub document_id: String,
    #[serde(default)]
    pub original_path: Option<PathBuf>,
    // Last-known disk identity; `Some` means the file existed at snapshot time.
    #[serde(default)]
    pub fingerprint: Option<DiskFingerprint>,
    // Whether the buffer had never been saved to any path.
    #[serde(default)]
    pub was_untitled: bool,
    // Set when the buffer exceeded [`MAX_RECOVERY_CONTENT_BYTES`]; `content`
    // is then empty and the record is quarantined at load.
    #[serde(default)]
    pub oversized: bool,
    #[serde(default = "utf8")]
    pub encoding: String,
    #[serde(default = "lf")]
    pub line_ending: String,
    #[serde(default)]
    pub language_override: Option<String>,
    #[serde(default)]
    pub caret: CaretMeta,
    // Authoritative byte length of the buffer at snapshot time. A mismatch
    // with `content.len()` is tamper/truncation and quarantines the record.
    #[serde(default)]
    pub content_bytes: u64,
    // The bounded UTF-8 buffer contents (empty when `oversized`).
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub updated_at_ms: u64,
}

impl RecoveryRecord {
    // Worker constructor: `content` is the materialised bounded text (empty
    // when oversized) and `content_bytes` the length captured before
    // materialisation, so an oversized buffer is never cloned.
    #[must_use]
    pub(crate) fn from_content(
        document_id: impl Into<String>,
        content: String,
        content_bytes: u64,
    ) -> Self {
        let oversized =
            usize::try_from(content_bytes).map_or(true, |bytes| bytes > MAX_RECOVERY_CONTENT_BYTES);
        Self {
            schema: SCHEMA_VERSION,
            document_id: document_id.into(),
            original_path: None,
            fingerprint: None,
            was_untitled: true,
            oversized,
            encoding: utf8(),
            line_ending: lf(),
            language_override: None,
            caret: CaretMeta::default(),
            content_bytes,
            content: if oversized { String::new() } else { content },
            updated_at_ms: now_ms(),
        }
    }

    #[must_use]
    pub(crate) fn with_path(
        mut self,
        path: Option<PathBuf>,
        fingerprint: Option<DiskFingerprint>,
    ) -> Self {
        self.was_untitled = path.is_none();
        self.original_path = path;
        self.fingerprint = fingerprint;
        self
    }

    #[must_use]
    pub(crate) fn with_caret(mut self, caret: CaretMeta) -> Self {
        self.caret = caret;
        self
    }

    #[must_use]
    pub(crate) fn with_metadata(
        mut self,
        encoding: impl Into<String>,
        line_ending: impl Into<String>,
        language_override: Option<String>,
    ) -> Self {
        self.encoding = encoding.into();
        self.line_ending = line_ending.into();
        self.language_override = language_override;
        self
    }

    // A record is well-formed only when it is a known schema, not oversized,
    // and its stored bytes match the authoritative length. Anything else is
    // truncation or tampering and must not be offered as a recovery.
    #[must_use]
    pub(crate) fn is_well_formed(&self) -> bool {
        self.schema == SCHEMA_VERSION
            && !self.document_id.is_empty()
            && self.document_id.len() <= MAX_DOCUMENT_ID_BYTES
            && self.original_path.as_ref().is_none_or(|path| {
                !path.as_os_str().is_empty()
                    && path.as_os_str().as_bytes().len() <= MAX_RECOVERY_PATH_BYTES
            })
            && self.was_untitled == self.original_path.is_none()
            && (self.fingerprint.is_none() || self.original_path.is_some())
            && self.encoding == "UTF-8"
            && matches!(self.line_ending.as_str(), "LF" | "CRLF" | "CR" | "Mixed")
            && self
                .language_override
                .as_ref()
                .is_none_or(|value| value.len() <= MAX_METADATA_STRING_BYTES)
            && self.caret.scroll_offset.is_finite()
            && self.caret.scroll_offset >= 0.0
            && usize::try_from(self.content_bytes) == Ok(self.content.len())
            && self.content.len() <= MAX_RECOVERY_CONTENT_BYTES
    }
}

// The whole atomic recovery document.
//
// `generation` is stamped by [`RecoveryStore::publish`] and stops an older
// lease replacing a newer publication, such as a slow write resurrecting a
// Discard tombstone. It also seeds the next session's generation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct RecoverySnapshot {
    #[serde(default = "default_schema")]
    pub version: u32,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub records: Vec<RecoveryRecord>,
}

impl Default for RecoverySnapshot {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            generation: 0,
            records: Vec::new(),
        }
    }
}

impl RecoverySnapshot {
    #[must_use]
    pub(crate) fn from_records(mut records: Vec<RecoveryRecord>) -> Self {
        records.sort_by_key(|record| std::cmp::Reverse(record.updated_at_ms));
        records.truncate(MAX_RECOVERY_RECORDS);
        Self {
            version: SCHEMA_VERSION,
            generation: 0,
            records,
        }
    }

    #[must_use]
    pub(crate) fn partition_well_formed(self) -> (Vec<RecoveryRecord>, usize) {
        let total = self.records.len();
        let mut document_ids = std::collections::HashSet::new();
        let kept: Vec<RecoveryRecord> = self
            .records
            .into_iter()
            .take(MAX_RECOVERY_RECORDS)
            .filter(|record| {
                record.is_well_formed() && document_ids.insert(record.document_id.clone())
            })
            .collect();
        let dropped = total.saturating_sub(kept.len());
        (kept, dropped)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Origin {
    // No recovery file existed — a clean prior shutdown.
    Missing,
    Loaded { dropped: usize },
    // The corrupt file was moved into verified retained-source custody.
    RecoveredFromCorrupt,
    // The file is a newer schema; left byte-for-byte untouched, store disabled.
    ReadOnlyFutureVersion,
    // Quarantine lacked verified durable custody; source bytes remain retained
    // wherever the framework outcome proved them, and writing is disabled.
    QuarantineRefused,
    // No recovery file exists, but its private folder could not be opened or
    // secured, so writing is disabled. Nothing suspicious was found.
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RecoveryDecision {
    // Never path-backed — restore directly as a dirty untitled buffer.
    RestoreUntitled(RecoveryRecord),
    // Path-backed and the disk is provably the same (or was never known and
    // the file is absent) — restore without a conflict prompt.
    RestoreClean(RecoveryRecord),
    // Path-backed and the disk changed since the crash — the user must choose
    // recovered-vs-disk before that path is written.
    Conflict {
        record: RecoveryRecord,
        disk: DiskFingerprint,
    },
    // Path-backed, existed at snapshot time, and is now missing — the user
    // must explicitly Save (recreate) or Save As before that path is written.
    Deleted {
        record: RecoveryRecord,
    },
}

#[must_use]
pub(crate) fn classify(record: RecoveryRecord, disk: Option<DiskFingerprint>) -> RecoveryDecision {
    if record.was_untitled || record.original_path.is_none() {
        return RecoveryDecision::RestoreUntitled(record);
    }
    match (record.fingerprint, disk) {
        // Existed at snapshot time, gone now: a deletion, never a clean restore.
        (Some(_), None) => RecoveryDecision::Deleted { record },
        // Never knew the disk and it is absent: a new file at this path is safe.
        (None, None) => RecoveryDecision::RestoreClean(record),
        // Provably unchanged since the crash.
        (Some(known), Some(cur)) if known.same_disk_state(&cur) => {
            RecoveryDecision::RestoreClean(record)
        }
        // Changed, or no baseline while the file exists: the user must choose.
        (_, Some(cur)) => RecoveryDecision::Conflict { record, disk: cur },
    }
}

#[derive(Debug)]
enum LoadError {
    Missing,
    Corrupt(ExpectedSourceIdentity),
    Refused,
    Future,
}

#[derive(Deserialize)]
struct VersionProbe {
    #[serde(default = "default_schema")]
    version: u32,
}

fn load(
    path: &Path,
    authority: &StateAuthority,
    max_bytes: usize,
) -> Result<RecoverySnapshot, LoadError> {
    let read_limit = match authority.identity() {
        None => return Err(LoadError::Missing),
        Some(identity) => {
            let metadata = std::fs::symlink_metadata(path).map_err(|_| LoadError::Refused)?;
            if !metadata.file_type().is_file()
                || metadata.dev() != identity.device()
                || metadata.ino() != identity.inode()
            {
                return Err(LoadError::Refused);
            }
            let length = usize::try_from(metadata.len()).map_err(|_| LoadError::Refused)?;
            if length > max_bytes {
                return Err(LoadError::Corrupt(ExpectedSourceIdentity::from_dev_ino(
                    identity.device(),
                    identity.inode(),
                )));
            }
            length
        }
    };
    let snapshot = authority
        .read_bounded(read_limit)
        .map_err(|_| LoadError::Refused)?;
    let (bytes, expected) = match snapshot {
        CustodySnapshot::Absent => return Err(LoadError::Missing),
        CustodySnapshot::Current {
            bytes,
            overflow,
            identity,
        } => {
            let expected =
                ExpectedSourceIdentity::from_dev_ino(identity.device(), identity.inode());
            if overflow {
                return Err(LoadError::Corrupt(expected));
            }
            (bytes, expected)
        }
        CustodySnapshot::Lost => return Err(LoadError::Refused),
    };
    let text = String::from_utf8(bytes).map_err(|_| LoadError::Corrupt(expected))?;
    if text.trim().is_empty() {
        return Ok(RecoverySnapshot::default());
    }
    let probe: VersionProbe =
        serde_json::from_str(&text).map_err(|_| LoadError::Corrupt(expected))?;
    if probe.version > SCHEMA_VERSION {
        return Err(LoadError::Future);
    }
    serde_json::from_str(&text).map_err(|_| LoadError::Corrupt(expected))
}

fn quarantine(path: &Path, expected: ExpectedSourceIdentity) -> bool {
    match quarantine_sibling_expected(path, expected) {
        QuarantineOutcome::Moved { custody } => custody
            .paths()
            .iter()
            .any(|path| path.identity() == CustodyIdentity::RetainedSource),
        QuarantineOutcome::CollisionExhausted { .. }
        | QuarantineOutcome::SourceMismatch { .. }
        | QuarantineOutcome::SourceReplaced { .. }
        | QuarantineOutcome::DurabilityUncertain { .. }
        | QuarantineOutcome::PartialCustody { .. }
        | QuarantineOutcome::Refused { .. } => false,
        _ => false,
    }
}

// The app-owned recovery store and its shared publication authority.
//
// The gate serializes every writer while retaining the exact destination inode
// across publications. The desired record set remains app-owned and is passed
// to each publication.
#[derive(Debug)]
struct RecoveryAuthority {
    authority: Option<StateAuthority>,
    published_generation: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct RecoveryStore {
    enabled: Arc<AtomicBool>,
    gate: Arc<Mutex<RecoveryAuthority>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublishOutcome {
    // The snapshot was written and became the on-disk truth.
    Written,
    // A newer generation already published; this older write was refused.
    Superseded,
    Skipped,
    // Publication failed or destination custody could not be retained.
    Failed,
}

impl PublishOutcome {
    #[must_use]
    pub(crate) fn committed(self) -> bool {
        matches!(self, PublishOutcome::Written)
    }
}

impl RecoveryStore {
    // Open the store, returning the offered records and a disclosure origin.
    // A corrupt regular source enables a fresh store only after verified,
    // durable retained-source custody; every other refusal disables writing.
    #[must_use]
    pub(crate) fn open(path: PathBuf) -> (Vec<RecoveryRecord>, Self, Origin) {
        Self::open_with_limit(path, MAX_RECOVERY_FILE_BYTES)
    }

    fn open_with_limit(path: PathBuf, max_bytes: usize) -> (Vec<RecoveryRecord>, Self, Origin) {
        let authority = match StateAuthority::open_or_create_tightening(&path) {
            Ok(authority) => authority,
            Err(error) => {
                eprintln!("recovery: the store could not be opened: {error}");
                // Something at the path that cannot be opened is suspicious;
                // nothing there means the folder itself is the problem.
                let origin = if std::fs::symlink_metadata(&path).is_ok() {
                    Origin::QuarantineRefused
                } else {
                    Origin::Unavailable
                };
                return (Vec::new(), Self::disabled_at(), origin);
            }
        };
        match load(&path, &authority, max_bytes) {
            Ok(snapshot) => {
                let generation = snapshot.generation;
                let (records, dropped) = snapshot.partition_well_formed();
                (
                    records,
                    Self::enabled_at(authority, generation),
                    Origin::Loaded { dropped },
                )
            }
            Err(LoadError::Missing) => {
                (Vec::new(), Self::enabled_at(authority, 0), Origin::Missing)
            }
            Err(LoadError::Corrupt(expected)) => {
                drop(authority);
                if quarantine(&path, expected)
                    && let Ok(authority) = StateAuthority::create_new(&path)
                {
                    return (
                        Vec::new(),
                        Self::enabled_at(authority, 0),
                        Origin::RecoveredFromCorrupt,
                    );
                }
                (Vec::new(), Self::disabled_at(), Origin::QuarantineRefused)
            }
            Err(LoadError::Refused) => (Vec::new(), Self::disabled_at(), Origin::QuarantineRefused),
            Err(LoadError::Future) => (
                Vec::new(),
                Self::disabled_at(),
                Origin::ReadOnlyFutureVersion,
            ),
        }
    }

    fn enabled_at(authority: StateAuthority, generation: u64) -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(true)),
            gate: Arc::new(Mutex::new(RecoveryAuthority {
                authority: Some(authority),
                published_generation: generation,
            })),
        }
    }

    fn disabled_at() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            gate: Arc::new(Mutex::new(RecoveryAuthority {
                authority: None,
                published_generation: 0,
            })),
        }
    }

    // A live-but-disabled store for the boot case where no safe state path
    // could be resolved: it refuses every write.
    #[must_use]
    pub(crate) fn disabled() -> Self {
        Self::disabled_at()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn inert() -> Self {
        Self::disabled()
    }

    #[must_use]
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    // The highest generation published so far (seeds the app's monotonic
    // generation at boot so publications stay ordered across restarts).
    #[must_use]
    pub(crate) fn published_generation(&self) -> u64 {
        self.gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .published_generation
    }

    // Publish a bounded snapshot if `generation` is newer.
    //
    // The retained authority refuses a destination replacement. A publication
    // that rotated custody but missed its durability barrier advances the
    // generation fence, preventing an older writer from replacing its bytes.
    pub(crate) fn publish(
        &self,
        generation: u64,
        mut snapshot: RecoverySnapshot,
    ) -> PublishOutcome {
        let mut state = self
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if generation <= state.published_generation {
            return PublishOutcome::Superseded;
        }
        let Some(authority) = state.authority.as_mut() else {
            return PublishOutcome::Skipped;
        };
        snapshot.generation = generation;
        let result = authority.publish_with(|output| write_json_bounded(output, &snapshot));
        match result {
            Ok(_) => {
                state.published_generation = generation;
                PublishOutcome::Written
            }
            Err(error) => {
                match error.custody() {
                    PublishCustody::Rotated => state.published_generation = generation,
                    PublishCustody::Lost => {
                        state.authority = None;
                        self.enabled.store(false, Ordering::Release);
                    }
                    PublishCustody::Unchanged => {}
                    _ => {}
                }
                PublishOutcome::Failed
            }
        }
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
                "recovery journal exceeds its byte limit",
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

fn write_json_bounded(output: &mut dyn Write, snapshot: &RecoverySnapshot) -> io::Result<()> {
    let mut bounded = BoundedWriter {
        output,
        remaining: MAX_RECOVERY_FILE_BYTES,
    };
    serde_json::to_writer(&mut bounded, snapshot).map_err(io::Error::other)
}

// A child module keeps private gates testable within the file-size ceiling.
#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;

#[cfg(test)]
impl RecoveryRecord {
    #[must_use]
    pub(crate) fn new(document_id: impl Into<String>, content: String) -> Self {
        let content_bytes = u64::try_from(content.len()).unwrap_or(u64::MAX);
        Self::from_content(document_id, content, content_bytes)
    }
}
