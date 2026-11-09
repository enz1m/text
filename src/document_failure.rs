//! Typed, non-paged document open/save failures.
//!
//! Filesystem paths and backend error text are diagnostic data, not interface
//! copy. This type classifies an `io::Error` at the boundary, immediately
//! turns all retained detail into an escaped diagnostic, and exposes only
//! operation-specific prose for banners.

use std::path::Path;

use enzui_core::file_watch::FileIdentity;
use enzui_core::io_class::IoFailureCategory;
use enzui_core::paged_text::{PagedTextSourceError, PagedTextSourceErrorKind};

#[derive(Clone, Debug)]
pub(crate) struct DocumentFailure {
    category: IoFailureCategory,
    escaped_detail: String,
    published_not_durable: bool,
    // `Some` when a save was refused because the file on disk is another
    // revision than the one the tab may replace; holds that revision.
    external_revision: Option<Option<FileIdentity>>,
}

impl DocumentFailure {
    // Classify an I/O error where it is produced and retain escaped detail.
    #[must_use]
    pub(crate) fn from_io(path: &Path, error: std::io::Error) -> Self {
        Self {
            category: IoFailureCategory::classify(&error),
            escaped_detail: escape_diagnostic(&format!(
                "path={}; kind={:?}; error={error}",
                path.display(),
                error.kind()
            )),
            published_not_durable: false,
            external_revision: None,
        }
    }

    #[must_use]
    pub(crate) fn from_user_file(path: &Path, error: enzui_core::user_file::SaveError) -> Self {
        let published_not_durable = error.was_published();
        let io_error = error.io_error();
        Self {
            category: IoFailureCategory::classify(io_error),
            escaped_detail: escape_diagnostic(&format!(
                "path={}; phase={}; operation={:?}; kind={:?}; error={error}",
                path.display(),
                if published_not_durable {
                    "published"
                } else {
                    "not-published"
                },
                error.operation(),
                io_error.kind()
            )),
            published_not_durable,
            external_revision: None,
        }
    }

    #[must_use]
    pub(crate) fn from_external_revision(path: &Path, found: Option<FileIdentity>) -> Self {
        Self {
            category: IoFailureCategory::Unknown,
            escaped_detail: escape_diagnostic(&format!(
                "path={}; phase=not-published; refused=external-revision; found={found:?}",
                path.display()
            )),
            published_not_durable: false,
            external_revision: Some(found),
        }
    }

    // The revision a refused save found on disk, `Some(None)` when the file is gone.
    #[must_use]
    pub(crate) fn external_revision(&self) -> Option<Option<FileIdentity>> {
        self.external_revision
    }

    #[must_use]
    pub(crate) fn from_detail(path: &Path, detail: impl AsRef<str>) -> Self {
        Self {
            category: IoFailureCategory::Unknown,
            escaped_detail: escape_diagnostic(&format!(
                "path={}; error={}",
                path.display(),
                detail.as_ref()
            )),
            published_not_durable: false,
            external_revision: None,
        }
    }

    #[must_use]
    pub(crate) fn from_paged(path: &Path, error: PagedTextSourceError) -> Self {
        let category = match error.kind() {
            PagedTextSourceErrorKind::Permission => IoFailureCategory::Permission,
            PagedTextSourceErrorKind::NotFound => IoFailureCategory::NotFound,
            PagedTextSourceErrorKind::Unreadable => IoFailureCategory::Unknown,
        };
        Self {
            category,
            escaped_detail: escape_diagnostic(&format!(
                "path={}; error={}",
                path.display(),
                error.diagnostic()
            )),
            published_not_durable: false,
            external_revision: None,
        }
    }

    #[must_use]
    pub(crate) fn escaped_diagnostic(&self) -> &str {
        &self.escaped_detail
    }

