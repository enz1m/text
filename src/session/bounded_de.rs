//! Bounded JSON admission for session restore.
//!
//! Count and byte budgets are charged during decoding and while a v2 workspace
//! root is cloned onto each tab, so a sub-cap file can neither exceed the entry
//! caps nor amplify one long root before the file is refused.

use std::cell::RefCell;
use std::path::PathBuf;

use serde::Deserialize;
use serde::de::{self, Deserializer, Visitor};

use super::{
    FORMAT_VERSION, PersistedTab, PersistedTabs, PersistedWorkspace, RestoreGuard, SessionLimits,
    SessionRefusal, SessionSnapshot,
};

struct Budget {
    tabs_left: usize,
    workspaces_left: usize,
    aggregate_left: u64,
    max_path_bytes: usize,
    refusal: Option<SessionRefusal>,
}

impl Budget {
    fn inert() -> Self {
        Self {
            tabs_left: 0,
            workspaces_left: 0,
            aggregate_left: 0,
            max_path_bytes: 0,
            refusal: None,
        }
    }
}

thread_local! {
    static BUDGET: RefCell<Budget> = RefCell::new(Budget::inert());
}

fn guard_tab() -> bool {
    BUDGET.with(|b| {
        let mut b = b.borrow_mut();
        if b.tabs_left == 0 {
            b.refusal.get_or_insert(SessionRefusal::TooManyEntries);
            false
        } else {
            b.tabs_left -= 1;
            true
        }
    })
}

fn guard_workspace() -> bool {
    BUDGET.with(|b| {
        let mut b = b.borrow_mut();
        if b.workspaces_left == 0 {
            b.refusal.get_or_insert(SessionRefusal::TooManyEntries);
            false
        } else {
            b.workspaces_left -= 1;
            true
        }
    })
}

fn guard_aggregate(len: usize) -> bool {
    BUDGET.with(|b| {
        let mut b = b.borrow_mut();
        if len > b.max_path_bytes {
            b.refusal.get_or_insert(SessionRefusal::PathBudget);
            return false;
        }
        match b.aggregate_left.checked_sub(len as u64) {
            Some(rest) => {
                b.aggregate_left = rest;
                true
            }
            None => {
                b.refusal.get_or_insert(SessionRefusal::PathBudget);
                false
            }
        }
    })
}

// A path string charged against the aggregate byte budget. Does not count a tab.
struct BPath(PathBuf);

impl<'de> Deserialize<'de> for BPath {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(BPath(PathBuf::from(d.deserialize_str(StringVisitor)?)))
    }
}

// A workspace name (or accent id) charged against the aggregate byte budget.
struct BName(String);

impl<'de> Deserialize<'de> for BName {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(BName(d.deserialize_str(StringVisitor)?))
    }
}

struct StringVisitor;

impl<'de> Visitor<'de> for StringVisitor {
    type Value = String;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a bounded string")
    }

    fn visit_str<E>(self, v: &str) -> Result<String, E>
    where
        E: de::Error,
    {
        if !guard_aggregate(v.len()) {
            return Err(E::custom("session byte budget exceeded"));
        }
        Ok(v.to_owned())
    }

    fn visit_string<E>(self, v: String) -> Result<String, E>
    where
        E: de::Error,
    {
        if !guard_aggregate(v.len()) {
            return Err(E::custom("session byte budget exceeded"));
        }
        Ok(v)
    }
}

// A path string that also counts one tab. The count and bytes are guarded before
// the owned path is retained.
struct BTabPath(PathBuf);

impl<'de> Deserialize<'de> for BTabPath {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(BTabPath(PathBuf::from(d.deserialize_str(TabPathVisitor)?)))
    }
}

struct TabPathVisitor;

impl<'de> Visitor<'de> for TabPathVisitor {
    type Value = String;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a bounded tab path")
    }

    fn visit_str<E>(self, v: &str) -> Result<String, E>
    where
        E: de::Error,
    {
        if !guard_tab() {
            return Err(E::custom("session tab budget exceeded"));
        }
        if !guard_aggregate(v.len()) {
            return Err(E::custom("session byte budget exceeded"));
        }
        Ok(v.to_owned())
    }

    fn visit_string<E>(self, v: String) -> Result<String, E>
    where
        E: de::Error,
    {
        if !guard_tab() {
            return Err(E::custom("session tab budget exceeded"));
        }
        if !guard_aggregate(v.len()) {
            return Err(E::custom("session byte budget exceeded"));
        }
        Ok(v)
    }
}

#[derive(Deserialize)]
struct V1 {
    #[serde(default)]
    paths: Vec<BTabPath>,
    #[serde(default)]
    active_path: Option<BPath>,
}

#[derive(Deserialize, Default)]
struct V2TabSet {
    #[serde(default)]
    paths: Vec<BTabPath>,
    #[serde(default)]
    active_path: Option<BPath>,
}

#[derive(Deserialize)]
struct V2WsInner {
    root: BPath,
    name: BName,
    #[serde(default)]
    tabs: V2TabSet,
}

struct V2WsElem(V2WsInner);

impl<'de> Deserialize<'de> for V2WsElem {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if !guard_workspace() {
            return Err(<D::Error as de::Error>::custom(
                "session workspace budget exceeded",
            ));
        }
        Ok(V2WsElem(V2WsInner::deserialize(d)?))
    }
}

#[derive(Deserialize)]
struct V2 {
    #[serde(default)]
    open_files: V2TabSet,
    #[serde(default)]
    workspaces: Vec<V2WsElem>,
}

#[derive(Deserialize)]
struct V3TabInner {
    path: BPath,
    #[serde(default)]
    workspace: Option<BPath>,
}

