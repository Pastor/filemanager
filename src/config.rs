//! Конфигурация: настройки пользователя и состояние сессии в одном TOML-файле.
//!
//! Файл лежит в `~/.config/fmgr/config.toml` на всех платформах. `dirs::config_dir`
//! на macOS указывает в `~/Library/Application Support`, где консольные утилиты
//! никто не ищет, поэтому путь строится от домашней директории явно.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::i18n::Lang;
use crate::ui::theme::ThemeName;

/// Наименьшая доля ширины окна, которую может занимать левая панель.
pub const LEFT_PCT_MIN: u16 = 15;
/// Наибольшая: правой панели должно остаться хоть что-то.
pub const LEFT_PCT_MAX: u16 = 85;
/// Шаг изменения ширины одним нажатием.
pub const LEFT_PCT_STEP: u16 = 5;
/// Наибольшее число колонок в панели.
pub const MAX_COLUMNS: u8 = 3;

/// Сторона двухпанельного окна.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[default]
    Left,
    Right,
}

impl Side {
    /// Противоположная сторона.
    pub fn other(self) -> Self {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// Состояние окна, которое переживает перезапуск.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    /// Последняя директория правой панели.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<PathBuf>,
    /// Доля ширины окна под левую панель, в процентах.
    pub left_pct: u16,
    /// Число колонок левой панели.
    pub left_columns: u8,
    /// Число колонок правой панели.
    pub right_columns: u8,
    /// Скрытая панель, если одна из панелей скрыта.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<Side>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            right: None,
            left_pct: 50,
            left_columns: 1,
            right_columns: 1,
            hidden: None,
        }
    }
}

impl Session {
    /// Подрезает значения, вылезшие за допустимые пределы: из-за ручной правки
    /// файла программа не должна ни падать, ни рисовать нулевую панель.
    fn sanitize(&mut self) {
        self.left_pct = clamp_left_pct(self.left_pct);
        self.left_columns = clamp_columns(self.left_columns);
        self.right_columns = clamp_columns(self.right_columns);
    }
}

/// Настройки, сохраняемые в TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Язык интерфейса.
    pub lang: Lang,
    /// Цветовая схема.
    pub theme: ThemeName,
    /// Показывать скрытые файлы.
    pub show_hidden: bool,
    /// Мягкий перенос строк в просмотре по умолчанию.
    pub wrap: bool,
    /// Размер отступа, если его не удалось определить по файлу и языку.
    pub indent: u8,
    /// `d` отправляет в корзину; иначе удаляет безвозвратно.
    pub use_trash: bool,
    /// Состояние сессии. В файле это раздел `[session]`, поэтому объявлен
    /// последним: TOML требует, чтобы таблицы шли после простых значений.
    pub session: Session,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lang: Lang::En,
            theme: ThemeName::Dark,
            show_hidden: true,
            wrap: true,
            indent: 4,
            use_trash: true,
            session: Session::default(),
        }
    }
}

impl Config {
    /// Путь к файлу конфигурации: `~/.config/fmgr/config.toml`.
    pub fn path() -> Option<PathBuf> {
        dirs::home_dir().map(|d| d.join(".config").join(APP).join("config.toml"))
    }

    /// Где лежал конфиг, пока программа называлась `fm`. Имя пришлось
    /// сменить: в macOS 27 появилась системная `/usr/bin/fm`.
    fn legacy_path() -> Option<PathBuf> {
        dirs::home_dir().map(|d| d.join(".config").join("fm").join("config.toml"))
    }

    /// Загружает конфиг; отсутствие файла — не ошибка, берутся значения по умолчанию.
    pub fn load() -> Result<Self> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        if let Some(legacy) = Self::legacy_path() {
            migrate(&legacy, &path);
        }
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("cannot parse {}", path.display()))
    }

    /// Разбирает текст конфига и приводит значения к допустимым.
    pub fn parse(text: &str) -> Result<Self> {
        let mut cfg: Config = toml::from_str(text)?;
        cfg.session.sanitize();
        cfg.indent = cfg.indent.clamp(1, 16);
        Ok(cfg)
    }

    /// Сохраняет конфиг, создавая директорию при необходимости.
    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path().context("cannot determine home directory")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(path)
    }
}

/// Имя программы: команда запуска и папка настроек.
pub const APP: &str = "fmgr";

/// Переносит конфиг со старого места, если на новом его ещё нет. Старый файл
/// не удаляется — копия безопаснее переноса, а мешать он не будет.
fn migrate(legacy: &std::path::Path, path: &std::path::Path) {
    if path.exists() || !legacy.is_file() {
        return;
    }
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_ok()
    {
        let _ = std::fs::copy(legacy, path);
    }
}

/// Приводит долю левой панели к допустимому диапазону.
pub fn clamp_left_pct(pct: u16) -> u16 {
    pct.clamp(LEFT_PCT_MIN, LEFT_PCT_MAX)
}

/// Приводит число колонок к диапазону `0..=MAX_COLUMNS`; 0 — одна колонка
/// без размера и даты.
pub fn clamp_columns(n: u8) -> u8 {
    n.min(MAX_COLUMNS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_toml_roundtrip() {
        let mut cfg = Config {
            lang: Lang::Ru,
            theme: ThemeName::Gruvbox,
            show_hidden: false,
            ..Config::default()
        };
        cfg.session.right = Some(PathBuf::from("/tmp"));
        cfg.session.left_pct = 35;
        cfg.session.right_columns = 3;
        cfg.session.hidden = Some(Side::Right);

        let text = toml::to_string_pretty(&cfg).unwrap();
        let back = Config::parse(&text).unwrap();
        assert_eq!(back.lang, Lang::Ru);
        assert_eq!(back.theme, ThemeName::Gruvbox);
        assert!(!back.show_hidden);
        assert_eq!(back.session.right, Some(PathBuf::from("/tmp")));
        assert_eq!(back.session.left_pct, 35);
        assert_eq!(back.session.right_columns, 3);
        assert_eq!(back.session.hidden, Some(Side::Right));
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let cfg = Config::parse("lang = \"ru\"").unwrap();
        assert_eq!(cfg.lang, Lang::Ru);
        assert_eq!(cfg.theme, ThemeName::Dark);
        assert_eq!(cfg.session.left_pct, 50);
        assert!(cfg.session.hidden.is_none());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let cfg = Config::parse(
            "indent = 0\n[session]\nleft_pct = 100\nleft_columns = 0\nright_columns = 9\n",
        )
        .unwrap();
        assert_eq!(cfg.indent, 1);
        assert_eq!(cfg.session.left_pct, LEFT_PCT_MAX);
        assert_eq!(cfg.session.left_columns, 0);
        assert_eq!(cfg.session.right_columns, MAX_COLUMNS);
    }

    #[test]
    fn legacy_config_is_copied_once() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("fm/config.toml");
        let path = tmp.path().join("fmgr/config.toml");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "lang = \"ru\"").unwrap();

        migrate(&legacy, &path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "lang = \"ru\"");
        assert!(legacy.exists());

        // Новый конфиг уже есть — старый его не затирает.
        std::fs::write(&path, "lang = \"en\"").unwrap();
        migrate(&legacy, &path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "lang = \"en\"");

        // Старого нет — ничего не происходит.
        let fresh = tmp.path().join("other/config.toml");
        migrate(&tmp.path().join("missing"), &fresh);
        assert!(!fresh.exists());
    }

    #[test]
    fn unknown_enum_value_is_an_error() {
        assert!(Config::parse("theme = \"neon\"").is_err());
    }
}
