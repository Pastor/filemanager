//! Цветовые схемы и деградация цвета под возможности терминала.
//!
//! Схема задаёт цвета в RGB. Терминал, не умеющий 24-битный цвет, получает
//! ближайший цвет из палитры xterm-256 или из базовых 16. Тема `dark` не
//! закрашивает фон и основной текст: она рассчитана на тёмный терминал
//! пользователя и сохраняет его собственные цвета. Остальные темы рисуют фон
//! сами, иначе светлая схема в тёмном терминале была бы нечитаемой.

use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

/// Название цветовой схемы.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    Dark,
    Light,
    Gruvbox,
    Solarized,
    Nord,
}

type Rgb = (u8, u8, u8);

/// Цвета одной схемы.
struct Palette {
    /// Фон; `None` — фон терминала.
    bg: Option<Rgb>,
    /// Основной текст; `None` — цвет терминала.
    fg: Option<Rgb>,
    /// Второстепенный текст: размеры, даты, подписи.
    dim: Rgb,
    /// Рамка неактивной панели.
    border: Rgb,
    /// Рамка и заголовок активной панели.
    accent: Rgb,
    /// Исполняемые файлы.
    exec: Rgb,
    /// Символические ссылки.
    link: Rgb,
    /// Ошибки и битые ссылки.
    error: Rgb,
    /// Курсор в активной панели: фон и текст.
    cursor_bg: Rgb,
    cursor_fg: Rgb,
    /// Курсор в неактивной панели: фон.
    cursor_idle_bg: Rgb,
    /// Клавиши в строке подсказок.
    key: Rgb,
}

const DARK: Palette = Palette {
    bg: None,
    fg: None,
    dim: (0x80, 0x86, 0x90),
    border: (0x50, 0x55, 0x5E),
    accent: (0x5A, 0xA9, 0xE6),
    exec: (0x74, 0xC4, 0x76),
    link: (0x5A, 0xA9, 0xE6),
    error: (0xE0, 0x6C, 0x6C),
    cursor_bg: (0x2F, 0x4A, 0x6B),
    cursor_fg: (0xFF, 0xFF, 0xFF),
    cursor_idle_bg: (0x33, 0x36, 0x3C),
    key: (0xE6, 0xC8, 0x55),
};

const LIGHT: Palette = Palette {
    bg: Some((0xFA, 0xFA, 0xF7)),
    fg: Some((0x24, 0x29, 0x2E)),
    dim: (0x7A, 0x80, 0x89),
    border: (0xC0, 0xC4, 0xCA),
    accent: (0x1F, 0x6F, 0xC5),
    exec: (0x1A, 0x7F, 0x37),
    link: (0x05, 0x50, 0xAE),
    error: (0xCF, 0x22, 0x2E),
    cursor_bg: (0xC8, 0xDD, 0xF5),
    cursor_fg: (0x10, 0x14, 0x18),
    cursor_idle_bg: (0xE6, 0xE6, 0xE3),
    key: (0x95, 0x40, 0x00),
};

const GRUVBOX: Palette = Palette {
    bg: Some((0x28, 0x28, 0x28)),
    fg: Some((0xEB, 0xDB, 0xB2)),
    dim: (0x92, 0x83, 0x74),
    border: (0x50, 0x49, 0x45),
    accent: (0xFA, 0xBD, 0x2F),
    exec: (0xB8, 0xBB, 0x26),
    link: (0x83, 0xA5, 0x98),
    error: (0xFB, 0x49, 0x34),
    cursor_bg: (0x50, 0x49, 0x45),
    cursor_fg: (0xFB, 0xF1, 0xC7),
    cursor_idle_bg: (0x3C, 0x38, 0x36),
    key: (0xFE, 0x80, 0x19),
};

const SOLARIZED: Palette = Palette {
    bg: Some((0x00, 0x2B, 0x36)),
    fg: Some((0x93, 0xA1, 0xA1)),
    dim: (0x65, 0x7B, 0x83),
    border: (0x0F, 0x4A, 0x58),
    accent: (0xB5, 0x89, 0x00),
    exec: (0x85, 0x99, 0x00),
    link: (0x26, 0x8B, 0xD2),
    error: (0xDC, 0x32, 0x2F),
    cursor_bg: (0x0B, 0x4F, 0x63),
    cursor_fg: (0xEE, 0xE8, 0xD5),
    cursor_idle_bg: (0x07, 0x36, 0x42),
    key: (0xCB, 0x4B, 0x16),
};

