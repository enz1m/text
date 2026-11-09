//! Test counters for whole-document work during bounded editing.

use std::cell::Cell;

thread_local! {
    static SNAPSHOT_FLATTENS: Cell<u64> = const { Cell::new(0) };
    static SNAPSHOT_MATERIALIZATIONS: Cell<u64> = const { Cell::new(0) };
}

pub(crate) fn bump_snapshot_flatten() {
    let _ = SNAPSHOT_FLATTENS.try_with(|c| c.set(c.get() + 1));
}

pub(crate) fn bump_snapshot_materialization() {
    let _ = SNAPSHOT_MATERIALIZATIONS.try_with(|c| c.set(c.get() + 1));
}

pub(crate) fn snapshot_flattens() -> u64 {
    SNAPSHOT_FLATTENS.with(Cell::get)
}

pub(crate) fn snapshot_materializations() -> u64 {
    SNAPSHOT_MATERIALIZATIONS.with(Cell::get)
}
