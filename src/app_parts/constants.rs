//! TextApp layout and timing constants.

use super::*;

// A timer, not `window::frames()`: each watch message runs an update, so a
// per-frame watch would never idle.
pub(crate) const FILE_WATCH_PROBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
// Nobody is looking at an unfocused window; refocusing probes at once.
pub(crate) const FILE_WATCH_UNFOCUSED_PROBE_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(5);
// A drag emits pointer events at display rate; the worker counts once after
// the gesture settles.
pub(crate) const SELECTION_STATS_DEBOUNCE: std::time::Duration =
    std::time::Duration::from_millis(150);

pub(crate) const RECOVERY_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(3);

pub(crate) const PRINT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
// How long a non-error print outcome toast stays: the framework `ToastHost`
// default dwell (slow motion token x 20). Errors stay until dismissed.
pub(crate) const PRINT_STATUS_DWELL: std::time::Duration = std::time::Duration::from_millis(5200);
pub(crate) const LOCATION_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);

// Logical inset shared by editor paint, input and its integrated gutter.
pub(crate) const EDITOR_INTERNAL_PAD: f32 = 5.0;
// Editor line-height factor; integrated numbering uses the same shaped row heads.
pub(crate) const EDITOR_LINE_HEIGHT: f32 = 1.3;

// Overflow rows fill their height limit; an unbounded bar would consume the editor.
pub(crate) const BAR_HEIGHT: f32 = 34.0;

pub(crate) const FONT_SCALES: [(&str, FontSizeScale); 3] = [
    ("Small", FontSizeScale::Small),
    ("Medium", FontSizeScale::Medium),
    ("Large", FontSizeScale::Large),
];

pub(crate) const SIDEBAR_WIDTH_PX: f32 = 240.0;
pub(crate) const EDITOR_PANE_MIN_WIDTH: f32 = 560.0;
