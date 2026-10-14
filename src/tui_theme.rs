//! Presentation-only palette. No domain semantics live here.
use ratatui::style::Color;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub fg: Color,
    pub border: Color,
    pub dim: Color,
    pub highlight_fg: Color,
    pub highlight_bg: Color,
    pub status: Color,
    pub danger: Color,
    pub accent: Color,
}

pub const MOCHA: Theme = Theme {
    name: "Catppuccin Mocha",
    bg: Color::Rgb(30, 30, 46),
    fg: Color::Rgb(205, 214, 244),
    border: Color::Rgb(88, 91, 112),
    dim: Color::Rgb(108, 112, 134),
    highlight_fg: Color::Rgb(30, 30, 46),
    highlight_bg: Color::Rgb(249, 226, 175),
    status: Color::Rgb(166, 227, 161),
    danger: Color::Rgb(243, 139, 168),
    accent: Color::Rgb(137, 180, 250),
};

pub const LATTE: Theme = Theme {
    name: "Catppuccin Latte",
    bg: Color::Rgb(239, 241, 245),
    fg: Color::Rgb(76, 79, 105),
    border: Color::Rgb(156, 160, 176),
    dim: Color::Rgb(140, 143, 161),
    highlight_fg: Color::Rgb(239, 241, 245),
    highlight_bg: Color::Rgb(136, 99, 171),
    status: Color::Rgb(64, 160, 67),
    danger: Color::Rgb(210, 61, 89),
    accent: Color::Rgb(30, 102, 175),
};

pub const ALL: [Theme; 2] = [MOCHA, LATTE];

pub fn next(theme: Theme) -> Theme {
    let index = ALL.iter().position(|t| *t == theme).unwrap_or(0);
    ALL[(index + 1) % ALL.len()]
}
