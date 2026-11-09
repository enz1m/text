//! Workspace model and lazy filesystem tree for the editor sidebar.
//!
//! A workspace is only a user-chosen root, its display name, and the tabs last
//! open while it was active. `Open Files` is the synthetic navigation context
//! documents with no workspace association live under; it has no sidebar row.
//! Directory children are read only when their disclosure is toggled.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use enzui_theme::WorkspaceAccentKey;

use crate::model::Model;

pub(crate) const OPEN_FILES_KEY: &str = "workspace:open-files";
pub(crate) const DIRECTORY_ENTRY_CAP: usize = 1_000;

// One open document as remembered on disk, with the navigation context it was
// opened under. `workspace == None` means the synthetic Open Files context.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistedTab {
    pub path: PathBuf,
    #[serde(default)]
    pub workspace: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistedTabs {
    #[serde(default)]
    pub tabs: Vec<PersistedTab>,
    #[serde(default)]
    pub active_path: Option<PathBuf>,
}

impl PersistedTabs {
    // Snapshot the live global tab set, keeping each tab's workspace
    // association and the active path. Untitled tabs are not persisted.
    pub(crate) fn from_model(model: &Model) -> Self {
        Self {
            tabs: model
                .tabs
                .iter()
                .filter_map(|tab| {
                    tab.file_path.clone().map(|path| PersistedTab {
                        path,
                        workspace: tab.workspace_root.clone(),
                    })
                })
                .collect(),
            active_path: model.active_tab().file_path.clone(),
        }
    }
}

// A named workspace root used for navigation and grouping. The tab set is
// global; see [`PersistedTabs`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistedWorkspace {
    pub root: PathBuf,
    pub name: String,
    // The optional workspace accent, persisted as the theme key's stable
    // `id_str` (`"blue"`, `"teal"`, ...) so it resolves against the current
    // preset rather than a frozen colour.
    //
    // Missing fields parse as no accent, and an unrecognised string resolves to
    // `None` rather than failing the parse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ActiveWorkspace {
    OpenFiles,
    Named(PathBuf),
}

#[derive(Clone, Debug)]
pub(crate) struct Workspace {
    pub root: PathBuf,
    pub name: String,
    pub expanded: bool,
    pub accent: Option<WorkspaceAccentKey>,
    children: DirectoryLoad,
}

#[derive(Clone, Debug)]
pub(crate) enum DirectoryLoad {
    Unread,
    Loading,
    Loaded(Vec<TreeEntry>, usize),
    // A classified read failure; the raw `io::Error` text is never rendered.
    Error(DirectoryError),
}

// Why one directory could not be listed, in terms a user can act on.
//
// The errno mapping is the shared
// [`enzui_core::io_class::IoFailureCategory`]; only the sidebar wording
// lives here. Unnamed categories become
// [`Unreadable`](DirectoryErrorKind::Unreadable), never the raw payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectoryErrorKind {
    // The OS refused to list the folder.
    Permission,
    Missing,
    NotADirectory,
    // Anything else; the retained diagnostic carries the detail.
    Unreadable,
}

impl DirectoryErrorKind {
    // Display copy contains no attacker-controlled detail.
    #[must_use]
    pub(crate) fn safe_reason(self) -> String {
        match self {
            Self::Permission => enzui_core::localization::text("text-permission-denied-b77f13"),
            Self::Missing => {
                enzui_core::localization::text("text-the-folder-no-longer-exists-9cc029")
            }
            Self::NotADirectory => {
                enzui_core::localization::text("text-this-is-not-a-folder-8aeafe")
            }
            Self::Unreadable => {
                enzui_core::localization::text("text-the-folder-could-not-be-read-2b5507")
            }
        }
    }
}

// A classified directory-read failure: safe prose for the row, the full
// payload for logs.
//
// Only [`safe_label`](Self::safe_label) may be rendered;
// [`diagnostic`](Self::diagnostic) is the unmodified `io::Error` text for logs.
#[derive(Clone, Debug)]
pub(crate) struct DirectoryError {
    kind: DirectoryErrorKind,
    diagnostic: String,
}

impl DirectoryError {
    #[must_use]
    pub(crate) fn classify(error: &std::io::Error) -> Self {
        use enzui_core::io_class::IoFailureCategory as Category;
        let kind = match Category::classify(error) {
            Category::Permission => DirectoryErrorKind::Permission,
            Category::NotFound => DirectoryErrorKind::Missing,
            Category::NotADirectory => DirectoryErrorKind::NotADirectory,
            Category::StorageExhausted | Category::Unknown => DirectoryErrorKind::Unreadable,
        };
        Self {
            kind,
            diagnostic: error.to_string(),
        }
    }

