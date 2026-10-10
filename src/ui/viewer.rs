//! Панель просмотра: что показано, куда прокручено, что найдено.
//!
//! Прокрутка считается в строках файла, а не в экранных строках: при переносе
//! одна строка файла может занять несколько строк экрана, и сколько именно,
//! известно только при отрисовке. Поэтому отрисовка сообщает панели высоту
//! области и предельную прокрутку, а та приводит к ним своё положение.

use std::path::PathBuf;

use crate::config::Side;
use crate::preview::{Body, Preview};
use crate::search::Needle;

/// Шаг горизонтальной прокрутки без переноса строк.
pub const HSCROLL_STEP: usize = 8;

/// Совпадение при поиске: строка и диапазон символов в ней.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub start: usize,
    pub len: usize,
}

/// Поиск по тексту просмотра.
#[derive(Debug, Default)]
pub struct Search {
    pub query: String,
    /// Совпадения по порядку: строка, затем позиция.
    pub matches: Vec<Match>,
    /// Индекс текущего совпадения в `matches`.
    pub current: usize,
}

impl Search {
    /// Совпадения в строке `line` (срез `matches`).
    pub fn in_line(&self, line: usize) -> &[Match] {
        let from = self.matches.partition_point(|m| m.line < line);
        let to = self.matches.partition_point(|m| m.line <= line);
        &self.matches[from..to]
    }

    /// Текущее совпадение.
    pub fn current(&self) -> Option<Match> {
        self.matches.get(self.current).copied()
    }
}

/// Состояние панели просмотра.
#[derive(Debug)]
pub struct Viewer {
    /// Сторона окна, на которой показан просмотр.
    pub side: Side,
    /// Запрошенный путь.
    pub path: Option<PathBuf>,
    /// Номер запроса к загрузчику: результаты старых запросов отбрасываются.
    pub request: u64,
    /// Загруженное содержимое; `None`, пока идёт загрузка.
    pub content: Option<Preview>,
    /// Первая видимая строка.
    pub top: usize,
    /// Сдвиг по горизонтали в колонках (только без переноса).
    pub left: usize,
    pub wrap: bool,
    /// Высота области на последнем кадре.
    pub rows: usize,
    /// Наибольшая допустимая `top` на последнем кадре.
    pub max_top: usize,
    /// Открытый поиск.
    pub search: Option<Search>,
    /// Найденная строка, которую показать, когда файл загрузится:
    /// запрос и номер строки.
    pending: Option<(String, usize)>,
}

impl Viewer {
    pub fn new(side: Side, wrap: bool) -> Self {
        Self {
            side,
            path: None,
            request: 0,
            content: None,
            top: 0,
            left: 0,
            wrap,
            rows: 1,
            max_top: 0,
            search: None,
            pending: None,
        }
    }

    /// Начинает показ нового пути.
    pub fn show(&mut self, path: PathBuf, request: u64) {
        self.path = Some(path);
        self.request = request;
        self.content = None;
        self.top = 0;
        self.left = 0;
        self.search = None;
        self.pending = None;
    }

    /// Принимает результат загрузчика, если он ответ на последний запрос.
    pub fn loaded(&mut self, request: u64, preview: Preview) -> bool {
        if request != self.request {
            return false;
        }
        self.content = Some(preview);
        if let Some((query, line)) = self.pending.take() {
            self.focus(&query, line);
        }
        true
    }

    /// Показывает найденное: подсвечивает все вхождения `query` и ставит
    /// в центр строку `line`. Если файл ещё грузится — сделает это по загрузке.
    pub fn focus(&mut self, query: &str, line: usize) {
        if self.content.is_none() {
            self.pending = Some((query.to_string(), line));
            return;
        }
        if !self.start_search() {
            return;
        }
        self.set_query(query.to_string());
        if let Some(search) = &mut self.search
            && let Some(i) = search.matches.iter().position(|m| m.line >= line)
        {
            search.current = i;
        }
        self.center(line);
    }

