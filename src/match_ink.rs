//! The editor's find-match paint, derived once per frame.
//!
//! A painted match is a fill quad (`text_editor::background_highlights`) plus
//! the glyph ink over it; syntax ink left unchanged on a match fill can fall
//! below the contrast floor. Both halves come from one
//! `SearchState::matches_in_lines` pass so they always agree on which match is
//! current. Fill colours belong to `FindMatchTokens::from_palette` and ink to
//! [`enzui_widgets::match_ink::format_for_editor_ink`]; this module
//! produces only the visible-viewport subset.

use std::ops::Range;

use enzui_theme::{FindMatchTokens, MatchEmphasis, Palette};
use enzui_widgets::match_ink::MatchRun;
use iced::Color;
use iced::widget::text_editor::Position;

use crate::search::SearchState;

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct MatchPaint {
    pub fills: Vec<(Range<Position>, Color)>,
    pub runs: Vec<MatchRun>,
}

#[must_use]
pub(crate) fn match_paint(
    search: &SearchState,
    document: &enzui_core::chunked_text::TextSnapshot,
    line_origin: usize,
    window: Range<usize>,
    palette: &Palette,
) -> MatchPaint {
    let mut paint = MatchPaint::default();

    if !search.open || search.matches.is_empty() {
        return paint;
    }

    // Reject empty windows and ranges above the buffer: saturating subtraction
    // would otherwise map them to relative line zero and a phantom run.
    if window.start >= window.end {
        return paint;
    }
    if window.end <= line_origin {
        return paint;
    }

    let tokens = FindMatchTokens::from_palette(palette);
    // Inclusive last paintable line, relative to the buffer; bounds runs for
    // unbounded multi-line matches.
    let window_last = window.end.saturating_sub(line_origin).saturating_sub(1);

    for (index, start, end) in search.matches_in_lines(document, window) {
        let Some(last_line) = end.line.checked_sub(line_origin) else {
            continue;
        };

        // A match may begin above the window; its unmaterialized leading lines
        // clamp to buffer line zero while the real end is kept.
        let starts_above_buffer = start.line < line_origin;
        let first_line = start.line.saturating_sub(line_origin);
        let first_column = if starts_above_buffer { 0 } else { start.index };

        // A match starting at `window.end` belongs to the next window.
        if first_line > window_last {
            continue;
        }

        let emphasis = if index == search.current {
            MatchEmphasis::Active
        } else {
            MatchEmphasis::Inactive
        };

        paint.fills.push((
            Position {
                line: first_line,
                index: first_column,
            }..Position {
                line: last_line,
                index: end.index,
            },
            tokens.fill(emphasis),
        ));

        let last_painted = last_line.min(window_last);

        for line in first_line..=last_painted {
            paint.runs.push(MatchRun {
                line,
                // Byte columns, as `Position::column` is; the highlighter clamps
                // to char boundaries.
                start: if line == first_line { first_column } else { 0 },
                end: if line == last_line {
                    end.index
                } else {
                    usize::MAX
                },
                emphasis,
            });
        }
    }

    paint
}