struct V3TabElem(V3TabInner);

impl<'de> Deserialize<'de> for V3TabElem {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if !guard_tab() {
            return Err(<D::Error as de::Error>::custom(
                "session tab budget exceeded",
            ));
        }
        Ok(V3TabElem(V3TabInner::deserialize(d)?))
    }
}

#[derive(Deserialize)]
struct V3WsInner {
    root: BPath,
    name: BName,
    #[serde(default)]
    accent: Option<BName>,
}

struct V3WsElem(V3WsInner);

impl<'de> Deserialize<'de> for V3WsElem {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if !guard_workspace() {
            return Err(<D::Error as de::Error>::custom(
                "session workspace budget exceeded",
            ));
        }
        Ok(V3WsElem(V3WsInner::deserialize(d)?))
    }
}

#[derive(Deserialize, Default)]
struct V3Tabs {
    #[serde(default)]
    tabs: Vec<V3TabElem>,
    #[serde(default)]
    active_path: Option<BPath>,
}

#[derive(Deserialize)]
struct V3 {
    #[serde(default)]
    tabs: V3Tabs,
    #[serde(default)]
    workspaces: Vec<V3WsElem>,
}

// Decode a versioned JSON session under bounded admission.
//
// Returns the migrated snapshot, or the typed refusal recorded when a count or
// byte budget was crossed (defaulting to [`SessionRefusal::Malformed`] for a
// syntax or shape error).
pub(super) fn parse_json_bounded(
    text: &str,
    version: u32,
    limits: SessionLimits,
) -> Result<SessionSnapshot, SessionRefusal> {
    set_budget(limits);
    let decoded = decode(text, version);
    let refusal = BUDGET.with(|b| b.borrow().refusal);
    clear_budget();
    match decoded {
        Ok(snapshot) => Ok(snapshot),
        Err(()) => Err(refusal.unwrap_or(SessionRefusal::Malformed)),
    }
}

fn decode(text: &str, version: u32) -> Result<SessionSnapshot, ()> {
    match version {
        1 => {
            let v1: V1 = serde_json::from_str(text).map_err(|_| ())?;
            Ok(SessionSnapshot {
                version: FORMAT_VERSION,
                tabs: PersistedTabs {
                    tabs: v1
                        .paths
                        .into_iter()
                        .map(|p| PersistedTab {
                            path: p.0,
                            workspace: None,
                        })
                        .collect(),
                    active_path: v1.active_path.map(|p| p.0),
                },
                workspaces: Vec::new(),
                restore_guard: RestoreGuard::default(),
            })
        }
        2 => decode_v2(text),
        3 => decode_v3(text),
        _ => Err(()),
    }
}

// Fold each v2 per-workspace tab set into the one global set: every remembered
// path becomes a global tab tagged with its workspace root; the active path
// prefers the Open Files active, then the first workspace that named one.
fn decode_v2(text: &str) -> Result<SessionSnapshot, ()> {
    let v2: V2 = serde_json::from_str(text).map_err(|_| ())?;
    let mut tabs: Vec<PersistedTab> = v2
        .open_files
        .paths
        .into_iter()
        .map(|p| PersistedTab {
            path: p.0,
            workspace: None,
        })
        .collect();
    let mut active_path = v2.open_files.active_path.map(|p| p.0);
    let mut workspaces = Vec::new();
    for ws in v2.workspaces {
        let ws = ws.0;
        let root = ws.root.0;
        let root_bytes = root.as_os_str().len();
        for tab in ws.tabs.paths {
            // Each association clones the root into an owned tab, so charge the
            // clone before it is retained.
            if !guard_aggregate(root_bytes) {
                return Err(());
            }
            tabs.push(PersistedTab {
                path: tab.0,
                workspace: Some(root.clone()),
            });
        }
        if active_path.is_none() {
            active_path = ws.tabs.active_path.map(|p| p.0);
        }
        workspaces.push(PersistedWorkspace {
            root,
            name: ws.name.0,
            accent: None,
        });
    }
    Ok(SessionSnapshot {
        version: FORMAT_VERSION,
        tabs: PersistedTabs { tabs, active_path },
        workspaces,
        restore_guard: RestoreGuard::default(),
    })
}

fn decode_v3(text: &str) -> Result<SessionSnapshot, ()> {
    let v3: V3 = serde_json::from_str(text).map_err(|_| ())?;
    let tabs = v3
        .tabs
        .tabs
        .into_iter()
        .map(|t| {
            let t = t.0;
            PersistedTab {
                path: t.path.0,
                workspace: t.workspace.map(|w| w.0),
            }
        })
        .collect();
    let workspaces = v3
        .workspaces
        .into_iter()
        .map(|w| {
            let w = w.0;
            PersistedWorkspace {
                root: w.root.0,
                name: w.name.0,
                accent: w.accent.map(|a| a.0),
            }
        })
        .collect();
    Ok(SessionSnapshot {
        version: FORMAT_VERSION,
        tabs: PersistedTabs {
            tabs,
            active_path: v3.tabs.active_path.map(|p| p.0),
        },
        workspaces,
        restore_guard: RestoreGuard::default(),
    })
}

fn set_budget(limits: SessionLimits) {
    BUDGET.with(|b| {
        *b.borrow_mut() = Budget {
            tabs_left: limits.max_tabs,
            workspaces_left: limits.max_workspaces,
            aggregate_left: limits.max_aggregate_path_bytes,
            max_path_bytes: limits.max_path_bytes,
            refusal: None,
        }
    });
}

fn clear_budget() {
    BUDGET.with(|b| *b.borrow_mut() = Budget::inert());
}
