//! Поиск в файловой панели: строка запроса, список находок, курсор по ним.
//!
//! Пока поиск открыт, панель показывает не директорию, а находки; вторая
//! панель, если она видна, показывает выбранную находку в просмотре.

use std::path::PathBuf;

use crossbeam_channel::Receiver;

use crate::config::Side;
use crate::panel::scroll_offset;
use crate::search::{Hit, Kind, Msg, Task};

/// Состояние поиска в панели.
pub struct Finder {
    pub kind: Kind,
    /// Панель, в которой открыт поиск.
    pub side: Side,
    /// Где ищем — директория панели на момент открытия.
    pub root: PathBuf,
    pub query: String,
    pub hits: Vec<Hit>,
    pub cursor: usize,
    pub offset: usize,
    /// Высота списка на последнем кадре.
    pub rows: usize,
    /// Поиск ещё идёт.
    pub running: bool,
    /// Просмотрено файлов.
    pub files: u64,
    /// Находок больше, чем показано.
    pub limited: bool,
    /// Просмотр открыт поиском и закроется вместе с ним.
    pub opened_viewer: bool,
    /// Какая находка сейчас показана в просмотре.
    pub shown: Option<(PathBuf, Option<usize>)>,
    show_hidden: bool,
    task: Option<Task>,
}

impl Finder {
    pub fn new(kind: Kind, side: Side, root: PathBuf, show_hidden: bool) -> Self {
        Self {
            kind,
            side,
            root,
            query: String::new(),
            hits: Vec::new(),
            cursor: 0,
            offset: 0,
            rows: 1,
            running: false,
            files: 0,
            limited: false,
            opened_viewer: false,
            shown: None,
            show_hidden,
            task: None,
        }
    }

    /// Новый запрос: прежний поиск бросается, новый запускается с нуля.
    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.hits.clear();
        self.cursor = 0;
        self.offset = 0;
        self.files = 0;
        self.limited = false;
        self.task = (!self.query.is_empty()).then(|| {
            Task::spawn(
                self.kind,
                self.root.clone(),
                self.query.clone(),
                self.show_hidden,
            )
        });
        self.running = self.task.is_some();
    }

    /// Канал текущего поиска, если он идёт.
    pub fn rx(&self) -> Option<Receiver<Msg>> {
        self.task.as_ref().map(|t| t.rx.clone())
    }

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Hits(batch) => self.hits.extend(batch),
            Msg::Progress(files) => self.files = files,
            Msg::Done { files, limited } => {
                self.files = files;
                self.limited = limited;
                self.running = false;
                self.task = None;
            }
        }
    }

    /// Выбранная находка.
    pub fn current(&self) -> Option<&Hit> {
        self.hits.get(self.cursor)
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.hits.is_empty() {
            return;
        }
        let last = self.hits.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.hits.len().saturating_sub(1);
    }

    /// Подгоняет прокрутку под высоту списка; вызывается при отрисовке.
    pub fn fit(&mut self, rows: usize) {
        self.rows = rows.max(1);
        self.offset = scroll_offset(self.offset, self.cursor, self.hits.len(), self.rows, 1);
    }

    /// Путь находки относительно корня поиска.
    pub fn relative(&self, hit: &Hit) -> String {
        hit.path
            .strip_prefix(&self.root)
            .unwrap_or(&hit.path)
            .display()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn finish(finder: &mut Finder) {
        while let Some(rx) = finder.rx() {
            let msg = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            finder.on_msg(msg);
        }
    }

    #[test]
    fn new_query_restarts_search() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "alpha\nbeta\n").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "beta\n").unwrap();
        let mut finder = Finder::new(Kind::Content, Side::Left, tmp.path().into(), true);

        finder.set_query("beta".into());
        finish(&mut finder);
        assert_eq!(finder.hits.len(), 2);
        assert!(!finder.running);
        assert_eq!(finder.files, 2);
        finder.end();
        assert_eq!(finder.cursor, 1);
        assert_eq!(finder.relative(finder.current().unwrap()), "b.txt");

        finder.set_query("alp".into());
        assert_eq!(finder.cursor, 0);
        finish(&mut finder);
        assert_eq!(finder.hits.len(), 1);

        // Пустой запрос ничего не ищет.
        finder.set_query(String::new());
        assert!(finder.rx().is_none());
        assert!(finder.hits.is_empty());
    }
}
