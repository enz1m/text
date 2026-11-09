//! The desktop's Search: open documents by file name.
//!
//! The provider answers from a snapshot of the open tabs that the reducer
//! refreshes when their files change, so a query never touches the model,
//! the filesystem or the window. A result id is the document's path; opening
//! it focuses that tab, or reopens the file when the tab has since closed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use enzui_core::activation::dbus::search::{
    SearchProvider, SearchQuery, SearchRequest, SearchResult,
};

use crate::model::Tab;

const RESULT_ICON: &str = "text-x-generic";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    id: String,
    title: String,
    folded: String,
    folder: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SearchIndex {
    entries: Arc<Mutex<Vec<Entry>>>,
    paths: Vec<PathBuf>,
}

impl SearchIndex {
    // Cheap when nothing changed: the snapshot is rebuilt only when the set
    // or order of open documents differs from the last refresh.
    pub(crate) fn refresh(&mut self, tabs: &[Tab], active: &str, home: Option<&Path>) {
        let paths = ordered_paths(tabs, active);
        if paths == self.paths {
            return;
        }
        *lock(&self.entries) = paths.iter().filter_map(|path| entry(path, home)).collect();
        self.paths = paths;
    }

    #[cfg(test)]
    pub(crate) fn ids(&self, text: &str) -> Vec<String> {
        let query = SearchQuery {
            text: text.to_owned(),
            limit: enzui_core::activation::dbus::search::MAX_SEARCH_RESULTS,
            locale: None,
        };
        search(&lock(&self.entries), &query)
            .iter()
            .map(|result| result.id().to_owned())
            .collect()
    }

    pub(crate) fn provider(&self) -> SearchProvider {
        let entries = Arc::clone(&self.entries);
        SearchProvider::blocking(move |query| search(&lock(&entries), query))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// The active document first, then the rest in tab order.
fn ordered_paths(tabs: &[Tab], active: &str) -> Vec<PathBuf> {
    let (first, rest): (Vec<&Tab>, Vec<&Tab>) = tabs.iter().partition(|tab| tab.id == active);
    first
        .into_iter()
        .chain(rest)
        .filter_map(|tab| tab.file_path.clone())
        .collect()
}

// A path that cannot round-trip through a result id is left out rather than
// answered with an id that would open a different file.
fn entry(path: &Path, home: Option<&Path>) -> Option<Entry> {
    let id = path.to_str()?.to_owned();
    let title = path.file_name()?.to_string_lossy().into_owned();
    SearchResult::new(id.clone(), title.clone()).ok()?;
    Some(Entry {
        path: path.to_path_buf(),
        id,
        folded: enzui_core::text_fold::fold(&title),
        title,
        folder: folder_label(path.parent()?, home),
    })
}

fn folder_label(folder: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| folder.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => folder.display().to_string(),
    }
}

fn search(entries: &[Entry], query: &SearchQuery) -> Vec<SearchResult> {
    let wanted = enzui_core::text_fold::fold(query.text.trim());
    if wanted.is_empty() {
        return Vec::new();
    }
    entries
        .iter()
        .filter(|entry| entry.folded.contains(&wanted))
        .filter_map(|entry| {
            SearchResult::new(entry.id.clone(), entry.title.clone())
                .ok()
                .map(|result| result.subtitle(entry.folder.clone()).icon(RESULT_ICON))
        })
        .take(query.limit)
        .collect()
}

// What a Search choice asks of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SearchChoice {
    Open(PathBuf),
    Find(String),
}

pub(crate) fn choice(request: SearchRequest) -> SearchChoice {
    match request {
        SearchRequest::Open { id, .. } => SearchChoice::Open(PathBuf::from(id)),
        SearchRequest::Search { query } => SearchChoice::Find(query),
    }
}

#[cfg(test)]
mod tests {}
