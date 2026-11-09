//! Logical line counts and native edit-delta byte totals.
//!
//! Native editor position indexes are UTF-8 bytes, not character columns.

pub(crate) fn line_count_of(text: &str) -> usize {
    text.bytes().filter(|byte| *byte == b'\n').count() + 1
}

// How many bytes one applied `EditDelta` inserted and deleted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DeltaBytes {
    pub inserted: usize,
    pub deleted: usize,
}