const NORD: Palette = Palette {
    bg: Some((0x2E, 0x34, 0x40)),
    fg: Some((0xD8, 0xDE, 0xE9)),
    dim: (0x7B, 0x88, 0xA1),
    border: (0x4C, 0x56, 0x6A),
    accent: (0x88, 0xC0, 0xD0),
    exec: (0xA3, 0xBE, 0x8C),
    link: (0x81, 0xA1, 0xC1),
    error: (0xBF, 0x61, 0x6A),
    cursor_bg: (0x43, 0x4C, 0x5E),
    cursor_fg: (0xEC, 0xEF, 0xF4),
    cursor_idle_bg: (0x3B, 0x42, 0x52),
    key: (0xEB, 0xCB, 0x8B),
};

impl ThemeName {
    fn palette(self) -> &'static Palette {
        match self {
            ThemeName::Dark => &DARK,
            ThemeName::Light => &LIGHT,
            ThemeName::Gruvbox => &GRUVBOX,
            ThemeName::Solarized => &SOLARIZED,
            ThemeName::Nord => &NORD,
        }
    }
}

/// Что умеет терминал.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    /// 24-битный цвет.
    True,
    /// Палитра из 256 цветов.
    Indexed,
    /// Базовые 16 цветов.
    Basic,
}

/// Готовые к отрисовке стили выбранной схемы.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub depth: ColorDepth,
    pub name: ThemeName,
}

impl Theme {
    /// Тема с глубиной цвета, определённой по переменным окружения.
    pub fn detect(name: ThemeName) -> Self {
        Self {
            depth: detect_depth(),
            name,
        }
    }

    fn palette(&self) -> &'static Palette {
        self.name.palette()
    }

    /// Точный цвет с деградацией под возможности терминала.
    pub fn rgb(&self, (r, g, b): Rgb) -> Color {
        match self.depth {
            ColorDepth::True => Color::Rgb(r, g, b),
            ColorDepth::Indexed => Color::Indexed(to_cube256(r, g, b)),
            ColorDepth::Basic => nearest_basic(r, g, b),
        }
    }

    /// Базовый стиль: фон и основной текст. В 16 цветах свои фон и текст
    /// не задаются — грубое приближение хуже, чем цвета терминала.
    pub fn base(&self) -> Style {
        let p = self.palette();
        let mut style = Style::default();
        if self.depth == ColorDepth::Basic {
            return style;
        }
        if let Some(bg) = p.bg {
            style = style.bg(self.rgb(bg));
        }
        if let Some(fg) = p.fg {
            style = style.fg(self.rgb(fg));
        }
        style
    }

    /// Второстепенный текст.
    pub fn dim(&self) -> Style {
        Style::default().fg(self.rgb(self.palette().dim))
    }

    /// Рамка панели.
    pub fn border(&self, active: bool) -> Style {
        let p = self.palette();
        Style::default().fg(self.rgb(if active { p.accent } else { p.border }))
    }

    /// Заголовок панели.
    pub fn title(&self, active: bool) -> Style {
        if active {
            Style::default()
                .fg(self.rgb(self.palette().accent))
                .add_modifier(Modifier::BOLD)
        } else {
            self.dim()
        }
    }

    /// Имя директории: жирное, цветом основного текста.
    pub fn dir(&self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    /// Имя исполняемого файла.
    pub fn exec(&self) -> Style {
        Style::default().fg(self.rgb(self.palette().exec))
    }

    /// Имя символической ссылки.
    pub fn link(&self) -> Style {
        Style::default().fg(self.rgb(self.palette().link))
    }

    /// Битая ссылка.
    pub fn broken(&self) -> Style {
        Style::default()
            .fg(self.rgb(self.palette().error))
            .add_modifier(Modifier::CROSSED_OUT)
    }

    /// Строка под курсором; в неактивной панели курсор приглушён.
    pub fn cursor(&self, active: bool) -> Style {
        let p = self.palette();
        if active {
            Style::default()
                .bg(self.rgb(p.cursor_bg))
                .fg(self.rgb(p.cursor_fg))
        } else {
            Style::default().bg(self.rgb(p.cursor_idle_bg))
        }
    }

    /// Клавиша в строке подсказок.
    pub fn key(&self) -> Style {
        Style::default()
            .fg(self.rgb(self.palette().key))
            .add_modifier(Modifier::BOLD)
    }

    /// Сообщение об ошибке.
    pub fn error(&self) -> Style {
        Style::default().fg(self.rgb(self.palette().error))
    }
}

