//! Persistent document colour pairs, independent of application chrome.

use enzui_theme::Palette;
use iced::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum EditorTheme {
    #[default]
    System,
    Paper,
    Sepia,
    Grass,
    Sky,
    Solarized,
    Nord,
    Midnight,
}

impl EditorTheme {
    pub(crate) const ALL: [Self; 8] = [
        Self::System,
        Self::Paper,
        Self::Sepia,
        Self::Grass,
        Self::Sky,
        Self::Solarized,
        Self::Nord,
        Self::Midnight,
    ];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Paper => "paper",
            Self::Sepia => "sepia",
            Self::Grass => "grass",
            Self::Sky => "sky",
            Self::Solarized => "solarized",
            Self::Nord => "nord",
            Self::Midnight => "midnight",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Paper => "Paper",
            Self::Sepia => "Sepia",
            Self::Grass => "Grass",
            Self::Sky => "Sky",
            Self::Solarized => "Solarized",
            Self::Nord => "Nord",
            Self::Midnight => "Midnight",
        }
    }

    pub(crate) fn parse(id: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|theme| theme.id() == id)
            .unwrap_or_default()
    }

    pub(crate) fn palette(self, chrome: &Palette) -> Palette {
        let (background, ink, dark) = match self {
            Self::System => return chrome.clone(),
            Self::Paper => (0xffffff, 0x1a1a1a, false),
            Self::Sepia => (0xf1e8d0, 0x4d3828, false),
            Self::Grass => (0xd7dbbd, 0x232c16, false),
            Self::Sky => (0xcedef5, 0x172a45, false),
            Self::Solarized => (0xfdf6e3, 0x465b62, false),
            Self::Nord => (0x2e3440, 0xe5e9f0, true),
            Self::Midnight => (0x191c24, 0xe1e5ed, true),
        };
        let mut palette = if dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        palette.background = rgb(background);
        palette.surface = palette.background;
        palette.surface_alt = palette.background;
        palette.text = rgb(ink);
        palette.text_muted = palette
            .text
            .mix(palette.background, if dark { 0.35 } else { 0.035 });
        palette.text_subtle = palette
            .text
            .mix(palette.background, if dark { 0.55 } else { 0.07 });
        palette
    }
}

fn rgb(hex: u32) -> Color {
    let [_, red, green, blue] = hex.to_be_bytes();
    Color::from_rgb8(red, green, blue)
}
