//! Bounded session loading: byte and path budgets are accounted before anything is
//! retained, and a path replaced between read and settlement is refused as a whole.

use std::path::Path;

use crate::workspace::{PersistedTab, PersistedTabs};

use super::{
    FORMAT_VERSION, LoadError, PublishOverBoundSummary, RestoreGuard, SessionLimits,
    SessionRefusal, SessionSnapshot, VersionProbe, bounded_de, parse_legacy_bounded,
};
#[cfg(test)]
use super::{FileIdentity, seam_after_read};
#[cfg(test)]
use super::{path_still_names, read_bounded};

fn account_bytes(
    bytes: usize,
    aggregate: u64,
    limits: SessionLimits,
) -> Result<u64, SessionRefusal> {
    if bytes > limits.max_path_bytes {
        return Err(SessionRefusal::PathBudget);
    }
    let next = aggregate.saturating_add(bytes as u64);
    if next > limits.max_aggregate_path_bytes {
        return Err(SessionRefusal::PathBudget);
    }
    Ok(next)
}

pub(super) fn account_path(
    path: &Path,
    aggregate: u64,
    limits: SessionLimits,
) -> Result<u64, SessionRefusal> {
    account_bytes(path.as_os_str().len(), aggregate, limits)
}

// Check the projection against publication bounds without opening a worker.
//
// Returns a [`PublishOverBoundSummary`] on refusal so a caller may hold a
// bounded typed debt in place of the projection.
pub(crate) fn preflight_bounds(snapshot: &SessionSnapshot) -> Result<(), PublishOverBoundSummary> {
    match enforce_snapshot_bounds(snapshot, SessionLimits::PRODUCTION) {
        Ok(()) => Ok(()),
        Err(refusal) => Err(summarize_over_bound(snapshot, refusal)),
    }
}

pub(super) fn summarize_over_bound(
    snapshot: &SessionSnapshot,
    reason: SessionRefusal,
) -> PublishOverBoundSummary {
    let mut aggregate: u64 = 0;
    for tab in &snapshot.tabs.tabs {
        aggregate = aggregate.saturating_add(tab.path.as_os_str().len() as u64);
        if let Some(ws) = &tab.workspace {
            aggregate = aggregate.saturating_add(ws.as_os_str().len() as u64);
        }
    }
    if let Some(active) = &snapshot.tabs.active_path {
        aggregate = aggregate.saturating_add(active.as_os_str().len() as u64);
    }
    for ws in &snapshot.workspaces {
        aggregate = aggregate.saturating_add(ws.root.as_os_str().len() as u64);
        aggregate = aggregate.saturating_add(ws.name.len() as u64);
    }
    PublishOverBoundSummary {
        tabs: snapshot.tabs.tabs.len(),
        workspaces: snapshot.workspaces.len(),
        aggregate_path_bytes: aggregate,
        reason,
    }
}

pub(super) fn enforce_snapshot_bounds(
    snapshot: &SessionSnapshot,
    limits: SessionLimits,
) -> Result<(), SessionRefusal> {
    if snapshot.tabs.tabs.len() > limits.max_tabs {
        return Err(SessionRefusal::TooManyEntries);
    }
    if snapshot.workspaces.len() > limits.max_workspaces {
        return Err(SessionRefusal::TooManyEntries);
    }
    let mut aggregate: u64 = 0;
    for tab in &snapshot.tabs.tabs {
        aggregate = account_path(&tab.path, aggregate, limits)?;
        if let Some(ws) = &tab.workspace {
            aggregate = account_path(ws, aggregate, limits)?;
        }
    }
    if let Some(active) = &snapshot.tabs.active_path {
        aggregate = account_path(active, aggregate, limits)?;
    }
    for ws in &snapshot.workspaces {
        aggregate = account_path(&ws.root, aggregate, limits)?;
        // The display name is decoded bytes retained independently of the path,
        // so it is bounded on the same budget.
        aggregate = account_bytes(ws.name.len(), aggregate, limits)?;
    }
    Ok(())
}

pub(super) fn parse_snapshot(
    text: &str,
    home: Option<&Path>,
    limits: SessionLimits,
) -> Result<SessionSnapshot, LoadError> {
    if !text.trim_start().starts_with('{') {
        let paths = parse_legacy_bounded(text, home, limits).map_err(LoadError::Refused)?;
        let active_path = paths.first().cloned();
        return Ok(SessionSnapshot {
            version: FORMAT_VERSION,
            tabs: PersistedTabs {
                tabs: paths
                    .into_iter()
                    .map(|path| PersistedTab {
                        path,
                        workspace: None,
                    })
                    .collect(),
                active_path,
            },
            workspaces: Vec::new(),
            restore_guard: RestoreGuard::default(),
        });
    }

    let probe: VersionProbe =
        serde_json::from_str(text).map_err(|_| LoadError::Refused(SessionRefusal::Malformed))?;
    if probe.version > FORMAT_VERSION {
        return Err(LoadError::Future(probe.version));
    }
    // Budgets are charged during decode; this pass is a redundant check.
    let snapshot =
        bounded_de::parse_json_bounded(text, probe.version, limits).map_err(LoadError::Refused)?;
    enforce_snapshot_bounds(&snapshot, limits).map_err(LoadError::Refused)?;
    Ok(snapshot)
}

#[cfg(test)]
pub(super) fn load_with_limits(
    path: &Path,
    home: Option<&Path>,
    limits: SessionLimits,
) -> Result<(SessionSnapshot, FileIdentity), LoadError> {
    let (text, identity) = read_bounded(path, limits.max_file_bytes)?;
    let snapshot = parse_snapshot(&text, home, limits)?;
    // Bind the returned snapshot to the inode that was read: a path replaced (or
    // turned into a symlink or non-regular file) between the read and settlement
    // is refused rather than trusted.
    seam_after_read();
    if !path_still_names(path, identity) {
        return Err(LoadError::Refused(SessionRefusal::Replaced));
    }
    Ok((snapshot, identity))
}
