//! Буфер редактора: текст, курсор, правка, отступы, сохранение.
//!
//! Позиция курсора хранится в символах строки, а не в байтах и не в колонках
//! экрана: так правка не разрежет многобайтовый символ, а перевод в колонки
//! с учётом табуляции и широких символов делается только при отрисовке.
//!
//! Подсветка кешируется построчно вместе с состоянием разметчика после каждой
//! строки. Правка в строке `n` обесценивает кеш с `n` и ниже; пересчитывается
//! он лениво — только до последней видимой строки.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use unicode_width::UnicodeWidthChar;

use crate::highlight::{self, Highlighter, Language, Piece};
use crate::indent::{self, Choice};

/// Больше этого файла не редактируем: держим его в памяти целиком.
const EDIT_LIMIT: u64 = 16 * 1024 * 1024;

/// Почему файл нельзя открыть на правку.
#[derive(Debug)]
pub enum OpenError {
    /// Файл не в UTF-8 или двоичный.
    NotText,
    TooLarge,
    Io(io::Error),
}

/// Редактируемый файл.
pub struct Editor {
    pub path: PathBuf,
    pub lines: Vec<String>,
    /// Строка курсора.
    pub row: usize,
    /// Позиция курсора в символах строки.
    pub col: usize,
    /// Колонка экрана, к которой курсор стремится при движении по вертикали.
    goal: Option<usize>,
    /// Первая видимая строка и колонка.
    pub top: usize,
    pub left: usize,
    /// Высота области на последнем кадре.
    pub rows: usize,
    pub indent: Choice,
    /// Разделитель строк файла: `\n` или `\r\n`.
    eol: &'static str,
    /// Файл заканчивался переводом строки.
    final_newline: bool,
    pub modified: bool,
    pub language: Option<&'static Language>,
    /// Кеш подсветки: фрагменты строки и состояние разметчика после неё.
    cache: Vec<(Vec<Piece>, Highlighter)>,
}

impl Editor {
    /// Открывает файл; `default_indent` — отступ из настроек.
    pub fn open(path: &Path, default_indent: usize) -> Result<Self, OpenError> {
        let meta = fs::metadata(path).map_err(OpenError::Io)?;
        if meta.len() > EDIT_LIMIT {
            return Err(OpenError::TooLarge);
        }
        let bytes = fs::read(path).map_err(OpenError::Io)?;
        if bytes.contains(&0) {
            return Err(OpenError::NotText);
        }
        let text = String::from_utf8(bytes).map_err(|_| OpenError::NotText)?;
        Ok(Self::from_text(path, &text, default_indent))
    }

    /// Буфер из готового текста.
    pub fn from_text(path: &Path, text: &str, default_indent: usize) -> Self {
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let final_newline = text.is_empty() || text.ends_with('\n');
        let mut lines: Vec<String> = text
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
            .collect();
        // `split` даёт пустой хвост после завершающего перевода строки.
        if final_newline && lines.len() > 1 {
            lines.pop();
        }
        if lines.is_empty() {
            lines.push(String::new());
        }

        let sample: Vec<&str> = lines.iter().take(30).map(String::as_str).collect();
        let language = highlight::detect_text(path, &sample);
        let indent = indent::choose(path, &lines, language.map(|l| l.name), default_indent);
        Self {
            path: path.to_path_buf(),
            lines,
            row: 0,
            col: 0,
            goal: None,
            top: 0,
            left: 0,
            rows: 1,
            indent,
            eol,
            final_newline,
            modified: false,
            language,
            cache: Vec::new(),
        }
    }

    /// Текст файла целиком, как он будет записан.
    pub fn text(&self) -> String {
        let mut out = self.lines.join(self.eol);
        if self.final_newline {
            out.push_str(self.eol);
        }
        out
    }

