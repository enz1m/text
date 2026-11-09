//! Search replace operations.

use super::*;

// Stale matches and oversized outputs are refused before allocation; captures use the complete haystack.
pub(crate) fn replace_one(
    haystack: &str,
    m: Match,
    query: &str,
    replacement: &str,
    opts: SearchOptions,
) -> Result<(String, ReplaceSpan), String> {
    if replacement.len() > MAX_REPLACEMENT_BYTES {
        return Err(enzui_core::localization::text(
            "text-replacement-is-too-long-dd8872",
        ));
    }
    if m.start >= m.end
        || m.end > haystack.len()
        || !haystack.is_char_boundary(m.start)
        || !haystack.is_char_boundary(m.end)
    {
        return Err(enzui_core::localization::text("text-stale-match-3b336e"));
    }
    let re = compile_regex(query, opts)?;
    let Some(caps) = re.captures_at(haystack, m.start) else {
        return Err(enzui_core::localization::text("text-stale-match-3b336e"));
    };
    match caps.get(0) {
        Some(hit) if hit.start() == m.start && hit.end() == m.end => {}
        _ => return Err(enzui_core::localization::text("text-stale-match-3b336e")),
    }
    let mut expanded = String::new();
    let replaced_len = if opts.regex {
        caps.expand(replacement, &mut expanded);
        if expanded.len() > MAX_REPLACE_OUTPUT_BYTES {
            return Err(enzui_core::localization::text(
                "text-replacement-would-exceed-the-output-size-limit-b76ee5",
            ));
        }
        expanded.len()
    } else {
        replacement.len()
    };
    let suffix_len = haystack
        .len()
        .checked_sub(m.end)
        .ok_or_else(|| enzui_core::localization::text("text-stale-match-3b336e"))?;
    let total = m
        .start
        .checked_add(replaced_len)
        .and_then(|s| s.checked_add(suffix_len))
        .ok_or_else(|| {
            enzui_core::localization::text(
                "text-replacement-would-exceed-the-output-size-limit-b76ee5",
            )
        })?;
    if total > MAX_REPLACE_OUTPUT_BYTES {
        return Err(enzui_core::localization::text(
            "text-replacement-would-exceed-the-output-size-limit-b76ee5",
        ));
    }
    let mut out = String::with_capacity(total);
    out.push_str(haystack.get(..m.start).ok_or_else(|| {
        enzui_core::localization::text("text-replacement-range-is-not-on-utf-8-86bdf0")
    })?);
    if opts.regex {
        out.push_str(&expanded);
    } else {
        out.push_str(replacement);
    }
    out.push_str(haystack.get(m.end..).ok_or_else(|| {
        enzui_core::localization::text("text-replacement-range-is-not-on-utf-8-86bdf0")
    })?);
    let span = ReplaceSpan {
        old_start: m.start,
        old_end: m.end,
        new_len: replaced_len,
    };
    Ok((out, span))
}

// One replacement performed by [`replace_all_spans`], in old-buffer byte
// coordinates, plus its byte length in the new buffer; [`map_offset`] uses it
// to carry a caret across a replace-all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReplaceSpan {
    // Byte offset of the match's start in the original haystack.
    pub old_start: usize,
    // Byte offset of the match's end (exclusive) in the original haystack.
    pub old_end: usize,
    // Byte length of the text that replaced it in the new buffer; differs per
    // match under a regex replacement with capture groups.
    pub new_len: usize,
}

impl ReplaceSpan {
    // The byte length this match occupied in the original haystack.
    pub(crate) fn old_len(&self) -> usize {
        self.old_end - self.old_start
    }
}

// Spans must be sorted and non-overlapping; offsets inside a replacement clamp to its new length.
pub(crate) fn map_offset(spans: &[ReplaceSpan], offset: usize) -> usize {
    let mut delta: isize = 0;
    for s in spans {
        if s.old_end <= offset {
            delta += s.new_len as isize - s.old_len() as isize;
        } else if s.old_start < offset {
            let within = (offset - s.old_start).min(s.new_len);
            return (s.old_start as isize + delta).max(0) as usize + within;
        } else {
            break;
        }
    }
    (offset as isize + delta).max(0) as usize
}

