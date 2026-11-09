//! Bounded, generation-bound Markdown display state; never document authority.

use crate::model::Tab;
use std::sync::Arc;

pub(crate) const MAX_PREVIEW_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_PREVIEW_NODES: usize = 4096;

#[derive(Debug)]
pub(crate) struct PreviewSnapshot {
    pub(crate) version: u64,
    pub(crate) blocks: Arc<Vec<enzui_widgets::markdown::EnzMarkdownBlock>>,
}

fn within_render_budget(blocks: &[enzui_widgets::markdown::EnzMarkdownBlock]) -> bool {
    use enzui_widgets::markdown::EnzMarkdownBlock as Block;
    if blocks.len() > MAX_PREVIEW_NODES {
        return false;
    }
    let mut pending: Vec<_> = blocks.iter().collect();
    let mut nodes = 0;
    while let Some(block) = pending.pop() {
        nodes += 1;
        match block {
            Block::Heading { inline, .. } | Block::Paragraph(inline) => {
                nodes += inline.spans().len();
            }
            Block::Quote(children) => {
                if pending.len() + children.len() > MAX_PREVIEW_NODES {
                    return false;
                }
                pending.extend(children);
            }
            Block::List { items, .. } => {
                nodes += items.len();
                for children in items {
                    if pending.len() + children.len() > MAX_PREVIEW_NODES {
                        return false;
                    }
                    pending.extend(children);
                }
            }
            Block::Table { header, rows, .. } => {
                nodes += header
                    .iter()
                    .chain(rows.iter().flatten())
                    .map(|cell| 1 + cell.spans().len())
                    .sum::<usize>();
            }
            Block::CodeBlock { code, .. } => nodes += code.lines().count(),
            Block::Rule | Block::Image { .. } | Block::Truncated { .. } => {}
            _ => return false,
        }
        if nodes > MAX_PREVIEW_NODES {
            return false;
        }
    }
    true
}

#[derive(Debug, Default)]
pub(crate) struct MarkdownPreview {
    pub(crate) open: bool,
    pub(crate) scroll: f32,
    pub(crate) source: Option<PreviewSnapshot>,
}

impl Tab {
    pub(crate) fn is_markdown(&self) -> bool {
        self.file_path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
            })
    }

    pub(crate) fn preview_visible(&self) -> bool {
        self.is_markdown()
            && self.preview.open
            && self
                .preview
                .source
                .as_ref()
                .is_some_and(|snapshot| snapshot.version == self.snapshot_version())
    }

    pub(crate) fn toggle_preview(&mut self) -> bool {
        if self.preview_visible() {
            self.preview.open = false;
            return true;
        }
        if self.loading
            || !self.is_markdown()
            || self.buffer.large().is_some()
            || self.document().len() > MAX_PREVIEW_BYTES
        {
            return false;
        }
        if self
            .preview
            .source
            .as_ref()
            .is_none_or(|snapshot| snapshot.version != self.snapshot_version())
        {
            let blocks = enzui_widgets::markdown::parse(&self.snapshot());
            if !within_render_budget(&blocks) {
                return false;
            }
            self.preview.source = Some(PreviewSnapshot {
                version: self.snapshot_version(),
                blocks: Arc::new(blocks),
            });
        }
        self.preview.open = true;
        true
    }
}