    /// Записывает файл: сначала во временный рядом, затем переименованием,
    /// чтобы сбой посреди записи не оставил полфайла. Права сохраняются.
    pub fn save(&mut self) -> io::Result<()> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = dir.join(format!(".{name}.fm-save"));
        let result = (|| {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(self.text().as_bytes())?;
            file.sync_all()?;
            if let Ok(meta) = fs::metadata(&self.path) {
                fs::set_permissions(&tmp, meta.permissions())?;
            }
            fs::rename(&tmp, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result?;
        self.modified = false;
        Ok(())
    }

    fn line_len(&self, row: usize) -> usize {
        self.lines[row].chars().count()
    }

    /// Байтовое смещение символа `col` в строке `row`.
    fn byte_at(&self, row: usize, col: usize) -> usize {
        let line = &self.lines[row];
        line.char_indices().nth(col).map_or(line.len(), |(i, _)| i)
    }

    /// Колонка экрана символа `col` строки `row` с учётом табуляции.
    pub fn screen_col(&self, row: usize, col: usize) -> usize {
        let mut x = 0;
        for ch in self.lines[row].chars().take(col) {
            x += self.char_width(ch, x);
        }
        x
    }

    /// Ширина символа на экране в колонке `x`.
    pub fn char_width(&self, ch: char, x: usize) -> usize {
        if ch == '\t' {
            let tw = self.indent.tab_width.max(1);
            tw - x % tw
        } else {
            ch.width().unwrap_or(0)
        }
    }

    /// Символ строки `row`, ближайший к колонке экрана `x`.
    fn col_at_screen(&self, row: usize, target: usize) -> usize {
        let mut x = 0;
        for (i, ch) in self.lines[row].chars().enumerate() {
            let w = self.char_width(ch, x);
            if x + w > target {
                return i;
            }
            x += w;
        }
        self.line_len(row)
    }

    /// Отмечает изменение строки `row`.
    fn touched(&mut self, row: usize) {
        self.modified = true;
        self.cache.truncate(row);
    }

    // ── движение ────────────────────────────────────────────────────────────

    pub fn up(&mut self, n: usize) {
        let goal = self
            .goal
            .unwrap_or_else(|| self.screen_col(self.row, self.col));
        self.row = self.row.saturating_sub(n);
        self.col = self.col_at_screen(self.row, goal);
        self.goal = Some(goal);
    }

    pub fn down(&mut self, n: usize) {
        let goal = self
            .goal
            .unwrap_or_else(|| self.screen_col(self.row, self.col));
        self.row = (self.row + n).min(self.lines.len() - 1);
        self.col = self.col_at_screen(self.row, goal);
        self.goal = Some(goal);
    }

    pub fn left(&mut self) {
        self.goal = None;
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.line_len(self.row);
        }
    }

    pub fn right(&mut self) {
        self.goal = None;
        if self.col < self.line_len(self.row) {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    /// Home: к первому непробельному символу, повторно — в начало строки.
    pub fn home(&mut self) {
        self.goal = None;
        let first = self.lines[self.row]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        self.col = if self.col == first { 0 } else { first };
    }

    pub fn end(&mut self) {
        self.goal = None;
        self.col = self.line_len(self.row);
    }

    pub fn file_start(&mut self) {
        self.goal = None;
        self.row = 0;
        self.col = 0;
    }

    pub fn file_end(&mut self) {
        self.goal = None;
        self.row = self.lines.len() - 1;
        self.col = self.line_len(self.row);
    }

    // ── правка ──────────────────────────────────────────────────────────────

    /// Вставляет символ. Закрывающая скобка в строке из одних пробелов
    /// снимает один уровень отступа — как в большинстве редакторов кода.
    pub fn insert(&mut self, ch: char) {
        self.goal = None;
        if matches!(ch, '}' | ')' | ']') && self.before_cursor().trim().is_empty() && self.col > 0 {
            self.dedent_line(self.row);
            self.col = self.line_len(self.row);
        }
        let at = self.byte_at(self.row, self.col);
        self.lines[self.row].insert(at, ch);
        self.col += 1;
        self.touched(self.row);
    }

    /// Текст строки до курсора.
    fn before_cursor(&self) -> &str {
        &self.lines[self.row][..self.byte_at(self.row, self.col)]
    }

    /// Enter: перенос строки с сохранением отступа. После открывающей скобки
    /// (или двоеточия в Python и YAML) отступ увеличивается; если сразу за
    /// курсором стоит парная закрывающая, она уходит на свою строку.
    pub fn newline(&mut self) {
        self.goal = None;
        let at = self.byte_at(self.row, self.col);
        let tail = self.lines[self.row].split_off(at);
        let head = self.lines[self.row].clone();
        let base: String = head
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();

        let trimmed = head.trim_end();
        let colon = matches!(self.language.map(|l| l.name), Some("Python" | "YAML"));
        let opens = trimmed.ends_with(['{', '(', '[']) || (colon && trimmed.ends_with(':'));
        let unit = self.indent.indent.unit();

        // Пробелы, на которых стоял курсор, переносить незачем.
        let tail = tail.trim_start_matches([' ', '\t']).to_string();
        let mut row = self.row + 1;
        if opens {
            let inner = format!("{base}{unit}");
            let closes = matches!(
                (trimmed.chars().last(), tail.chars().next()),
                (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']'))
            );
            self.lines.insert(row, inner.clone());
            if closes {
                self.lines.insert(row + 1, format!("{base}{tail}"));
            } else {
                self.lines[row].push_str(&tail);
            }
            self.col = inner.chars().count();
        } else {
            self.lines.insert(row, format!("{base}{tail}"));
            self.col = base.chars().count();
        }
        // Строка, от которой отломили хвост, не должна кончаться пробелами.
        let kept = self.lines[self.row].trim_end().len();
        if self.lines[self.row].trim().is_empty() {
            self.lines[self.row].clear();
        } else {
            self.lines[self.row].truncate(kept);
        }
        std::mem::swap(&mut self.row, &mut row);
        self.touched(row);
    }

    /// Backspace. В отступе из пробелов удаляет до предыдущей ступени,
    /// в начале строки склеивает её с предыдущей.
    pub fn backspace(&mut self) {
        self.goal = None;
        if self.col == 0 {
            if self.row == 0 {
                return;
            }
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.line_len(self.row);
            self.lines[self.row].push_str(&line);
            self.touched(self.row);
            return;
        }
        let before = self.before_cursor();
        let n =
            if !self.indent.indent.tabs && !before.is_empty() && before.chars().all(|c| c == ' ') {
                let size = self.indent.indent.size.max(1);
                match self.col % size {
                    0 => size,
                    r => r,
                }
            } else {
                1
            };
        let from = self.byte_at(self.row, self.col - n);
        let to = self.byte_at(self.row, self.col);
        self.lines[self.row].replace_range(from..to, "");
        self.col -= n;
        self.touched(self.row);
    }

    /// Delete: удаляет символ под курсором или склеивает со следующей строкой.
    pub fn delete(&mut self) {
        self.goal = None;
        if self.col < self.line_len(self.row) {
            let from = self.byte_at(self.row, self.col);
            let to = self.byte_at(self.row, self.col + 1);
            self.lines[self.row].replace_range(from..to, "");
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        } else {
            return;
        }
        self.touched(self.row);
    }

    /// Tab: вставляет отступ до следующей ступени (или символ табуляции).
    pub fn tab(&mut self) {
        self.goal = None;
        let text = if self.indent.indent.tabs {
            "\t".to_string()
        } else {
            let size = self.indent.indent.size.max(1);
            let x = self.screen_col(self.row, self.col);
            " ".repeat(size - x % size)
        };
        let at = self.byte_at(self.row, self.col);
        self.lines[self.row].insert_str(at, &text);
        self.col += text.chars().count();
        self.touched(self.row);
    }

    /// Shift+Tab: снимает один уровень отступа со строки курсора.
    pub fn backtab(&mut self) {
        self.goal = None;
        let removed = self.dedent_line(self.row);
        self.col = self.col.saturating_sub(removed);
        if removed > 0 {
            self.touched(self.row);
        }
    }

    /// Снимает с начала строки один уровень отступа; возвращает число
    /// удалённых символов.
    fn dedent_line(&mut self, row: usize) -> usize {
        let line = &self.lines[row];
        let n = if line.starts_with('\t') {
            1
        } else {
            let spaces = line.chars().take_while(|&c| c == ' ').count();
            spaces.min(self.indent.indent.size.max(1))
        };
        self.lines[row].replace_range(..n, "");
        n
    }

    // ── прокрутка и подсветка ───────────────────────────────────────────────

    /// Подгоняет прокрутку так, чтобы курсор был виден в области `rows`×`cols`.
    pub fn fit(&mut self, rows: usize, cols: usize) {
        self.rows = rows.max(1);
        if self.row < self.top {
            self.top = self.row;
        } else if self.row >= self.top + self.rows {
            self.top = self.row + 1 - self.rows;
        }
        let x = self.screen_col(self.row, self.col);
        let cols = cols.max(1);
        if x < self.left {
            self.left = x;
        } else if x >= self.left + cols {
            self.left = x + 1 - cols;
        }
    }

    /// Подсвеченные фрагменты строки `row`; кеш досчитывается до неё.
    pub fn highlighted(&mut self, row: usize) -> &[Piece] {
        while self.cache.len() <= row {
            let i = self.cache.len();
            let mut h = match self.cache.last() {
                Some((_, state)) => state.clone(),
                None => Highlighter::new(self.language),
            };
            let pieces = h.line(&self.lines[i]);
            self.cache.push((pieces, h));
        }
        &self.cache[row].0
    }

    /// Человекочитаемый разделитель строк.
    pub fn eol_name(&self) -> &'static str {
        if self.eol == "\r\n" { "CRLF" } else { "LF" }
    }

    /// Задаёт размер отступа явно (для тестов и будущих настроек).
    #[cfg(test)]
    pub fn set_indent(&mut self, indent: crate::indent::Indent) {
        self.indent.indent = indent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indent::Indent;

    fn ed(path: &str, text: &str) -> Editor {
        Editor::from_text(Path::new(path), text, 4)
    }

    fn type_str(e: &mut Editor, s: &str) {
        for ch in s.chars() {
            match ch {
                '\n' => e.newline(),
                c => e.insert(c),
            }
        }
    }

    #[test]
    fn roundtrip_keeps_line_endings() {
        assert_eq!(ed("a", "x\ny\n").text(), "x\ny\n");
        assert_eq!(ed("a", "x\ny").text(), "x\ny");
        assert_eq!(ed("a", "x\r\ny\r\n").text(), "x\r\ny\r\n");
        assert_eq!(ed("a", "").text(), "\n");
        assert_eq!(ed("a", "x\r\n").eol_name(), "CRLF");
    }

    #[test]
    fn typing_with_auto_indent_in_braces() {
        let mut e = ed("main.rs", "");
        type_str(&mut e, "fn main() {\nlet x = 1;\n}");
        assert_eq!(e.lines, ["fn main() {", "    let x = 1;", "}"]);
        assert!(e.modified);
    }

    #[test]
    fn enter_between_braces_splits_them() {
        let mut e = ed("a.js", "if (x) {}\n");
        e.set_indent(Indent::spaces(2));
        e.end();
        e.left();
        e.newline();
        assert_eq!(e.lines, ["if (x) {", "  ", "}"]);
        assert_eq!((e.row, e.col), (1, 2));
    }

    #[test]
    fn python_colon_indents() {
        let mut e = ed("a.py", "");
        type_str(&mut e, "def f():\nreturn 1");
        assert_eq!(e.lines, ["def f():", "    return 1"]);
    }

    #[test]
    fn enter_keeps_indent_and_trims_trailing_spaces() {
        let mut e = ed("a.txt", "    abc def");
        e.end();
        for _ in 0..4 {
            e.left();
        }
        e.insert(' ');
        e.newline();
        assert_eq!(e.lines, ["    abc", "    def"]);
        assert_eq!(e.col, 4);
    }

    #[test]
    fn backspace_removes_whole_indent_step() {
        let mut e = ed("a.rs", "        x");
        // Единственная строка с отступом 8 — автоопределение дало бы шаг 8.
        e.set_indent(Indent::spaces(4));
        e.home();
        e.backspace();
        assert_eq!(e.lines[0], "    x");
        e.backspace();
        assert_eq!(e.lines[0], "x");
        // В середине текста — по одному символу.
        e.end();
        e.backspace();
        assert_eq!(e.lines[0], "");
    }

    #[test]
    fn backspace_and_delete_join_lines() {
        let mut e = ed("a", "ab\ncd\n");
        e.down(1);
        e.backspace();
        assert_eq!(e.lines, ["abcd"]);
        assert_eq!(e.col, 2);
        let mut e = ed("a", "ab\ncd\n");
        e.end();
        e.delete();
        assert_eq!(e.lines, ["abcd"]);
    }

    #[test]
    fn closing_brace_dedents() {
        let mut e = ed("a.rs", "fn x() {\n    \n");
        e.down(1);
        e.end();
        e.insert('}');
        assert_eq!(e.lines[1], "}");
    }

    #[test]
    fn tab_and_backtab() {
        let mut e = ed("a.rs", "x");
        e.end();
        e.tab(); // колонка 1 → 4
        assert_eq!(e.lines[0], "x   ");
        e.home();
        e.tab();
        assert_eq!(e.lines[0], "    x   ");
        e.backtab();
        assert_eq!(e.lines[0], "x   ");

        let mut e = ed("main.go", "");
        e.tab();
        assert_eq!(e.lines[0], "\t");
    }

    #[test]
    fn vertical_moves_keep_goal_column() {
        let mut e = ed("a", "abcdef\nab\nabcdef\n");
        e.end();
        e.down(1);
        assert_eq!(e.col, 2);
        e.down(1);
        assert_eq!(e.col, 6);
        e.up(5);
        assert_eq!((e.row, e.col), (0, 6));
    }

    #[test]
    fn unicode_is_edited_by_chars() {
        let mut e = ed("a", "привет");
        e.end();
        e.backspace();
        e.insert('!');
        assert_eq!(e.lines[0], "приве!");
        e.home();
        e.delete();
        assert_eq!(e.lines[0], "риве!");
    }

    #[test]
    fn tabs_are_measured_by_tab_stops() {
        let e = ed("main.go", "\tx\n");
        assert_eq!(e.screen_col(0, 1), 4);
        assert_eq!(e.col_at_screen(0, 2), 0);
        assert_eq!(e.col_at_screen(0, 4), 1);
    }

    #[test]
    fn highlight_cache_follows_edits() {
        let mut e = ed("a.rs", "/* a\nb */ let x;\n");
        assert_eq!(e.highlighted(1)[0].kind, highlight::Token::Comment);
        // Закрываем комментарий в первой строке — вторая перестаёт быть им.
        e.end();
        type_str(&mut e, " */");
        assert_ne!(e.highlighted(1)[0].kind, highlight::Token::Comment);
    }

    #[test]
    fn fit_scrolls_to_cursor() {
        let text: String = (0..100).map(|i| format!("{i}\n")).collect();
        let mut e = ed("a", &text);
        e.down(50);
        e.fit(10, 20);
        assert_eq!(e.top, 41);
        e.up(45);
        e.fit(10, 20);
        assert_eq!(e.top, 5);
    }

    #[test]
    fn save_writes_atomically_and_keeps_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("run.sh");
        fs::write(&path, "echo 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut e = Editor::open(&path, 4).unwrap();
        e.end();
        e.insert('2');
        e.save().unwrap();
        assert!(!e.modified);
        assert_eq!(fs::read_to_string(&path).unwrap(), "echo 12\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        // Временный файл не остался.
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn binary_and_invalid_utf8_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("b"), [0u8, 1, 2]).unwrap();
        fs::write(tmp.path().join("l"), [0xffu8, 0xfe]).unwrap();
        assert!(matches!(
            Editor::open(&tmp.path().join("b"), 4),
            Err(OpenError::NotText)
        ));
        assert!(matches!(
            Editor::open(&tmp.path().join("l"), 4),
            Err(OpenError::NotText)
        ));
    }
}