    #[must_use]
    pub(crate) fn open_summary(&self) -> String {
        match self.category {
            IoFailureCategory::Permission => enzui_core::localization::text(
                "text-could-not-read-this-file-because-access-219890",
            ),
            IoFailureCategory::NotFound | IoFailureCategory::NotADirectory => {
                enzui_core::localization::text("text-could-not-read-this-file-because-it-46c576")
            }
            IoFailureCategory::StorageExhausted => enzui_core::localization::text(
                "text-could-not-read-this-file-because-storage-429221",
            ),
            IoFailureCategory::Unknown => {
                enzui_core::localization::text("text-could-not-read-this-file-bba5c0")
            }
        }
    }

    #[must_use]
    pub(crate) fn reload_summary(&self) -> String {
        match self.category {
            IoFailureCategory::Permission => enzui_core::localization::text(
                "text-could-not-reload-this-document-because-access-8ab709",
            ),
            IoFailureCategory::NotFound | IoFailureCategory::NotADirectory => {
                enzui_core::localization::text(
                    "text-could-not-reload-this-document-because-the-e0f617",
                )
            }
            IoFailureCategory::StorageExhausted => enzui_core::localization::text(
                "text-could-not-reload-this-document-because-storage-989f70",
            ),
            IoFailureCategory::Unknown => {
                enzui_core::localization::text("text-could-not-reload-this-document-08a717")
            }
        }
    }

    #[must_use]
    pub(crate) fn save_summary(&self) -> String {
        if self.published_not_durable {
            return enzui_core::localization::text(
                "text-document-publication-occurred-but-current-on-disk-f0952f",
            );
        }
        match self.category {
            IoFailureCategory::Permission => enzui_core::localization::text(
                "text-could-not-save-this-document-because-access-0729cf",
            ),
            IoFailureCategory::NotFound | IoFailureCategory::NotADirectory => {
                enzui_core::localization::text(
                    "text-could-not-save-this-document-because-its-c58e75",
                )
            }
            IoFailureCategory::StorageExhausted => enzui_core::localization::text(
                "text-could-not-save-this-document-because-storage-95a1bd",
            ),
            IoFailureCategory::Unknown => {
                enzui_core::localization::text("text-could-not-save-this-document-because-a-22f084")
            }
        }
    }

    // Save-As copy may identify the chosen filename, but never its path.
    #[must_use]
    pub(crate) fn save_as_summary(&self, path: &Path) -> String {
        let raw_name = path.file_name().map_or_else(
            || enzui_core::localization::text("text-this-item-545362").into(),
            |name| name.to_string_lossy(),
        );
        let name = crate::safe_text::bounded_inline(&raw_name);
        if self.published_not_durable {
            return enzui_core::localized!("text-document-publication-occurred-for-name-but-current-72de51", "name" => name);
        }
        let reason = match self.category {
            IoFailureCategory::Permission => {
                enzui_core::localization::text("text-access-was-denied-111ed0")
            }
            IoFailureCategory::NotFound | IoFailureCategory::NotADirectory => {
                enzui_core::localization::text("text-the-destination-is-unavailable-c97cad")
            }
            IoFailureCategory::StorageExhausted => {
                enzui_core::localization::text("text-storage-space-or-quota-is-exhausted-74982f")
            }
            IoFailureCategory::Unknown => {
                enzui_core::localization::text("text-a-storage-error-occurred-5bba5a")
            }
        };
        enzui_core::localized!("text-could-not-save-name-because-reason-6af140", "name" => name, "reason" => reason)
    }
}

// `U+2028`/`U+2029` are *not* `char::is_control`, but a log viewer, journal
// reader or terminal may still break a line on them, so a filesystem name can
// forge an extra diagnostic record without them being escaped here.
fn escape_diagnostic(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c.is_control()
            || matches!(
                c,
                '\u{2028}'
                    | '\u{2029}'
                    | '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            )
        {
            escaped.extend(c.escape_unicode());
        } else {
            escaped.push(c);
        }
    }
    escaped
}