    #[must_use]
    pub(crate) fn kind(&self) -> DirectoryErrorKind {
        self.kind
    }

    // The unmodified backend payload, for logs only.
    #[must_use]
    pub(crate) fn diagnostic(&self) -> &str {
        &self.diagnostic
    }

    // Bounded, single-line, user-safe row copy.
    #[must_use]
    pub(crate) fn safe_label(&self) -> String {
        enzui_core::localized!("text-unavailable-p0-d27d60", "p0" => self.kind.safe_reason())
    }

    // The retained payload escaped for one log line: `Debug` turns a newline,
    // `NUL` or bidi override into a `\u{..}` escape, so a hostile payload
    // cannot forge a record or reorder the text.
    #[must_use]
    pub(crate) fn escaped_diagnostic(&self) -> String {
        format!("{:?}", self.diagnostic())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TreeEntry {
    path: PathBuf,
    name: String,
    kind: EntryKind,
    expanded: bool,
    children: DirectoryLoad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryKind {
    Directory,
    File,
    Symlink,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TreeRowKind {
    Workspace(PathBuf),
    Directory { workspace: PathBuf, path: PathBuf },
    File { workspace: PathBuf, path: PathBuf },
    Status,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeRow {
    pub key: String,
    pub label: String,
    pub level: u8,
    pub has_children: bool,
    pub expanded: bool,
    pub icon: &'static str,
    pub kind: TreeRowKind,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceState {
    pub workspaces: Vec<Workspace>,
    pub active: ActiveWorkspace,
    pub selected_key: String,
    // Cached flat row projection, invalidated by every mutation, so a redraw
    // that changes nothing reuses the same `Rc<[TreeRow]>` instead of
    // allocating per row.
    rows_cache: std::cell::RefCell<Option<std::rc::Rc<[TreeRow]>>>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            workspaces: Vec::new(),
            active: ActiveWorkspace::OpenFiles,
            selected_key: OPEN_FILES_KEY.to_owned(),
            rows_cache: std::cell::RefCell::new(None),
        }
    }
}

impl WorkspaceState {
    pub(crate) fn from_persisted(workspaces: Vec<PersistedWorkspace>) -> Self {
        Self {
            workspaces: workspaces
                .into_iter()
                .map(|saved| Workspace {
                    root: saved.root,
                    name: saved.name,
                    expanded: false,
                    accent: saved
                        .accent
                        .as_deref()
                        .and_then(WorkspaceAccentKey::from_id_str),
                    children: DirectoryLoad::Unread,
                })
                .collect(),
            ..Self::default()
        }
    }

    fn invalidate_rows(&self) {
        self.rows_cache.borrow_mut().take();
    }

    pub(crate) fn persisted_workspaces(&self) -> Vec<PersistedWorkspace> {
        self.workspaces
            .iter()
            .map(|workspace| PersistedWorkspace {
                root: workspace.root.clone(),
                name: workspace.name.clone(),
                accent: workspace.accent.map(|key| key.id_str().to_owned()),
            })
            .collect()
    }

    pub(crate) fn add(&mut self, root: PathBuf) -> bool {
        if self
            .workspaces
            .iter()
            .any(|workspace| workspace.root == root)
        {
            return false;
        }
        let name = root
            .file_name()
            .filter(|name| !name.is_empty())
            .map_or_else(
                || root.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
        self.workspaces.push(Workspace {
            root,
            name,
            expanded: false,
            // A new workspace starts with no accent: the chooser's `Default`.
            accent: None,
            children: DirectoryLoad::Unread,
        });
        self.invalidate_rows();
        true
    }

    pub(crate) fn containing_root(&self, path: &Path) -> Option<&Path> {
        self.workspaces
            .iter()
            .map(|workspace| workspace.root.as_path())
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
    }

    pub(crate) fn accent_for(&self, root: &Path) -> Option<WorkspaceAccentKey> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.root == root)
            .and_then(|workspace| workspace.accent)
    }

    // Set (or clear, with `None`) one workspace's accent.
    //
    // Returns whether anything changed, so the caller writes the session only
    // when it did.
    pub(crate) fn set_accent(&mut self, root: &Path, accent: Option<WorkspaceAccentKey>) -> bool {
        let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.root == root)
        else {
            return false;
        };
        if workspace.accent == accent {
            return false;
        }
        workspace.accent = accent;
        // Every `&mut self` writer invalidates, even when rows are unchanged.
        self.invalidate_rows();
        true
    }

    pub(crate) fn remove(&mut self, root: &Path) -> bool {
        let before = self.workspaces.len();
        self.workspaces.retain(|workspace| workspace.root != root);
        if self.active == ActiveWorkspace::Named(root.to_path_buf()) {
            self.active = ActiveWorkspace::OpenFiles;
            OPEN_FILES_KEY.clone_into(&mut self.selected_key);
        }
        self.invalidate_rows();
        before != self.workspaces.len()
    }

    pub(crate) fn select_open_files(&mut self) {
        self.active = ActiveWorkspace::OpenFiles;
        OPEN_FILES_KEY.clone_into(&mut self.selected_key);
        self.invalidate_rows();
    }

    pub(crate) fn select_workspace(&mut self, root: &Path) -> bool {
        if self.active == ActiveWorkspace::Named(root.to_path_buf()) {
            return false;
        }
        if self
            .workspaces
            .iter()
            .any(|workspace| workspace.root == root)
        {
            self.active = ActiveWorkspace::Named(root.to_path_buf());
            self.selected_key = workspace_key(root);
            self.invalidate_rows();
            true
        } else {
            false
        }
    }

    pub(crate) fn select_file(&mut self, path: &Path) {
        self.selected_key = path_key(path);
        // Every `&mut self` writer invalidates, even when rows are unchanged.
        self.invalidate_rows();
    }

    // Flip a branch's `expanded` flag and, on its first expansion, mark its
    // children `Loading` and return the path to read.
    //
    // `Some(path)` must be read on the app's bounded blocking owner and
    // completed through [`apply_directory_load`](Self::apply_directory_load);
    // a network-mounted home can take up to 1 000 `symlink_metadata` calls.
    pub(crate) fn begin_toggle(&mut self, path: &Path) -> Option<PathBuf> {
        let pending = self.begin_toggle_inner(path);
        self.invalidate_rows();
        pending
    }

    fn begin_toggle_inner(&mut self, path: &Path) -> Option<PathBuf> {
        if let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.root == path)
        {
            workspace.expanded = !workspace.expanded;
            if workspace.expanded && matches!(workspace.children, DirectoryLoad::Unread) {
                workspace.children = DirectoryLoad::Loading;
                return Some(workspace.root.clone());
            }
            // Collapsing mid-read drops the eventual payload so a re-expand
            // reads afresh.
            if !workspace.expanded && matches!(workspace.children, DirectoryLoad::Loading) {
                workspace.children = DirectoryLoad::Unread;
            }
            return None;
        }
        for workspace in &mut self.workspaces {
            if let Some(pending) = begin_toggle_entry(&mut workspace.children, path) {
                return Some(pending);
            }
        }
        None
    }

    pub(crate) fn apply_directory_load(&mut self, path: &Path, load: DirectoryLoad) -> bool {
        let installed = self.apply_directory_load_inner(path, load);
        if installed {
            self.invalidate_rows();
        }
        installed
    }

    fn apply_directory_load_inner(&mut self, path: &Path, load: DirectoryLoad) -> bool {
        let mut carrier = Some(load);
        for workspace in &mut self.workspaces {
            if workspace.root == path {
                if matches!(workspace.children, DirectoryLoad::Loading)
                    && let Some(new_load) = carrier.take()
                {
                    workspace.children = new_load;
                    return true;
                }
                return false;
            }
            if apply_load_entry(&mut workspace.children, path, &mut carrier) {
                return true;
            }
            if carrier.is_none() {
                return false;
            }
        }
        false
    }

    pub(crate) fn rows(&self) -> std::rc::Rc<[TreeRow]> {
        if let Some(cached) = self.rows_cache.borrow().as_ref() {
            return std::rc::Rc::clone(cached);
        }
        // No "Open Files" row: the tab set is global and the context has no
        // children, so selecting it would change nothing visible.
        let mut rows: Vec<TreeRow> = Vec::new();
        for workspace in &self.workspaces {
            rows.push(TreeRow {
                key: workspace_key(&workspace.root),
                label: crate::safe_text::bounded_inline(&workspace.name),
                level: 0,
                has_children: true,
                expanded: workspace.expanded,
                icon: "folder",
                kind: TreeRowKind::Workspace(workspace.root.clone()),
            });
            if workspace.expanded {
                flatten_load(&workspace.children, &workspace.root, 1, &mut rows);
            }
        }
        let projection: std::rc::Rc<[TreeRow]> = rows.into();
        *self.rows_cache.borrow_mut() = Some(std::rc::Rc::clone(&projection));
        projection
    }

    pub(crate) fn selected_row(
        &self,
        rows: &[TreeRow],
        active_path: Option<&Path>,
    ) -> Option<usize> {
        active_path
            .and_then(|active| {
                rows.iter().position(
                    |row| matches!(&row.kind, TreeRowKind::File { path, .. } if path == active),
                )
            })
            .or_else(|| rows.iter().position(|row| row.key == self.selected_key))
    }

    pub(crate) fn active_empty_state(&self) -> Option<(String, String, &'static str)> {
        if self.workspaces.is_empty() {
            return Some((
                enzui_core::localization::text("text-no-workspaces-yet-97d0b1"),
                enzui_core::localization::text("text-add-a-folder-to-browse-its-files-2fa6b2"),
                "folder-plus",
            ));
        }
        let ActiveWorkspace::Named(root) = &self.active else {
            return None;
        };
        let workspace = self
            .workspaces
            .iter()
            .find(|workspace| &workspace.root == root)?;
        match &workspace.children {
            DirectoryLoad::Error(error) => Some((
                enzui_core::localization::text("text-folder-unavailable-72a50a"),
                // The folder name is attacker-influenced and the full path does
                // not belong in an error body; show one sanitized name.
                enzui_core::localized!("text-p0-p1-remembered-tabs-are-kept-8e4db2", "p0" => crate::safe_text::bounded_inline(&workspace.name), "p1" => error.kind().safe_reason()),
                "folder-x",
            )),
            DirectoryLoad::Loaded(entries, _) if entries.is_empty() => Some((
                enzui_core::localization::text("text-no-files-c219d9"),
                enzui_core::localization::text("text-this-folder-is-empty-bd88d7"),
                "folder-open",
            )),
            _ => None,
        }
    }
}

fn begin_toggle_entry(load: &mut DirectoryLoad, path: &Path) -> Option<PathBuf> {
    let DirectoryLoad::Loaded(entries, _) = load else {
        return None;
    };
    for entry in entries {
        if entry.path == path && entry.kind == EntryKind::Directory {
            entry.expanded = !entry.expanded;
            if entry.expanded && matches!(entry.children, DirectoryLoad::Unread) {
                entry.children = DirectoryLoad::Loading;
                return Some(entry.path.clone());
            }
            if !entry.expanded && matches!(entry.children, DirectoryLoad::Loading) {
                entry.children = DirectoryLoad::Unread;
            }
            return None;
        }
        if let Some(pending) = begin_toggle_entry(&mut entry.children, path) {
            return Some(pending);
        }
    }
    None
}

fn apply_load_entry(
    load: &mut DirectoryLoad,
    path: &Path,
    payload: &mut Option<DirectoryLoad>,
) -> bool {
    let DirectoryLoad::Loaded(entries, _) = load else {
        return false;
    };
    for entry in entries {
        if entry.path == path && entry.kind == EntryKind::Directory {
            if matches!(entry.children, DirectoryLoad::Loading)
                && let Some(new_load) = payload.take()
            {
                entry.children = new_load;
                return true;
            }
            return false;
        }
        if apply_load_entry(&mut entry.children, path, payload) {
            return true;
        }
    }
    false
}

// Reads one directory into an installable [`DirectoryLoad`].
//
// This is synchronous and must run on the app's bounded blocking owner.
pub(crate) fn read_directory(path: &Path) -> DirectoryLoad {
    let iter = match std::fs::read_dir(path) {
        Ok(iter) => iter,
        // Only the escaped log line carries the raw `io::Error` detail.
        Err(error) => {
            let classified = DirectoryError::classify(&error);
            eprintln!(
                "[text-app] directory listing failed: kind={:?} detail={}",
                classified.kind(),
                classified.escaped_diagnostic()
            );
            return DirectoryLoad::Error(classified);
        }
    };
    // Dotfiles are listed: in a workspace they are often the files to edit.
    let mut entries = Vec::new();
    for result in iter {
        let Ok(dir_entry) = result else { continue };
        let name = dir_entry.file_name().to_string_lossy().into_owned();
        let Ok(metadata) = std::fs::symlink_metadata(dir_entry.path()) else {
            continue;
        };
        let kind = if metadata.file_type().is_symlink() {
            EntryKind::Symlink
        } else if metadata.is_dir() {
            EntryKind::Directory
        } else {
            EntryKind::File
        };
        entries.push(TreeEntry {
            path: dir_entry.path(),
            name,
            kind,
            expanded: false,
            children: DirectoryLoad::Unread,
        });
    }
    entries.sort_by(|a, b| {
        let a_dir = a.kind == EntryKind::Directory;
        let b_dir = b.kind == EntryKind::Directory;
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let omitted = entries.len().saturating_sub(DIRECTORY_ENTRY_CAP);
    entries.truncate(DIRECTORY_ENTRY_CAP);
    DirectoryLoad::Loaded(entries, omitted)
}

fn flatten_load(load: &DirectoryLoad, workspace_root: &Path, level: u8, rows: &mut Vec<TreeRow>) {
    match load {
        DirectoryLoad::Unread | DirectoryLoad::Loading => {}
        DirectoryLoad::Error(error) => rows.push(TreeRow {
            key: format!("status:{}", workspace_root.display()),
            label: error.safe_label(),
            level,
            has_children: false,
            expanded: false,
            icon: "triangle-alert",
            kind: TreeRowKind::Status,
        }),
        DirectoryLoad::Loaded(entries, omitted) => {
            for entry in entries {
                let is_dir = entry.kind == EntryKind::Directory;
                let icon = match entry.kind {
                    EntryKind::Directory => "folder",
                    EntryKind::Symlink => "file-symlink",
                    EntryKind::File => "file",
                };
                rows.push(TreeRow {
                    key: path_key(&entry.path),
                    label: crate::safe_text::bounded_inline(&entry.name),
                    level,
                    has_children: is_dir,
                    expanded: entry.expanded,
                    icon,
                    kind: if is_dir {
                        TreeRowKind::Directory {
                            workspace: workspace_root.to_path_buf(),
                            path: entry.path.clone(),
                        }
                    } else {
                        TreeRowKind::File {
                            workspace: workspace_root.to_path_buf(),
                            path: entry.path.clone(),
                        }
                    },
                });
                if entry.expanded {
                    flatten_load(
                        &entry.children,
                        workspace_root,
                        level.saturating_add(1).min(12),
                        rows,
                    );
                }
            }
            if *omitted > 0 {
                rows.push(TreeRow {
                    key: format!("more:{}:{level}", workspace_root.display()),
                    label: enzui_core::localized!("text-and-omitted-more-0557de", "omitted" => omitted),
                    level,
                    has_children: false,
                    expanded: false,
                    icon: "ellipsis",
                    kind: TreeRowKind::Status,
                });
            }
        }
    }
}

pub(crate) fn workspace_key(root: &Path) -> String {
    format!("workspace:{}", root.display())
}

fn path_key(path: &Path) -> String {
    format!("path:{}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_selection_tracks_the_file_across_tab_changes_reorder_and_collapse() {
        let root = PathBuf::from("/workspace");
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        let mut state = WorkspaceState {
            selected_key: workspace_key(&root),
            ..WorkspaceState::default()
        };
        let mut rows = vec![TreeRow {
            key: workspace_key(&root),
            label: "workspace".into(),
            level: 0,
            has_children: true,
            expanded: true,
            icon: "folder",
            kind: TreeRowKind::Workspace(root.clone()),
        }];
        for path in [&first, &second] {
            rows.push(TreeRow {
                key: path_key(path),
                label: path.display().to_string(),
                level: 1,
                has_children: false,
                expanded: false,
                icon: "file",
                kind: TreeRowKind::File {
                    workspace: root.clone(),
                    path: path.clone(),
                },
            });
        }
        assert_eq!(state.selected_row(&rows, Some(&first)), Some(1));
        assert_eq!(state.selected_row(&rows, Some(&second)), Some(2));
        rows.swap(1, 2);
        assert_eq!(state.selected_row(&rows, Some(&first)), Some(2));
        state.select_file(&first);
        rows.truncate(1);
        assert_eq!(state.selected_row(&rows, Some(&first)), None);
        assert_eq!(state.selected_row(&rows, None), None);
    }
}