    /// Сообщает панели размеры области; вызывается при отрисовке.
    pub fn fit(&mut self, rows: usize, max_top: usize) {
        self.rows = rows.max(1);
        self.max_top = max_top;
        self.top = self.top.min(max_top);
    }

    pub fn scroll(&mut self, delta: isize) {
        self.top = (self.top as isize + delta).clamp(0, self.max_top as isize) as usize;
    }

    pub fn page(&mut self, pages: isize) {
        self.scroll(pages * self.rows as isize);
    }

    pub fn home(&mut self) {
        self.top = 0;
        self.left = 0;
    }

    pub fn end(&mut self) {
        self.top = self.max_top;
    }

    /// Горизонтальная прокрутка; при переносе строк она не нужна.
    pub fn hscroll(&mut self, delta: isize) {
        if self.wrap {
            return;
        }
        self.left = (self.left as isize + delta).max(0) as usize;
    }

    pub fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        self.left = 0;
    }

    /// Ставит строку `line` в середину экрана. Предел прокрутки применит
    /// следующая отрисовка.
    pub fn center(&mut self, line: usize) {
        self.top = line.saturating_sub(self.rows / 2);
    }

    /// Открывает поиск. Искать можно только в тексте.
    pub fn start_search(&mut self) -> bool {
        if !matches!(self.content.as_ref().map(|c| &c.body), Some(Body::Text(_))) {
            return false;
        }
        self.search = Some(Search::default());
        true
    }

    /// Новый текст запроса: совпадения пересчитываются, и показывается первое
    /// из них, начиная с текущего положения.
    pub fn set_query(&mut self, query: String) {
        let Some(Body::Text(text)) = self.content.as_ref().map(|c| &c.body) else {
            return;
        };
        let lines = (0..text.lines.len()).map(|i| text.plain(i));
        let matches = find_all(lines, &query);
        let from = self.top;
        let current = matches.iter().position(|m| m.line >= from).unwrap_or(0);
        let target = matches.get(current).map(|m| m.line);
        self.search = Some(Search {
            query,
            matches,
            current,
        });
        if let Some(line) = target {
            self.center(line);
        }
    }

    /// Переходит к следующему (`+1`) или предыдущему (`-1`) совпадению по кругу.
    pub fn jump(&mut self, step: isize) {
        let Some(search) = &mut self.search else {
            return;
        };
        let n = search.matches.len();
        if n == 0 {
            return;
        }
        search.current = (search.current as isize + step).rem_euclid(n as isize) as usize;
        let line = search.matches[search.current].line;
        self.center(line);
    }
}