/// Читает `COLORTERM`/`TERM` и выбирает глубину цвета.
fn detect_depth() -> ColorDepth {
    let colorterm = std::env::var("COLORTERM")
        .unwrap_or_default()
        .to_lowercase();
    if colorterm.contains("truecolor") || colorterm.contains("24bit") {
        return ColorDepth::True;
    }
    let term = std::env::var("TERM").unwrap_or_default().to_lowercase();
    if term.contains("256") || term.contains("kitty") || term.contains("alacritty") {
        return ColorDepth::Indexed;
    }
    if term.is_empty() {
        // Пустой TERM встречается при запуске не из терминала; при реальном
        // запуске TUI терминал почти всегда умеет хотя бы 256 цветов.
        return ColorDepth::Indexed;
    }
    ColorDepth::Basic
}

/// Ближайший индекс из цветового куба 6×6×6 палитры xterm-256.
fn to_cube256(r: u8, g: u8, b: u8) -> u8 {
    let q = |v: u8| ((v as f32 / 255.0) * 5.0).round() as u8;
    16 + 36 * q(r) + 6 * q(g) + q(b)
}

/// Ближайший из 16 базовых цветов: по доминирующим каналам и яркости.
fn nearest_basic(r: u8, g: u8, b: u8) -> Color {
    let level = |v: u8| v >= 0x80;
    let bright = (r as u16 + g as u16 + b as u16) / 3 >= 0x90;
    match (level(r), level(g), level(b)) {
        (false, false, false) => {
            if bright {
                Color::Gray
            } else {
                Color::DarkGray
            }
        }
        (true, false, false) => Color::Red,
        (false, true, false) => Color::Green,
        (false, false, true) => Color::Blue,
        (true, true, false) => Color::Yellow,
        (true, false, true) => Color::Magenta,
        (false, true, true) => Color::Cyan,
        (true, true, true) => {
            if bright {
                Color::White
            } else {
                Color::Gray
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ThemeName; 5] = [
        ThemeName::Dark,
        ThemeName::Light,
        ThemeName::Gruvbox,
        ThemeName::Solarized,
        ThemeName::Nord,
    ];

    /// Относительная яркость по WCAG.
    fn luminance((r, g, b): Rgb) -> f64 {
        let ch = |v: u8| {
            let c = v as f64 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
    }

    fn contrast(a: Rgb, b: Rgb) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn file_colors_are_readable_on_theme_background() {
        // Тёмная тема рисует на фоне терминала — проверяем её на типичном чёрном.
        for name in ALL {
            let p = name.palette();
            let bg = p.bg.unwrap_or((0x1E, 0x1E, 0x1E));
            for (what, color) in [("exec", p.exec), ("link", p.link), ("dim", p.dim)] {
                let c = contrast(color, bg);
                assert!(c >= 3.0, "{name:?}: {what} контраст {c:.2}");
            }
            let c = contrast(p.cursor_fg, p.cursor_bg);
            assert!(c >= 4.5, "{name:?}: курсор контраст {c:.2}");
        }
    }

    #[test]
    fn exec_and_link_differ() {
        for name in ALL {
            let p = name.palette();
            assert_ne!(p.exec, p.link, "{name:?}");
        }
    }

    #[test]
    fn indexed_colors_stay_in_cube() {
        for name in ALL {
            let p = name.palette();
            for (r, g, b) in [p.exec, p.link, p.dim, p.accent, p.cursor_bg] {
                assert!((16..=231).contains(&to_cube256(r, g, b)));
            }
        }
    }

    #[test]
    fn theme_name_parses_from_toml() {
        #[derive(Deserialize)]
        struct W {
            t: ThemeName,
        }
        let w: W = toml::from_str("t = \"solarized\"").unwrap();
        assert_eq!(w.t, ThemeName::Solarized);
    }
}
