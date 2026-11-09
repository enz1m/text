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

fn paths_of(snapshot: &SessionSnapshot) -> Vec<PathBuf> {
    snapshot
        .tabs()
        .tabs
        .iter()
        .map(|t| t.path.clone())
        .collect()
}

#[test]
fn real_v1_json_migrates_all_paths_into_open_files() {
    crate::test_fixture::with_locale(|| {
        let path = sandbox("v1");
        write_session(
            &path,
            r#"{"version":1,"paths":["/tmp/one.txt","/tmp/two.txt"],"active_path":"/tmp/two.txt"}"#,
        )
        .unwrap();
        let (snapshot, _, origin) = SessionStore::open(path.clone(), None);
        assert_eq!(origin, Origin::Restored);
        assert_eq!(
            paths_of(&snapshot),
            vec![PathBuf::from("/tmp/one.txt"), PathBuf::from("/tmp/two.txt")]
        );
        assert!(snapshot.tabs().tabs.iter().all(|t| t.workspace.is_none()));
        assert_eq!(
            snapshot.tabs().active_path,
            Some(PathBuf::from("/tmp/two.txt"))
        );
        assert!(snapshot.workspaces.is_empty());
        let _ = std::fs::remove_file(path);
    });
}

#[test]
fn v2_workspace_tab_sets_migrate_into_one_global_mixed_set() {
    crate::test_fixture::with_locale(|| {
        let path = sandbox("v2");
        let root = PathBuf::from("/tmp/proj");
        write_session(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": 2,
                "open_files": {
                    "paths": ["/tmp/loose.txt"],
                    "active_path": "/tmp/loose.txt",
                },
                "workspaces": [{
                    "root": root,
                    "name": "proj",
                    "tabs": { "paths": ["/tmp/proj/a.txt"], "active_path": "/tmp/proj/a.txt" },
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        let (snapshot, _, _) = SessionStore::open(path.clone(), None);
        let loose = snapshot
            .tabs()
            .tabs
            .iter()
            .find(|t| t.path == *"/tmp/loose.txt")
            .expect("open-files tab survives");
        assert_eq!(loose.workspace, None);
        let named = snapshot
            .tabs()
            .tabs
            .iter()
            .find(|t| t.path == *"/tmp/proj/a.txt")
            .expect("workspace tab survives");
        assert_eq!(named.workspace, Some(root.clone()));
        assert_eq!(
            snapshot
                .workspaces()
                .iter()
                .map(|w| w.root.clone())
                .collect::<Vec<_>>(),
            vec![root]
        );
        let _ = std::fs::remove_file(path);
    });
}
