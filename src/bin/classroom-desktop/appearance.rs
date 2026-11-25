use super::*;
use iced::theme::Palette;
use std::sync::LazyLock;

static LIGHT: LazyLock<Theme> = LazyLock::new(|| Theme::custom("Classroom Light", Palette {
    background: WHITE,
    text: FG,
    primary: BLUE,
    ..Palette::LIGHT
}));

static DARK: LazyLock<Theme> = LazyLock::new(|| Theme::custom("Classroom Dark", Palette {
    background: color!(0x202124),
    text: color!(0xe8eaed),
    primary: color!(0x8ab4f8),
    success: color!(0x81c995),
    warning: color!(0xfdd663),
    danger: color!(0xf28b82),
}));

// A live window lets Iced follow the system. Supplying a Theme would force
// the window's native appearance and suppress later macOS theme changes.
impl App {
    pub(super) fn theme(&self) -> Option<Theme> {
        self.screenshot_directory.as_ref().map(|_| {
            if self.dark_fixture { DARK.clone() } else { LIGHT.clone() }
        })
    }
}

pub(super) fn classroom_palette(theme: &Theme) -> Palette {
    if theme.extended_palette().is_dark { DARK.palette() } else { LIGHT.palette() }
}

pub(super) fn appearance_color(theme: &Theme, color: Color) -> Color {
    if !theme.extended_palette().is_dark { return color; }

    match color {
        c if c == FG => classroom_palette(theme).text,
        c if c == MUTED => color!(0xb0b7c0),
        c if c == BORDER => color!(0x484c52),
        c if c == SOFT => color!(0x303237),
        c if c == BLUE || c == ACTIVE_FG => classroom_palette(theme).primary,
        c if c == ACTIVE_BG => color!(0x283b56),
        c if c == color!(0x9b2722) => classroom_palette(theme).danger,
        c if c == color!(0x137333) => classroom_palette(theme).success,
        c if c == color!(0xfce8e6) => color!(0x4c292b),
        c if c == color!(0xe6f4ea) => color!(0x253c32),
        c => c,
    }
}

pub(super) fn themed_text(theme: &Theme, color: Color) -> text::Style {
    text::Style { color: Some(appearance_color(theme, color)) }
}