// Capture expansion uses complete-document context; output and span limits refuse the entire operation.
pub(crate) fn replace_all_spans(
    haystack: &str,
    query: &str,
    replacement: &str,
    opts: SearchOptions,
) -> Result<(String, Vec<ReplaceSpan>), String> {
    replace_all_spans_with_limits(
        haystack,
        query,
        replacement,
        opts,
        ReplaceLimits::PER_DOCUMENT,
    )
}

// [`replace_all_spans`] with caller-provided output-size and match-count
// ceilings, such as a batch's remaining budget.
//
// `limits` is clamped to [`MAX_REPLACE_OUTPUT_BYTES`] and
// [`MAX_RETAINED_MATCHES`]. A zero limit refuses the first non-zero write.
pub(crate) fn replace_all_spans_with_limits(
    haystack: &str,
    query: &str,
    replacement: &str,
    opts: SearchOptions,
    limits: ReplaceLimits,
) -> Result<(String, Vec<ReplaceSpan>), String> {
    let output_limit = limits.output_bytes.min(MAX_REPLACE_OUTPUT_BYTES);
    let match_limit = limits.matches.min(MAX_RETAINED_MATCHES);
    if query.is_empty() {
        return Ok((haystack.to_owned(), Vec::new()));
    }
    if replacement.len() > MAX_REPLACEMENT_BYTES {
        return Err(enzui_core::localization::text(
            "text-replacement-is-too-long-dd8872",
        ));
    }
    let re = compile_regex(query, opts)?;
    let mut spans: Vec<ReplaceSpan> = Vec::new();
    let mut out = String::new();
    let mut last = 0;
    for caps in re.captures_iter(haystack) {
        let Some(hit) = caps.get(0) else { continue };
        if hit.start() == hit.end() {
            continue;
        }
        if spans.len() >= match_limit {
            return Err(enzui_core::localization::text(
                "text-too-many-matches-for-a-single-replace-4ed30b",
            ));
        }
        let prefix_len = hit.start().checked_sub(last).ok_or_else(|| {
            enzui_core::localization::text("text-replace-all-offsets-went-backwards-cca406")
        })?;
        let after_prefix = out.len().checked_add(prefix_len).ok_or_else(|| {
            enzui_core::localization::text(
                "text-replacement-would-exceed-the-output-size-limit-b76ee5",
            )
        })?;
        if after_prefix > output_limit {
            return Err(enzui_core::localization::text(
                "text-replacement-would-exceed-the-output-size-limit-b76ee5",
            ));
        }
        let mut expanded = String::new();
        let new_len = if opts.regex {
            caps.expand(replacement, &mut expanded);
            if expanded.len() > output_limit {
                return Err(enzui_core::localization::text(
                    "text-replacement-would-exceed-the-output-size-limit-b76ee5",
                ));
            }
            expanded.len()
        } else {
            replacement.len()
        };
        let after_replacement = after_prefix.checked_add(new_len).ok_or_else(|| {
            enzui_core::localization::text(
                "text-replacement-would-exceed-the-output-size-limit-b76ee5",
            )
        })?;
        if after_replacement > output_limit {
            return Err(enzui_core::localization::text(
                "text-replacement-would-exceed-the-output-size-limit-b76ee5",
            ));
        }
        out.push_str(haystack.get(last..hit.start()).ok_or_else(|| {
            enzui_core::localization::text("text-replacement-range-is-not-on-utf-8-86bdf0")
        })?);
        if opts.regex {
            out.push_str(&expanded);
        } else {
            out.push_str(replacement);
        }
        spans.push(ReplaceSpan {
            old_start: hit.start(),
            old_end: hit.end(),
            new_len,
        });
        last = hit.end();
    }
    if spans.is_empty() {
        return Ok((haystack.to_owned(), spans));
    }
    let tail_len = haystack.len().checked_sub(last).ok_or_else(|| {
        enzui_core::localization::text("text-replace-all-offsets-went-past-the-haystack-af46ab")
    })?;
    let total = out.len().checked_add(tail_len).ok_or_else(|| {
        enzui_core::localization::text("text-replacement-would-exceed-the-output-size-limit-b76ee5")
    })?;
    if total > output_limit {
        return Err(enzui_core::localization::text(
            "text-replacement-would-exceed-the-output-size-limit-b76ee5",
        ));
    }
    out.push_str(haystack.get(last..).ok_or_else(|| {
        enzui_core::localization::text("text-replacement-range-is-not-on-utf-8-86bdf0")
    })?);
    Ok((out, spans))
}
