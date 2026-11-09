//! Bounded, single-line rendering of text the user's filesystem chose.
//!
//! File and folder names can carry newlines, `NUL`, ANSI escapes, bidi
//! overrides or kilobytes of padding, so every name spliced into user-facing
//! copy goes through here. The output is for display only, not escaping; the
//! unmodified name stays in the model and diagnostics.

pub(crate) const MAX_INLINE_CHARS: usize = 48;

// Names are bounded display fragments; original paths retain filesystem authority.
#[must_use]
pub(crate) fn bounded_inline(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control() && !is_line_separator(*c) && !is_bidi_control(*c))
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return enzui_core::localization::text("text-this-item-545362");
    }
    if cleaned.chars().count() <= MAX_INLINE_CHARS {
        return cleaned.to_owned();
    }
    let kept: String = cleaned.chars().take(MAX_INLINE_CHARS - 1).collect();
    format!("{kept}\u{2026}")
}

#[must_use]
pub(crate) fn bounded_path(path: &std::path::Path) -> String {
    let name = path.file_name().unwrap_or(path.as_os_str());
    bounded_inline(&name.to_string_lossy())
}

fn is_line_separator(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}')
}

fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}