/// Все вхождения `query` в строках; правила сравнения — [`Needle`].
pub fn find_all(lines: impl Iterator<Item = String>, query: &str) -> Vec<Match> {
    let needle = Needle::new(query);
    let mut out = Vec::new();
    for (line, text) in lines.enumerate() {
        out.extend(
            needle
                .find(&text)
                .into_iter()
                .map(|(start, len)| Match { line, start, len }),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::text_body;
    use std::path::Path;

    fn lines(text: &[&str]) -> impl Iterator<Item = String> {
        text.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn find_is_smart_case() {
        let text = ["Foo foo FOO", "nothing", "фу ФУ"];
        assert_eq!(find_all(lines(&text), "foo").len(), 3);
        assert_eq!(find_all(lines(&text), "Foo").len(), 1);
        assert_eq!(find_all(lines(&text), "фу").len(), 2);
        assert!(find_all(lines(&text), "").is_empty());
        let m = find_all(lines(&text), "ФУ");
        assert_eq!(
            m,
            [Match {
                line: 2,
                start: 3,
                len: 2
            }]
        );
    }

    #[test]
    fn matches_do_not_overlap() {
        assert_eq!(find_all(lines(&["aaaa"]), "aa").len(), 2);
    }

    fn viewer_with(text: &str, rows: usize) -> Viewer {
        let mut viewer = Viewer::new(Side::Right, true);
        viewer.show(PathBuf::from("/x.txt"), 1);
        let body = text_body(Path::new("/x.txt"), text, false);
        let n = body.lines.len();
        viewer.loaded(
            1,
            Preview {
                path: PathBuf::from("/x.txt"),
                size: 0,
                modified: None,
                mode: String::new(),
                owner: String::new(),
                body: Body::Text(body),
            },
        );
        viewer.fit(rows, n.saturating_sub(rows));
        viewer
    }

    #[test]
    fn search_centers_and_cycles() {
        let text: String = (0..100)
            .map(|i| {
                if i % 30 == 0 {
                    format!("hit {i}\n")
                } else {
                    format!("line {i}\n")
                }
            })
            .collect();
        let mut viewer = viewer_with(&text, 10);
        assert!(viewer.start_search());
        viewer.set_query("hit".into());
        let search = viewer.search.as_ref().unwrap();
        assert_eq!(search.matches.len(), 4);
        assert_eq!(viewer.top, 0); // строка 0 уже наверху

        viewer.jump(1);
        assert_eq!(viewer.top, 30 - 5);
        viewer.jump(1);
        viewer.jump(1);
        assert_eq!(viewer.search.as_ref().unwrap().current, 3);
        // По кругу на первое.
        viewer.jump(1);
        assert_eq!(viewer.search.as_ref().unwrap().current, 0);
        viewer.jump(-1);
        assert_eq!(viewer.search.as_ref().unwrap().current, 3);
    }

    #[test]
    fn search_starts_from_current_position() {
        let text: String = (0..100).map(|i| format!("hit {i}\n")).collect();
        let mut viewer = viewer_with(&text, 10);
        viewer.top = 50;
        viewer.start_search();
        viewer.set_query("hit".into());
        assert_eq!(viewer.search.as_ref().unwrap().current, 50);
        assert_eq!(viewer.search.as_ref().unwrap().in_line(50).len(), 1);
    }

    #[test]
    fn scrolling_is_clamped() {
        let text: String = (0..30).map(|i| format!("{i}\n")).collect();
        let mut viewer = viewer_with(&text, 10);
        viewer.scroll(-5);
        assert_eq!(viewer.top, 0);
        viewer.page(5);
        assert_eq!(viewer.top, 20);
        viewer.home();
        viewer.end();
        assert_eq!(viewer.top, 20);
    }

    #[test]
    fn horizontal_scroll_only_without_wrap() {
        let mut viewer = viewer_with("long line", 10);
        viewer.hscroll(8);
        assert_eq!(viewer.left, 0);
        viewer.toggle_wrap();
        viewer.hscroll(8);
        viewer.hscroll(-20);
        assert_eq!(viewer.left, 0);
        viewer.hscroll(16);
        assert_eq!(viewer.left, 16);
    }

    #[test]
    fn focus_waits_for_content() {
        let mut viewer = Viewer::new(Side::Right, true);
        viewer.show(PathBuf::from("/x.txt"), 1);
        viewer.rows = 10;
        viewer.focus("hit", 40);
        assert!(viewer.search.is_none());

        let text: String = (0..100)
            .map(|i| {
                if i % 20 == 0 {
                    format!("hit {i}\n")
                } else {
                    format!("{i}\n")
                }
            })
            .collect();
        let body = text_body(Path::new("/x.txt"), &text, false);
        viewer.loaded(
            1,
            Preview {
                path: PathBuf::from("/x.txt"),
                size: 0,
                modified: None,
                mode: String::new(),
                owner: String::new(),
                body: Body::Text(body),
            },
        );
        let search = viewer.search.as_ref().unwrap();
        assert_eq!(search.current().unwrap().line, 40);
        assert_eq!(viewer.top, 35);
    }
}
