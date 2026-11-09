//! Persistent editor settings.
//!
//! Nothing here reads the process environment: paths resolve through pure
//! `*_from(xdg, home)` helpers, and boot resolves the settings path once
//! through `TextApp::resolve_boot_settings_path`. [`Settings::save_to`]
//! publishes through the framework's atomic-state writer.

use std::path::{Path, PathBuf};

const MAX_SETTINGS_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FontSizeScale {
    Small,
    Medium,
    Large,
}

impl FontSizeScale {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FontSizeScale::Small => "small",
            FontSizeScale::Medium => "medium",
            FontSizeScale::Large => "large",
        }
    }
    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "small" => FontSizeScale::Small,
            "large" => FontSizeScale::Large,
            _ => FontSizeScale::Medium,
        }
    }
    pub(crate) fn px(self) -> f32 {
        let authored = match self {
            FontSizeScale::Small => 11.0,
            FontSizeScale::Medium => 14.0,
            FontSizeScale::Large => 17.0,
        };
        enzui_theme::text_scale::effective().apply_px(authored)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Settings {
    pub show_line_numbers: bool,
    pub word_wrap: bool,
    pub font_size: FontSizeScale,
    pub editor_theme: crate::editor_theme::EditorTheme,
    pub show_counts: bool,
    pub restore_session: bool,
    pub sidebar_visible: bool,
    // The last-known workspace-tree scroll offset in logical px, restored on
    // rebuild and at the next launch.
    //
    // Always finite and non-negative; parsing rejects other values.
    pub workspace_sidebar_scroll_offset: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            show_line_numbers: true,
            word_wrap: true,
            font_size: FontSizeScale::Medium,
            editor_theme: crate::editor_theme::EditorTheme::default(),
            show_counts: true,
            restore_session: true,
            sidebar_visible: true,
            workspace_sidebar_scroll_offset: 0.0,
        }
    }
}

impl Settings {
    pub(crate) fn config_path_from(
        xdg_config_home: Option<&Path>,
        home: Option<&Path>,
    ) -> Option<PathBuf> {
        let base = xdg_config_home.map(PathBuf::from).or_else(|| {
            home.map(|h| {
                let mut p = PathBuf::from(h);
                p.push(".config");
                p
            })
        })?;
        let mut p = base;
        p.push("text-app");
        p.push("settings.conf");
        Some(p)
    }

    // Refuse special files without following symlinks or waiting for FIFO writers.
    pub(crate) fn load_from(path: &Path) -> Option<Self> {
        use rustix::fs::{Mode, OFlags};
        use std::io::Read;

        let descriptor = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .ok()?;
        let file = std::fs::File::from(descriptor);
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > MAX_SETTINGS_BYTES {
            return None;
        }
        // The descriptor can grow after stat, so the read has its own ceiling.
        let mut contents = String::new();
        file.take(MAX_SETTINGS_BYTES + 1)
            .read_to_string(&mut contents)
            .ok()?;
        if contents.len() as u64 > MAX_SETTINGS_BYTES {
            return None;
        }
        Some(Self::parse(&contents))
    }

    // Atomic replacement preserves a symlink target and publishes owner-only data.
    pub(crate) fn save_to(&self, path: &Path) -> bool {
        enzui_core::atomic_state::save_atomic_raw(path, self.serialize().as_bytes()).is_ok()
    }
}

impl Settings {
    pub(crate) fn serialize(&self) -> String {
        format!(
            "show_line_numbers={}\nword_wrap={}\nfont_size={}\nshow_counts={}\nrestore_session={}\nsidebar_visible={}\nworkspace_sidebar_scroll_offset={}\neditor_theme={}\n",
            self.show_line_numbers,
            self.word_wrap,
            self.font_size.as_str(),
            self.show_counts,
            self.restore_session,
            self.sidebar_visible,
            self.workspace_sidebar_scroll_offset,
            self.editor_theme.id(),
        )
    }

    pub(crate) fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "show_line_numbers" => {
                    s.show_line_numbers = parse_bool(v).unwrap_or(s.show_line_numbers);
                }
                "word_wrap" => s.word_wrap = parse_bool(v).unwrap_or(s.word_wrap),
                "font_size" => s.font_size = FontSizeScale::parse(v),
                "editor_theme" => s.editor_theme = crate::editor_theme::EditorTheme::parse(v),
                "show_counts" => s.show_counts = parse_bool(v).unwrap_or(s.show_counts),
                "restore_session" => s.restore_session = parse_bool(v).unwrap_or(s.restore_session),
                "sidebar_visible" => s.sidebar_visible = parse_bool(v).unwrap_or(s.sidebar_visible),
                "workspace_sidebar_scroll_offset" => {
                    // Keeps the model finite and non-negative even for a
                    // hand-edited file.
                    if let Ok(parsed) = v.parse::<f32>() {
                        s.workspace_sidebar_scroll_offset =
                            enzui_core::metrics::FiniteNonNeg::new_or(parsed, 0.0).get();
                    }
                }
                _ => {}
            }
        }
        s
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {}
