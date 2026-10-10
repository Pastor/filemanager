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

    // Выделение и операции.
    pub hint_mark: &'static str,
    pub hint_copy: &'static str,
    pub hint_move_to: &'static str,
    pub hint_delete: &'static str,
    /// Подпись счётчика выделенных в рамке панели.
    pub marked: &'static str,
    pub no_second_panel: &'static str,
    pub same_dir: &'static str,
    pub into_itself: &'static str,

    // Диалог удаления.
    pub delete_title: &'static str,
    pub delete_forever_title: &'static str,
    pub delete_to_trash: &'static str,
    pub delete_forever: &'static str,
    /// Подпись числа объектов: «объектов: 3».
    pub items: &'static str,
    pub confirm: &'static str,
    pub cancel: &'static str,

    // Ход операции.
    pub job_copy: &'static str,
    pub job_move: &'static str,
    pub job_trash: &'static str,
    pub job_delete: &'static str,
    pub cancelling: &'static str,
    pub conflict_exists: &'static str,
    pub replace: &'static str,
    pub skip: &'static str,
    pub replace_all: &'static str,
    pub skip_all: &'static str,

    // Итог операции.
    pub done_copy: &'static str,
    pub done_move: &'static str,
    pub done_trash: &'static str,
    pub done_delete: &'static str,
    pub skipped: &'static str,
    pub errors: &'static str,
    pub cancelled: &'static str,
    pub job_lost: &'static str,
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

    hint_mark: "mark",
    hint_copy: "copy",
    hint_move_to: "move",
    hint_delete: "delete",
    marked: "marked",
    no_second_panel: "copying and moving need the second panel",
    same_dir: "source and destination are the same directory",
    into_itself: "cannot copy a directory into itself",

    delete_title: "Delete",
    delete_forever_title: "Delete permanently",
    delete_to_trash: "Move to trash?",
    delete_forever: "Delete permanently? This cannot be undone.",
    items: "items",
    confirm: "confirm",
    cancel: "cancel",

    job_copy: "Copying",
    job_move: "Moving",
    job_trash: "Moving to trash",
    job_delete: "Deleting",
    cancelling: "cancelling…",
    conflict_exists: "Already exists:",
    replace: "replace",
    skip: "skip",
    replace_all: "replace all",
    skip_all: "skip all",

    done_copy: "copied",
    done_move: "moved",
    done_trash: "moved to trash",
    done_delete: "deleted",
    skipped: "skipped",
    errors: "errors",
    cancelled: "cancelled",
    job_lost: "the operation stopped unexpectedly",
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

    hint_mark: "выделить",
    hint_copy: "копировать",
    hint_move_to: "перенести",
    hint_delete: "удалить",
    marked: "выделено",
    no_second_panel: "для копирования и переноса нужна вторая панель",
    same_dir: "источник и цель — одна и та же директория",
    into_itself: "нельзя скопировать директорию внутрь самой себя",

    delete_title: "Удаление",
    delete_forever_title: "Безвозвратное удаление",
    delete_to_trash: "Переместить в корзину?",
    delete_forever: "Удалить безвозвратно? Отменить будет нельзя.",
    items: "объектов",
    confirm: "подтвердить",
    cancel: "отмена",

    job_copy: "Копирование",
    job_move: "Перенос",
    job_trash: "Перемещение в корзину",
    job_delete: "Удаление",
    cancelling: "отмена…",
    conflict_exists: "Уже существует:",
    replace: "заменить",
    skip: "пропустить",
    replace_all: "заменить все",
    skip_all: "пропустить все",

    done_copy: "скопировано",
    done_move: "перенесено",
    done_trash: "в корзине",
    done_delete: "удалено",
    skipped: "пропущено",
    errors: "ошибок",
    cancelled: "прервано",
    job_lost: "операция неожиданно остановилась",
};
