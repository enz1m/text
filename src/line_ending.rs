//! Line-ending detection, conversion, and localized display labels.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineEnding {
    Lf,
    Crlf,
    Cr,
    Mixed,
}

impl LineEnding {
    pub(crate) fn display_label(self) -> String {
        match self {
            LineEnding::Lf => enzui_core::localization::text("text-lf-unix-1d9ec2"),
            LineEnding::Crlf => enzui_core::localization::text("text-crlf-windows-adb850"),
            LineEnding::Cr => enzui_core::localization::text("text-cr-classic-mac-431c3a"),
            LineEnding::Mixed => enzui_core::localization::text("text-mixed-f340b6"),
        }
    }

    pub(crate) const fn written_as(self) -> LineEnding {
        match self {
            LineEnding::Crlf => LineEnding::Crlf,
            LineEnding::Cr => LineEnding::Cr,
            LineEnding::Lf | LineEnding::Mixed => LineEnding::Lf,
        }
    }
}

// Classify a document's dominant line-break style with one byte-wise scan.
//
// `Cr` requires at least one bare `\r` and no LF. A document with no line
// breaks is `Lf`.
pub(crate) fn detect_dominant(text: &str) -> LineEnding {
    let bytes = text.as_bytes();
    let mut crlf = 0usize;
    let mut bare_cr = 0usize;
    let mut bare_lf = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                    crlf += 1;
                    i += 2;
                    continue;
                }
                bare_cr += 1;
            }
            b'\n' => bare_lf += 1,
            _ => {}
        }
        i += 1;
    }
    match (crlf, bare_cr, bare_lf) {
        (0, 0, 0) => LineEnding::Lf,
        (c, 0, 0) if c > 0 => LineEnding::Crlf,
        (0, r, 0) if r > 0 => LineEnding::Cr,
        (0, 0, _) => LineEnding::Lf,
        _ => LineEnding::Mixed,
    }
}

pub(crate) fn normalize(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    let _ = chars.next();
                }
                out.push('\n');
            }
            '\n' => {
                if chars.peek() == Some(&'\r') {
                    let _ = chars.next();
                }
                out.push('\n');
            }
            other => out.push(other),
        }
    }
    out
}

// Normalize an owned load buffer without retaining a second whole-document
// copy when it already uses LF endings.
//
// A CR-bearing source needs one replacement allocation; `raw` is released
// before the editor copies the result.
pub(crate) fn load_normalized_owned(raw: String) -> (LineEnding, String) {
    let detected = detect_dominant(&raw);
    let text = if raw.contains('\r') {
        normalize(&raw)
    } else {
        raw
    };
    (detected, text)
}
