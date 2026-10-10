//! Цветовые схемы и деградация цвета под возможности терминала.
//!
//! Схема задаёт цвета в RGB. Терминал, не умеющий 24-битный цвет, получает
//! ближайший цвет из палитры xterm-256 или из базовых 16. Тема `dark` не
//! закрашивает фон и основной текст: она рассчитана на тёмный терминал
//! пользователя и сохраняет его собственные цвета. Остальные темы рисуют фон
//! сами, иначе светлая схема в тёмном терминале была бы нечитаемой.

use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

use crate::highlight::Token;

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

/// Цвета подсветки синтаксиса и поиска.
struct Syntax {
    comment: Rgb,
    string: Rgb,
    number: Rgb,
    keyword: Rgb,
    typ: Rgb,
    key: Rgb,
    added: Rgb,
    removed: Rgb,
    warn: Rgb,
    /// Фон найденного текста и текущего совпадения.
    found: Rgb,
    found_current: Rgb,
    /// Текст на фоне текущего совпадения.
    found_ink: Rgb,
}

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
    syntax: Syntax,
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
    syntax: Syntax {
        comment: (0x82, 0x8B, 0x99),
        string: (0x6F, 0xA8, 0x5A),
        number: (0xC9, 0x8A, 0x4B),
        keyword: (0xB0, 0x6B, 0xC9),
        typ: (0x4E, 0xA3, 0xC4),
        key: (0x5A, 0x9B, 0xE0),
        added: (0x5F, 0xA8, 0x5F),
        removed: (0xD0, 0x5F, 0x5F),
        warn: (0xE6, 0xC8, 0x55),
        found: (0x5C, 0x4A, 0x1A),
        found_current: (0xE6, 0xA8, 0x2E),
        found_ink: (0x10, 0x10, 0x10),
    },
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
    syntax: Syntax {
        comment: (0x6A, 0x73, 0x7D),
        string: (0x22, 0x86, 0x3A),
        number: (0xB3, 0x59, 0x00),
        keyword: (0x8F, 0x3F, 0xBF),
        typ: (0x1B, 0x7C, 0x83),
        key: (0x00, 0x5C, 0xC5),
        added: (0x22, 0x86, 0x3A),
        removed: (0xCB, 0x24, 0x31),
        warn: (0x9A, 0x67, 0x00),
        found: (0xFF, 0xEB, 0x99),
        found_current: (0xFF, 0xB0, 0x20),
        found_ink: (0x10, 0x10, 0x10),
    },
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
    syntax: Syntax {
        comment: (0x92, 0x83, 0x74),
        string: (0xB8, 0xBB, 0x26),
        number: (0xD3, 0x86, 0x9B),
        keyword: (0xFB, 0x49, 0x34),
        typ: (0xFA, 0xBD, 0x2F),
        key: (0x83, 0xA5, 0x98),
        added: (0xB8, 0xBB, 0x26),
        removed: (0xFB, 0x49, 0x34),
        warn: (0xFE, 0x80, 0x19),
        found: (0x5A, 0x4A, 0x1E),
        found_current: (0xFA, 0xBD, 0x2F),
        found_ink: (0x28, 0x28, 0x28),
    },
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
    syntax: Syntax {
        comment: (0x7A, 0x8D, 0x93),
        string: (0x2A, 0xA1, 0x98),
        number: (0xD3, 0x36, 0x82),
        keyword: (0x85, 0x99, 0x00),
        typ: (0xB5, 0x89, 0x00),
        key: (0x26, 0x8B, 0xD2),
        added: (0x85, 0x99, 0x00),
        removed: (0xDC, 0x32, 0x2F),
        warn: (0xCB, 0x4B, 0x16),
        found: (0x2E, 0x4A, 0x1E),
        found_current: (0xB5, 0x89, 0x00),
        found_ink: (0x00, 0x2B, 0x36),
    },
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
    syntax: Syntax {
        comment: (0x8C, 0x98, 0xB0),
        string: (0xA3, 0xBE, 0x8C),
        number: (0xB4, 0x8E, 0xAD),
        keyword: (0x81, 0xA1, 0xC1),
        typ: (0x8F, 0xBC, 0xBB),
        key: (0x88, 0xC0, 0xD0),
        added: (0xA3, 0xBE, 0x8C),
        removed: (0xBF, 0x61, 0x6A),
        warn: (0xEB, 0xCB, 0x8B),
        found: (0x4C, 0x56, 0x6A),
        found_current: (0xEB, 0xCB, 0x8B),
        found_ink: (0x2E, 0x34, 0x40),
    },
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

    /// Выделенный элемент.
    pub fn marked(&self) -> Style {
        self.key()
    }

    /// Фрагмент подсвеченного текста. Обычный текст рисуется цветом темы.
    pub fn token(&self, token: Token) -> Style {
        let x = &self.palette().syntax;
        let color = match token {
            Token::Plain => return Style::default(),
            Token::Comment => x.comment,
            Token::Str => x.string,
            Token::Number => x.number,
            Token::Keyword => x.keyword,
            Token::Type => x.typ,
            Token::Key => x.key,
            Token::Added => x.added,
            Token::Removed => x.removed,
            Token::Warn => x.warn,
            Token::Error => {
                return Style::default()
                    .fg(self.rgb(x.removed))
                    .add_modifier(Modifier::BOLD);
            }
        };
        Style::default().fg(self.rgb(color))
    }

    /// Найденный при поиске текст; текущее совпадение ярче остальных.
    pub fn found(&self, current: bool) -> Style {
        let x = &self.palette().syntax;
        if current {
            Style::default()
                .bg(self.rgb(x.found_current))
                .fg(self.rgb(x.found_ink))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(self.rgb(x.found))
        }
    }

    /// Сообщение об ошибке.
    pub fn error(&self) -> Style {
        Style::default().fg(self.rgb(self.palette().error))
    }
}

