//! Color palettes. Every color in the UI comes from here.

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Stable name: this is the one saved in the configuration.
    pub id: &'static str,
    pub name: &'static str,
    pub bg: Color,
    /// Background of modals and floating windows.
    pub surface: Color,
    /// Background of the selected row.
    pub selection: Color,
    pub fg: Color,
    /// Secondary text: line numbers, notes.
    pub muted: Color,
    pub border: Color,
    /// Border of the focused panel, active tab, headers.
    pub accent: Color,
    pub on_accent: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub magenta: Color,
    pub cyan: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// All themes, in picker order. The first one is the default.
pub const THEMES: &[Theme] = &[
    Theme {
        id: "terminal",
        name: "Terminal",
        bg: Color::Reset,
        surface: Color::Reset,
        selection: Color::DarkGray,
        fg: Color::Reset,
        muted: Color::Gray,
        border: Color::Gray,
        accent: Color::Green,
        on_accent: Color::Black,
        red: Color::Red,
        green: Color::Green,
        yellow: Color::Yellow,
        blue: Color::Blue,
        magenta: Color::Magenta,
        cyan: Color::Cyan,
    },
    Theme {
        id: "kanagawa",
        name: "Kanagawa",
        bg: rgb(0x1F1F28),
        surface: rgb(0x2A2A37),
        selection: rgb(0x2D4F67),
        fg: rgb(0xDCD7BA),
        muted: rgb(0x727169),
        border: rgb(0x54546D),
        accent: rgb(0x7E9CD8),
        on_accent: rgb(0x1F1F28),
        red: rgb(0xE46876),
        green: rgb(0x98BB6C),
        yellow: rgb(0xE6C384),
        blue: rgb(0x7E9CD8),
        magenta: rgb(0x957FB8),
        cyan: rgb(0x7FB4CA),
    },
    Theme {
        id: "monokai",
        name: "Monokai",
        bg: rgb(0x272822),
        surface: rgb(0x3E3D32),
        selection: rgb(0x49483E),
        fg: rgb(0xF8F8F2),
        muted: rgb(0x75715E),
        border: rgb(0x75715E),
        accent: rgb(0xA6E22E),
        on_accent: rgb(0x272822),
        red: rgb(0xF92672),
        green: rgb(0xA6E22E),
        yellow: rgb(0xE6DB74),
        blue: rgb(0x66D9EF),
        magenta: rgb(0xAE81FF),
        cyan: rgb(0x66D9EF),
    },
    Theme {
        id: "dracula",
        name: "Dracula",
        bg: rgb(0x282A36),
        surface: rgb(0x21222C),
        selection: rgb(0x44475A),
        fg: rgb(0xF8F8F2),
        muted: rgb(0x6272A4),
        border: rgb(0x6272A4),
        accent: rgb(0xBD93F9),
        on_accent: rgb(0x282A36),
        red: rgb(0xFF5555),
        green: rgb(0x50FA7B),
        yellow: rgb(0xF1FA8C),
        blue: rgb(0xBD93F9),
        magenta: rgb(0xFF79C6),
        cyan: rgb(0x8BE9FD),
    },
    Theme {
        id: "gruvbox",
        name: "Gruvbox",
        bg: rgb(0x282828),
        surface: rgb(0x3C3836),
        selection: rgb(0x504945),
        fg: rgb(0xEBDBB2),
        muted: rgb(0x928374),
        border: rgb(0x665C54),
        accent: rgb(0xFABD2F),
        on_accent: rgb(0x282828),
        red: rgb(0xFB4934),
        green: rgb(0xB8BB26),
        yellow: rgb(0xFABD2F),
        blue: rgb(0x83A598),
        magenta: rgb(0xD3869B),
        cyan: rgb(0x8EC07C),
    },
    Theme {
        id: "catppuccin",
        name: "Catppuccin Mocha",
        bg: rgb(0x1E1E2E),
        surface: rgb(0x313244),
        selection: rgb(0x45475A),
        fg: rgb(0xCDD6F4),
        muted: rgb(0x7F849C),
        border: rgb(0x585B70),
        accent: rgb(0xCBA6F7),
        on_accent: rgb(0x1E1E2E),
        red: rgb(0xF38BA8),
        green: rgb(0xA6E3A1),
        yellow: rgb(0xF9E2AF),
        blue: rgb(0x89B4FA),
        magenta: rgb(0xF5C2E7),
        cyan: rgb(0x94E2D5),
    },
    Theme {
        id: "tokyonight",
        name: "Tokyo Night",
        bg: rgb(0x1A1B26),
        surface: rgb(0x24283B),
        selection: rgb(0x283457),
        fg: rgb(0xC0CAF5),
        muted: rgb(0x565F89),
        border: rgb(0x3B4261),
        accent: rgb(0x7AA2F7),
        on_accent: rgb(0x1A1B26),
        red: rgb(0xF7768E),
        green: rgb(0x9ECE6A),
        yellow: rgb(0xE0AF68),
        blue: rgb(0x7AA2F7),
        magenta: rgb(0xBB9AF7),
        cyan: rgb(0x7DCFFF),
    },
    Theme {
        id: "nord",
        name: "Nord",
        bg: rgb(0x2E3440),
        surface: rgb(0x3B4252),
        selection: rgb(0x434C5E),
        fg: rgb(0xD8DEE9),
        muted: rgb(0x616E88),
        border: rgb(0x4C566A),
        accent: rgb(0x88C0D0),
        on_accent: rgb(0x2E3440),
        red: rgb(0xBF616A),
        green: rgb(0xA3BE8C),
        yellow: rgb(0xEBCB8B),
        blue: rgb(0x81A1C1),
        magenta: rgb(0xB48EAD),
        cyan: rgb(0x8FBCBB),
    },
    Theme {
        id: "onedark",
        name: "One Dark",
        bg: rgb(0x282C34),
        surface: rgb(0x21252B),
        selection: rgb(0x3E4451),
        fg: rgb(0xABB2BF),
        muted: rgb(0x5C6370),
        border: rgb(0x4B5263),
        accent: rgb(0x61AFEF),
        on_accent: rgb(0x282C34),
        red: rgb(0xE06C75),
        green: rgb(0x98C379),
        yellow: rgb(0xE5C07B),
        blue: rgb(0x61AFEF),
        magenta: rgb(0xC678DD),
        cyan: rgb(0x56B6C2),
    },
    Theme {
        id: "rosepine",
        name: "Rosé Pine",
        bg: rgb(0x191724),
        surface: rgb(0x1F1D2E),
        selection: rgb(0x403D52),
        fg: rgb(0xE0DEF4),
        muted: rgb(0x6E6A86),
        border: rgb(0x524F67),
        accent: rgb(0xC4A7E7),
        on_accent: rgb(0x191724),
        red: rgb(0xEB6F92),
        green: rgb(0x9CCFD8),
        yellow: rgb(0xF6C177),
        blue: rgb(0x31748F),
        magenta: rgb(0xC4A7E7),
        cyan: rgb(0xEBBCBA),
    },
    Theme {
        id: "everforest",
        name: "Everforest",
        bg: rgb(0x2D353B),
        surface: rgb(0x343F44),
        selection: rgb(0x475258),
        fg: rgb(0xD3C6AA),
        muted: rgb(0x859289),
        border: rgb(0x4F585E),
        accent: rgb(0xA7C080),
        on_accent: rgb(0x2D353B),
        red: rgb(0xE67E80),
        green: rgb(0xA7C080),
        yellow: rgb(0xDBBC7F),
        blue: rgb(0x7FBBB3),
        magenta: rgb(0xD699B6),
        cyan: rgb(0x83C092),
    },
    Theme {
        id: "solarized",
        name: "Solarized Dark",
        bg: rgb(0x002B36),
        surface: rgb(0x073642),
        selection: rgb(0x0D4A57),
        fg: rgb(0x839496),
        muted: rgb(0x586E75),
        border: rgb(0x586E75),
        accent: rgb(0x268BD2),
        on_accent: rgb(0x002B36),
        red: rgb(0xDC322F),
        green: rgb(0x859900),
        yellow: rgb(0xB58900),
        blue: rgb(0x268BD2),
        magenta: rgb(0xD33682),
        cyan: rgb(0x2AA198),
    },
];

pub fn default_theme() -> &'static Theme {
    &THEMES[0]
}

pub fn by_id(id: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|theme| theme.id == id)
}

/// Position in `THEMES`, to mark the theme in the picker.
pub fn index_of(id: &str) -> Option<usize> {
    THEMES.iter().position(|theme| theme.id == id)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn ids_are_unique_and_terminal_is_the_default() {
        let ids: HashSet<&str> = THEMES.iter().map(|theme| theme.id).collect();
        assert_eq!(THEMES.len(), 12);
        assert_eq!(ids.len(), THEMES.len());
        assert_eq!(default_theme().id, "terminal");
    }

    #[test]
    fn lookups_by_id_and_position() {
        assert_eq!(by_id("nord").map(|theme| theme.name), Some("Nord"));
        assert_eq!(by_id("nope"), None);
        assert_eq!(index_of("nord"), Some(7));
        assert_eq!(index_of("nope"), None);
        for (i, theme) in THEMES.iter().enumerate() {
            assert_eq!(index_of(theme.id), Some(i));
            assert_eq!(by_id(theme.id), Some(&THEMES[i]));
        }
    }
}
