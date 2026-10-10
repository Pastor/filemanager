//! Строки интерфейса на поддерживаемых языках.
//!
//! Каждый язык — экземпляр структуры [`Text`]. Новая строка добавляется полем
//! структуры, и компилятор не даст забыть перевод ни для одного языка. Новый язык —
//! ещё одна константа и вариант [`Lang`], логику интерфейса трогать не нужно.

use serde::{Deserialize, Serialize};

/// Язык интерфейса.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    /// Таблица строк этого языка.
    pub fn text(self) -> &'static Text {
        match self {
            Lang::En => &EN,
            Lang::Ru => &RU,
        }
    }
}

/// Все строки интерфейса.
pub struct Text {
    // Подсказки нижней строки.
    pub hint_move: &'static str,
    pub hint_in_out: &'static str,
    pub hint_panel: &'static str,
    pub hint_width: &'static str,
    pub hint_hide: &'static str,
    pub hint_columns: &'static str,
    pub hint_hidden: &'static str,
    pub hint_quit: &'static str,

    // Содержимое панели.
    /// Заглушка пустой директории.
    pub empty: &'static str,
    /// Метка директории в колонке размера.
    pub dir_label: &'static str,
    /// Метка ссылки на директорию в колонке размера.
    pub link_dir_label: &'static str,

    // Сообщения строки состояния.
    pub cannot_open: &'static str,
    pub hidden_shown: &'static str,
    pub hidden_concealed: &'static str,
    pub columns: &'static str,
    pub config_not_saved: &'static str,
}

pub static EN: Text = Text {
    hint_move: "move",
    hint_in_out: "in/out",
    hint_panel: "panel",
    hint_width: "width",
    hint_hide: "hide panel",
    hint_columns: "columns",
    hint_hidden: "hidden files",
    hint_quit: "quit",

    empty: "(empty)",
    dir_label: "<DIR>",
    link_dir_label: "<LINK>",

    cannot_open: "cannot open",
    hidden_shown: "hidden files are shown",
    hidden_concealed: "hidden files are concealed",
    columns: "columns",
    config_not_saved: "settings were not saved",
};

pub static RU: Text = Text {
    hint_move: "навигация",
    hint_in_out: "войти/выйти",
    hint_panel: "панель",
    hint_width: "ширина",
    hint_hide: "скрыть панель",
    hint_columns: "колонки",
    hint_hidden: "скрытые файлы",
    hint_quit: "выход",

    empty: "(пусто)",
    dir_label: "<ДИР>",
    link_dir_label: "<ССЫЛ>",

    cannot_open: "не удалось открыть",
    hidden_shown: "скрытые файлы показаны",
    hidden_concealed: "скрытые файлы спрятаны",
    columns: "колонок",
    config_not_saved: "настройки не сохранены",
};
