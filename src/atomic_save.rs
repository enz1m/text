//! Streaming document publication through the framework user-file writer.
//!
//! Existing files retain mode and ownership, final symlinks remain links, and
//! callback failures leave the prior bytes untouched. Hard-linked and
//! non-regular destinations are refused rather than downgraded to truncation.
//! An in-place save replaces only the revision its tab loaded, saved or had
//! acknowledged; any other revision at the destination is refused unwritten.

use enzui_core::file_watch::FileIdentity;
use std::io;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteMode {
    Atomic,
}

// The destination revision a save is entitled to replace. Identities follow a
// final symlink, like the load and the file watcher, so a retargeted link is a
// different revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Replaces {
    // A Save As destination: the file chooser already confirmed any overwrite.
    Chosen,
    // Exactly this revision, and it must still exist.
    Revision(FileIdentity),
    // No revision: the file must still be absent and is created without clobbering.
    Absent,
}

#[derive(Debug)]
pub(crate) enum WriteError {
    // The destination is no longer the revision the save may replace; nothing
    // was published. Carries the revision found there (`None` when it is gone).
    ExternalRevision(Option<FileIdentity>),
    Save(enzui_core::user_file::SaveError),
}

#[cfg(test)]
pub(crate) fn write_document(path: &Path, bytes: &[u8]) -> Result<WriteMode, WriteError> {
    write_document_with(path, &DocumentBytes::Exact(bytes), Replaces::Chosen)
}

