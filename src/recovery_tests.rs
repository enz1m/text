use super::*;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("text-app-recovery-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_private(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).expect("write private recovery source");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .expect("set recovery source mode");
}

#[test]
fn partition_quarantines_malformed_and_duplicate_identities() {
    crate::test_fixture::with_locale(|| {
        let good = RecoveryRecord::new("a", "ok".to_owned());
        let duplicate = RecoveryRecord::new("a", "other".to_owned());
        let mut bad = RecoveryRecord::new("b", "ok".to_owned());
        bad.content_bytes = 1234;
        let snapshot = RecoverySnapshot::from_records(vec![good, duplicate, bad]);
        let (kept, dropped) = snapshot.partition_well_formed();
        assert_eq!(kept.len(), 1);
        assert_eq!(dropped, 2);
    });
}

#[test]
fn a_future_version_is_left_untouched_and_disables_writing() {
    crate::test_fixture::with_locale(|| {
        let dir = TempDir::new();
        let path = dir.path("recovery.json");
        let raw = format!("{{\"version\":{},\"records\":[]}}", SCHEMA_VERSION + 5);
        write_private(&path, raw.as_bytes());
        let (records, store, origin) = RecoveryStore::open(path.clone());
        assert!(records.is_empty());
        assert!(!store.is_enabled());
        assert_eq!(origin, Origin::ReadOnlyFutureVersion);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        assert_eq!(
            store.publish(1, RecoverySnapshot::default()),
            PublishOutcome::Skipped
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
    });
}

#[cfg(unix)]
#[test]
fn a_symlink_has_no_admitted_identity_and_is_left_untouched() {
    crate::test_fixture::with_locale(|| {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new();
        let victim = dir.path("victim.secret");
        std::fs::write(&victim, "TOP SECRET").expect("victim");
        let link = dir.path("recovery.json");
        symlink(&victim, &link).expect("symlink");

        let (records, store, origin) = RecoveryStore::open(link.clone());
        assert!(records.is_empty());
        assert!(!store.is_enabled());
        assert_eq!(origin, Origin::QuarantineRefused);
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "TOP SECRET");
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    });
}

fn content_snapshot(id: &str, text: &str) -> RecoverySnapshot {
    RecoverySnapshot::from_records(vec![RecoveryRecord::new(id, text.to_owned())])
}

#[test]
fn a_delayed_old_writer_cannot_overtake_a_concurrent_tombstone() {
    crate::test_fixture::with_locale(|| {
        use std::thread;
        use std::time::Duration;

        let dir = TempDir::new();
        let path = dir.path("recovery.json");
        let (_records, store, _origin) = RecoveryStore::open(path.clone());

        let delayed = store.clone();
        let old_writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(40));
            delayed.publish(6, content_snapshot("d", "unsaved work"))
        });

        let tombstone = store.publish(7, RecoverySnapshot::default());
        let stale = old_writer.join().expect("old writer thread");

        assert_eq!(tombstone, PublishOutcome::Written);
        assert_eq!(stale, PublishOutcome::Superseded);

        let (records, _store2, _origin2) = RecoveryStore::open(path);
        assert!(records.is_empty());
    });
}
