//! Character, word and line counts for contiguous and chunked text, plus
//! language detection.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextStats {
    pub chars: usize,
    pub words: usize,
    pub lines: usize,
}

// Count a persistent snapshot without flattening it.
pub(crate) fn text_stats_snapshot(snapshot: &enzui_core::chunked_text::TextSnapshot) -> TextStats {
    let mut chars = 0;
    let mut words = 0;
    let mut in_word = false;
    snapshot.for_each_chunk(|chunk| {
        for ch in chunk.chars() {
            chars += 1;
            let whitespace = ch.is_whitespace();
            if !whitespace && !in_word {
                words += 1;
            }
            in_word = !whitespace;
        }
    });
    TextStats {
        chars,
        words,
        lines: snapshot.line_count(),
    }
}

pub(crate) fn detect_language(path: Option<&std::path::Path>) -> Option<&'static str> {
    let ext = path?.extension()?.to_str()?;
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some("Rust"),
        "json" => Some("JSON"),
        "toml" => Some("TOML"),
        "yaml" | "yml" => Some("YAML"),
        "md" | "markdown" => Some("Markdown"),
        "sh" | "bash" | "zsh" => Some("Shell"),
        "c" => Some("C"),
        "cpp" | "cc" | "cxx" => Some("C++"),
        "py" => Some("Python"),
        "txt" => Some("Plain Text"),
        "html" | "htm" => Some("HTML"),
        "css" => Some("CSS"),
        "js" => Some("JavaScript"),
        "ts" => Some("TypeScript"),
        _ => None,
    }
}