// The bytes a save publishes without materialising a second document copy.
//
// Line-ending conversion streams chunk by chunk.
pub(crate) enum DocumentBytes<'a> {
    // Publish exactly these contiguous bytes.
    #[cfg(test)]
    Exact(&'a [u8]),
    // Stream an exact immutable piece-tree generation.
    ChunkedExact(&'a enzui_core::chunked_text::TextSnapshot),
    // Stream a piece-tree generation while converting LF to CRLF.
    Crlf(&'a enzui_core::chunked_text::TextSnapshot),
    // Stream a piece-tree generation while emitting a bare CR for each LF.
    Cr(&'a enzui_core::chunked_text::TextSnapshot),
}

impl DocumentBytes<'_> {
    fn stream(&self, out: &mut impl io::Write) -> io::Result<()> {
        match self {
            #[cfg(test)]
            Self::Exact(bytes) => out.write_all(bytes),
            Self::ChunkedExact(snapshot) => snapshot.write_to(out),
            Self::Crlf(snapshot) => snapshot.write_crlf_to(out),
            Self::Cr(snapshot) => write_cr(snapshot, out),
        }
    }
}

fn write_cr(
    snapshot: &enzui_core::chunked_text::TextSnapshot,
    out: &mut impl io::Write,
) -> io::Result<()> {
    let mut result = Ok(());
    snapshot.for_each_chunk(|chunk| {
        if result.is_err() {
            return;
        }
        let mut rest = chunk.as_bytes();
        while let Some(at) = rest.iter().position(|b| *b == b'\n') {
            result = out.write_all(&rest[..at]);
            if result.is_ok() {
                result = out.write_all(b"\r");
            }
            if result.is_err() {
                return;
            }
            rest = &rest[at + 1..];
        }
        if result.is_ok() {
            result = out.write_all(rest);
        }
    });
    result
}

// Atomically save a snapshot, converting line endings while streaming.
//
// # Errors
// Returns `ExternalRevision` when the destination is not the revision
// `replaces` names, before or after streaming; otherwise a classified error
// when destination admission, writing, or durable publication fails.
// Pre-publication failures leave the destination unchanged.
pub(crate) fn write_document_with(
    path: &Path,
    document: &DocumentBytes<'_>,
    replaces: Replaces,
) -> Result<WriteMode, WriteError> {
    let policy = match replaces {
        Replaces::Revision(_) => enzui_core::user_file::SavePolicy::ReplaceExisting,
        Replaces::Absent => enzui_core::user_file::SavePolicy::CreateNew,
        Replaces::Chosen => match std::fs::symlink_metadata(path) {
            Ok(_) => enzui_core::user_file::SavePolicy::ReplaceExisting,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                enzui_core::user_file::SavePolicy::ReplaceOrCreate
            }
            Err(error) => {
                return Err(WriteError::Save(
                    enzui_core::user_file::SaveError::PrePublication {
                        operation: enzui_core::user_file::SaveOperation::InspectDestination,
                        source: error,
                    },
                ));
            }
        },
    };
    check_replaces(path, replaces)?;
    let saved = enzui_core::user_file::save_with(path, policy, |file| {
        #[cfg(test)]
        if let Some(error) = fault::denied().or_else(fault::take) {
            return Err(error);
        }
        let mut output = io::BufWriter::with_capacity(256 * 1024, file);
        document.stream(&mut output)?;
        std::io::Write::flush(&mut output)?;
        #[cfg(test)]
        fault::run_after_stream(path);
        // The writer admitted the destination before this callback and refuses a
        // replaced inode at publication, so a revision that changed while the
        // document streamed is refused here.
        check_replaces(path, replaces)
            .map_err(|_| io::Error::other("the destination changed while the document was written"))
    });
    if let Err(error) = saved {
        if error.was_published() {
            return Err(WriteError::Save(error));
        }
        check_replaces(path, replaces)?;
        return Err(WriteError::Save(error));
    }
    #[cfg(test)]
    if let Some(source) = fault::take_published_failure() {
        return Err(WriteError::Save(
            enzui_core::user_file::SaveError::PublishedButNotDurable {
                operation: enzui_core::user_file::SaveOperation::SyncParent,
                source,
            },
        ));
    }
    Ok(WriteMode::Atomic)
}

fn check_replaces(path: &Path, replaces: Replaces) -> Result<(), WriteError> {
    let expected = match replaces {
        Replaces::Chosen => return Ok(()),
        Replaces::Revision(identity) => Some(identity),
        Replaces::Absent => None,
    };
    let current = match std::fs::metadata(path) {
        Ok(metadata) => Some(FileIdentity::from_metadata(&metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        // A destination that cannot be inspected is left to the writer's
        // own classified refusal.
        Err(_) => return Ok(()),
    };
    if current == expected {
        Ok(())
    } else {
        Err(WriteError::ExternalRevision(current))
    }
}

// One forced write failure, armed per thread as a deterministic `ENOSPC`.
#[cfg(test)]
pub(crate) mod fault {
    use std::cell::{Cell, RefCell};
    use std::io;
    use std::path::Path;
    type AfterStream = Box<dyn FnOnce(&Path)>;
    thread_local! {
        static ARMED: Cell<bool> = const { Cell::new(false) };
        static PUBLISHED_FAILURE: Cell<bool> = const { Cell::new(false) };
        static AFTER_STREAM: RefCell<Option<AfterStream>> = const { RefCell::new(None) };
    }

    // Run `action` once on this thread after the next save streams its bytes,
    // inside the writer's admission-to-publication window.
    pub(crate) fn arm_after_stream(action: impl FnOnce(&Path) + 'static) {
        AFTER_STREAM.with(|slot| *slot.borrow_mut() = Some(Box::new(action)));
    }

    pub(super) fn run_after_stream(path: &Path) {
        if let Some(action) = AFTER_STREAM.with(|slot| slot.borrow_mut().take()) {
            action(path);
        }
    }
    // Arm one forced failure on this thread.
    pub(crate) fn arm() {
        ARMED.with(|a| a.set(true));
    }

    thread_local! {
        static DENIED: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn denied() -> Option<io::Error> {
        DENIED.with(|denied| {
            denied.get().then(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected permission denial",
                )
            })
        })
    }

    // Report one uncertainty after the publication call completes.
    pub(crate) fn arm_published_failure() {
        PUBLISHED_FAILURE.with(|armed| armed.set(true));
    }

    pub(super) fn take_published_failure() -> Option<io::Error> {
        PUBLISHED_FAILURE.with(|armed| {
            armed
                .replace(false)
                .then(|| io::Error::other("injected parent sync failure"))
        })
    }

    // The faults a test armed on its own thread, carried to the blocking
    // owner thread that performs the save.
    #[derive(Clone, Copy, Debug, Default)]
    pub(crate) struct Carried {
        storage: bool,
        denied: bool,
        published: bool,
    }

    pub(crate) fn carry() -> Carried {
        Carried {
            storage: ARMED.with(|a| a.replace(false)),
            denied: DENIED.with(Cell::get),
            published: PUBLISHED_FAILURE.with(|p| p.replace(false)),
        }
    }

    #[must_use]
    pub(crate) fn install(carried: Carried) -> Installed {
        ARMED.with(|a| a.set(carried.storage));
        DENIED.with(|d| d.set(carried.denied));
        PUBLISHED_FAILURE.with(|p| p.set(carried.published));
        Installed(())
    }

    pub(crate) struct Installed(());

    impl Drop for Installed {
        fn drop(&mut self) {
            ARMED.with(|a| a.set(false));
            DENIED.with(|d| d.set(false));
            PUBLISHED_FAILURE.with(|p| p.set(false));
        }
    }

    // Consume a pending forced failure, if armed.
    pub(crate) fn take() -> Option<io::Error> {
        ARMED.with(|a| {
            if a.replace(false) {
                Some(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected: no space left on device",
                ))
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "enzui-text-1701-unit-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("scratch");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_destinations_permission_bits_survive_a_save() {
        crate::test_fixture::with_locale(|| {
            let scratch = Scratch::new("mode");
            let path = scratch.0.join("secret.txt");
            std::fs::write(&path, "old").expect("fixture");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");

            assert_eq!(
                write_document(&path, b"new").expect("write"),
                WriteMode::Atomic
            );

            let mode = std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
            assert_eq!(std::fs::read_to_string(&path).expect("read"), "new");
        });
    }

    #[test]
    fn a_hard_linked_file_is_refused_without_changing_either_name() {
        crate::test_fixture::with_locale(|| {
            let scratch = Scratch::new("hardlink");
            let a = scratch.0.join("a.txt");
            let b = scratch.0.join("b.txt");
            std::fs::write(&a, "old").expect("fixture");
            std::fs::hard_link(&a, &b).expect("hard link fixture");
            let inode = std::fs::metadata(&a).expect("metadata").ino();

            write_document(&a, b"new").expect_err("strict atomic save refuses hard links");

            assert_eq!(std::fs::read_to_string(&a).expect("first name"), "old");
            assert_eq!(std::fs::read_to_string(&b).expect("second name"), "old");
            assert_eq!(std::fs::metadata(&a).expect("metadata").ino(), inode);
        });
    }

    fn revision(path: &Path) -> FileIdentity {
        FileIdentity::from_metadata(&std::fs::metadata(path).expect("metadata"))
    }

    #[test]
    fn a_revision_changed_while_the_document_streams_is_never_replaced() {
        crate::test_fixture::with_locale(|| {
            let scratch = Scratch::new("revision");
            let path = scratch.0.join("note.txt");
            let replacement = scratch.0.join("replacement.txt");
            std::fs::write(&path, "loaded").expect("fixture");
            let loaded = revision(&path);
            std::fs::write(&replacement, "replaced while saving").expect("fixture");
            fault::arm_after_stream(move |path| {
                std::fs::rename(&replacement, path).expect("replace");
            });
            let refused = write_document_with(
                &path,
                &DocumentBytes::Exact(b"draft"),
                Replaces::Revision(loaded),
            );
            assert!(
                matches!(refused, Err(WriteError::ExternalRevision(Some(found))) if found == revision(&path))
            );
            assert_eq!(
                std::fs::read_to_string(&path).expect("read"),
                "replaced while saving"
            );

            let loaded = revision(&path);
            fault::arm_after_stream(|path| {
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(path)
                    .expect("open");
                io::Write::write_all(&mut file, b" and appended").expect("append");
            });
            let refused = write_document_with(
                &path,
                &DocumentBytes::Exact(b"draft"),
                Replaces::Revision(loaded),
            );
            assert!(matches!(
                refused,
                Err(WriteError::ExternalRevision(Some(_)))
            ));
            assert_eq!(
                std::fs::read_to_string(&path).expect("read"),
                "replaced while saving and appended"
            );

            let loaded = revision(&path);
            write_document_with(
                &path,
                &DocumentBytes::Exact(b"draft"),
                Replaces::Revision(loaded),
            )
            .expect("the loaded revision is replaced");
            assert_eq!(std::fs::read_to_string(&path).expect("read"), "draft");
        });
    }

    #[test]
    fn a_file_that_appeared_is_not_clobbered_by_a_save_entitled_to_create() {
        crate::test_fixture::with_locale(|| {
            let scratch = Scratch::new("absent");
            let path = scratch.0.join("note.txt");
            fault::arm_after_stream(|path| {
                std::fs::write(path, "created elsewhere").expect("create");
            });
            let refused =
                write_document_with(&path, &DocumentBytes::Exact(b"draft"), Replaces::Absent);
            assert!(matches!(
                refused,
                Err(WriteError::ExternalRevision(Some(_)))
            ));
            assert_eq!(
                std::fs::read_to_string(&path).expect("read"),
                "created elsewhere"
            );

            std::fs::remove_file(&path).expect("remove");
            write_document_with(&path, &DocumentBytes::Exact(b"draft"), Replaces::Absent)
                .expect("an absent file is created");
            assert_eq!(std::fs::read_to_string(&path).expect("read"), "draft");
        });
    }
}
