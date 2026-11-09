//! TextApp gutter and line-window helpers.

use super::*;

// Highlight geometry must be limited to the editor viewport to bound per-frame work.
pub(crate) fn visible_line_window(
    content: &text_editor::Content,
    line_origin: usize,
    total_lines: usize,
    row_px: f32,
) -> std::ops::Range<usize> {
    const OVERSCAN_LINES: usize = 24;

    let metrics = content.scroll_metrics();
    let row_px = if row_px.is_finite() && row_px > 0.0 {
        row_px
    } else {
        1.0
    };
    if metrics.viewport.height <= 0.0 {
        // Before the first layout (or after a tab switch or rebuild), use a
        // bounded window seeded at the caret rather than every match.
        const FIRST_FRAME_LINES: usize = 128;

        let caret = content.cursor().position.line.saturating_add(line_origin);
        let first = caret.saturating_sub(FIRST_FRAME_LINES / 2);
        return first.min(total_lines)
            ..first
                .saturating_add(FIRST_FRAME_LINES)
                .min(total_lines)
                .max(first);
    }

    let first = metrics
        .first_line
        .saturating_add(line_origin)
        .saturating_sub(OVERSCAN_LINES);
    let visible = ((metrics.viewport.height / row_px).ceil() as usize)
        .saturating_add(2 * OVERSCAN_LINES)
        .max(1);

    let start = first.min(total_lines);
    start
        ..first
            .saturating_add(visible)
            .saturating_add(1)
            .min(total_lines)
            .max(start)
}