/// Опорные точки шкалы размера: от светло-зелёного (мало) к буро-красному
/// (много) — как во `flob`.
const HEAT: [(f32, Rgb); 5] = [
    (0.00, (0xA8, 0xE6, 0xA0)),
    (0.35, (0xD9, 0xE0, 0x6B)),
    (0.60, (0xE8, 0xA3, 0x3D)),
    (0.80, (0xD9, 0x54, 0x2B)),
    (1.00, (0x8C, 0x2F, 0x14)),
];

/// Сколько порядков величины укладывается в шкалу: объект в `10^4` раз меньше
/// максимального уже полностью «холодный».
const DYNAMIC_RANGE: f64 = 1e4;

/// Нормированное «тепло» объекта: логарифм отношения к максимуму. Линейная
/// шкала окрасила бы почти всё в зелёный — размеры распределены очень неравно.
pub fn heat_t(size: u64, max: u64) -> f32 {
    if max == 0 || size == 0 {
        return 0.0;
    }
    let ratio = max as f64 / size as f64;
    (1.0 - ratio.ln() / DYNAMIC_RANGE.ln()).clamp(0.0, 1.0) as f32
}

fn heat_rgb(t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8, k: f32| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
    for pair in HEAT.windows(2) {
        let ((t0, c0), (t1, c1)) = (pair[0], pair[1]);
        if t <= t1 {
            let k = (t - t0) / (t1 - t0);
            return (
                lerp(c0.0, c1.0, k),
                lerp(c0.1, c1.1, k),
                lerp(c0.2, c1.2, k),
            );
        }
    }
    HEAT[HEAT.len() - 1].1
}

/// Число с разделителями разрядов: `1 234 567`.
pub fn format_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

impl Theme {
    /// Цвет «тепла» `t` из `[0, 1]`.
    pub fn heat(&self, t: f32) -> Style {
        Style::default().fg(self.rgb(heat_rgb(t)))
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
            let x = &p.syntax;
            for (what, color) in [
                ("comment", x.comment),
                ("string", x.string),
                ("number", x.number),
                ("keyword", x.keyword),
                ("type", x.typ),
                ("key", x.key),
                ("removed", x.removed),
                ("warn", x.warn),
            ] {
                let c = contrast(color, bg);
                assert!(c >= 3.0, "{name:?}: {what} контраст {c:.2}");
            }
            let c = contrast(x.found_ink, x.found_current);
            assert!(c >= 4.5, "{name:?}: совпадение контраст {c:.2}");
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

    #[test]
    fn format_count_groups_digits() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(1234), "1 234");
        assert_eq!(format_count(1_234_567), "1 234 567");
    }

    #[test]
    fn heat_scale_is_logarithmic() {
        let max = 160 * 1024 * 1024;
        assert!((heat_t(max, max) - 1.0).abs() < 1e-6);
        assert!((heat_t(max / 100, max) - 0.5).abs() < 0.02);
        assert_eq!(heat_t(max / 100_000, max), 0.0);
        assert_eq!(heat_t(0, max), 0.0);
        assert_eq!(heat_rgb(0.0), HEAT[0].1);
        assert_eq!(heat_rgb(1.0), HEAT[4].1);
    }
}
