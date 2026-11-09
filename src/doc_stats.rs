//! Document statistics over contiguous text and chunked snapshots.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    pub chars: usize,
    pub chars_no_spaces: usize,
    pub words: usize,
    pub lines: usize,
    pub bytes: usize,
}

impl Stats {
    pub(crate) fn reading_time(&self) -> (usize, usize) {
        let total_seconds = (self.words * 3) / 10;
        (total_seconds / 60, total_seconds % 60)
    }

    pub(crate) fn reading_time_label(&self) -> String {
        let (minutes, seconds) = self.reading_time();
        if self.words > 0 && minutes == 0 && seconds == 0 {
            return enzui_core::localization::text("text-less-than-a-second-8e6483");
        }
        let message = if minutes == 0 {
            "text-reading-seconds-a0811d"
        } else if seconds == 0 {
            "text-reading-minutes-5f1f4c"
        } else {
            "text-reading-duration-1efdf0"
        };
        let mut arguments = enzui_core::localization::Arguments::new();
        arguments.set("minutes", minutes);
        arguments.set("seconds", seconds);
        enzui_core::localization::text_with(message, &arguments)
    }
}

pub(crate) fn document_stats(text: &str) -> Stats {
    let mut chars = 0usize;
    let mut chars_no_spaces = 0usize;
    for c in text.chars() {
        chars += 1;
        if !c.is_whitespace() {
            chars_no_spaces += 1;
        }
    }
    let words = text.split_whitespace().count();
    // A trailing newline opens a final line for the caret.
    let lines = crate::doc_index::line_count_of(text);
    let bytes = text.len();
    Stats {
        chars,
        chars_no_spaces,
        words,
        lines,
        bytes,
    }
}

// Count a persistent snapshot chunk by chunk without a contiguous copy.
pub(crate) fn document_stats_snapshot(snapshot: &enzui_core::chunked_text::TextSnapshot) -> Stats {
    let mut chars = 0;
    let mut chars_no_spaces = 0;
    let mut words = 0;
    let mut in_word = false;
    snapshot.for_each_chunk(|chunk| {
        for ch in chunk.chars() {
            chars += 1;
            let whitespace = ch.is_whitespace();
            if !whitespace {
                chars_no_spaces += 1;
                if !in_word {
                    words += 1;
                }
            }
            in_word = !whitespace;
        }
    });
    Stats {
        chars,
        chars_no_spaces,
        words,
        lines: snapshot.line_count(),
        bytes: snapshot.len(),
    }
}
