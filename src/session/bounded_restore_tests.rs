use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn sandbox(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "text-app-session-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn security_dir(label: &str) -> PathBuf {
    let dir = sandbox(label);
    std::fs::create_dir_all(&dir).expect("security dir");
    dir
}

fn write_session(
    path: impl AsRef<std::path::Path>,
    bytes: impl AsRef<[u8]>,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes.as_ref())
}

fn tiny_limits() -> SessionLimits {
    SessionLimits::new(4096, 4, 2, 32, 128)
}

fn within_deadline<T, F>(label: &str, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(value) => {
            let _ = handle.join();
            value
        }
        Err(_) => panic!("{label} did not complete within its deadline"),
    }
}

#[test]
fn json_tabs_over_cap_refuse_whole() {
    crate::test_fixture::with_locale(|| {
        let path = sandbox("json-tabs");
        write_session(
            &path,
            r#"{"version":1,"paths":["/a","/b","/c","/d","/e"],"active_path":"/a"}"#,
        )
        .unwrap();
        let err = load_with_limits(&path, None, tiny_limits()).unwrap_err();
        assert!(matches!(
            err,
            LoadError::Refused(SessionRefusal::TooManyEntries)
        ));
        let _ = std::fs::remove_file(path);
    });
}

#[test]
fn future_version_json_is_read_only_and_preserved() {
    crate::test_fixture::with_locale(|| {
        let path = sandbox("future");
        let original = r#"{"version":99,"tabs":{"tabs":[],"active_path":null},"workspaces":[]}"#;
        write_session(&path, original).unwrap();
        let (snapshot, mut store, origin) = SessionStore::open(path.clone(), None);
        assert_eq!(origin, Origin::ReadOnlyFutureVersion(99));
        assert!(snapshot.tabs().tabs.is_empty());
        assert!(!store.discard_open_tabs());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let _ = std::fs::remove_file(path);
    });
}

#[test]
fn a_fifo_source_is_refused_without_blocking() {
    crate::test_fixture::with_locale(|| {
        let dir = security_dir("fifo");
        let fifo = dir.join("session.list");
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .unwrap();
        let origin = within_deadline("FIFO open", move || SessionStore::open(fifo, None).2);
        assert_eq!(origin, Origin::Refused(SessionRefusal::NotRegularFile));
        let _ = std::fs::remove_dir_all(&dir);
    });
}

#[test]
fn a_symlink_source_is_not_followed() {
    crate::test_fixture::with_locale(|| {
        use std::os::unix::fs::symlink;
        let dir = security_dir("source-symlink");
        let victim = dir.join("victim.secret");
        write_session(&victim, "PRECIOUS").unwrap();
        let source = dir.join("session.list");
        symlink(&victim, &source).unwrap();
        let (_, _, origin) = SessionStore::open(source, None);
        assert_eq!(origin, Origin::Refused(SessionRefusal::OpenRefused));
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "PRECIOUS");
        let _ = std::fs::remove_dir_all(&dir);
    });
}

#[test]
fn an_under_cap_same_inode_append_during_read_is_refused() {
    crate::test_fixture::with_locale(|| {
        use std::io::Write;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let _reset = test_seam::Reset;
        let limits = SessionLimits::new(64, 4, 2, 128, 4096);
        let path = sandbox("grow-under-cap");

        write_session(&path, "/x/one.txt\n").unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!((before.len() as u64) < limits.max_file_bytes);

        let reached = Arc::new(AtomicBool::new(false));
        {
            let reached = std::sync::Arc::clone(&reached);
            let grow_path = path.clone();
            test_seam::set_after_metadata(move || {
                reached.store(true, Ordering::SeqCst);
                let mut f = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&grow_path)
                    .unwrap();
                f.write_all(b"/x/two.txt\n").unwrap();
            });
        }

        let (_, mut store, origin) = SessionStore::open_with_limits(path.clone(), None, limits);
        assert!(reached.load(Ordering::SeqCst));
        assert_eq!(origin, Origin::Refused(SessionRefusal::Unstable));
        assert!(!store.discard_open_tabs());
        let after = std::fs::read(&path).unwrap();
        assert!(after.starts_with(&before) && after.len() > before.len());
        let _ = std::fs::remove_file(path);
    });
}
