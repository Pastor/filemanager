//! Состояние одной файловой панели: текущая директория, курсор, прокрутка.
//!
//! Элементы раскладываются по колонкам сверху вниз, как у `ls`: при нескольких
//! колонках курсор, дойдя до низа одной колонки, переходит в верх следующей.
//! Прокрутка поэтому идёт целыми колонками — `offset` кратен высоте. Одна
//! колонка прокручивается построчно.
//!
//! Архив открывается как директория: он распаковывается во временную папку,
//! и панель запоминает это «монтирование». В заголовке показывается путь
//! через архив (`~/x/a.zip/src`), а при выходе из архива временная папка
//! удаляется.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use crate::config::clamp_columns;
use crate::listing::{self, Entry};

/// Файловая панель.
#[derive(Debug)]
pub struct Panel {
    /// Текущая директория. Хранится как путь, по которому сюда пришли, а не
    /// канонический: выход из директории за ссылкой ведёт туда, откуда вошли.
    pub dir: PathBuf,
    /// Содержимое директории в порядке показа.
    pub entries: Vec<Entry>,
    /// Индекс элемента под курсором.
    pub cursor: usize,
    /// Индекс первого видимого элемента.
    pub offset: usize,
    /// Число колонок.
    pub columns: u8,
    /// Высота колонки на последнем кадре — шаг для PageUp/PageDown.
    pub rows: usize,
    /// Имена выделенных элементов текущей директории.
    pub marked: HashSet<String>,
    /// Открытые архивы, от внешнего к вложенному.
    pub mounts: Vec<Mount>,
}

/// Архив, открытый как директория.
#[derive(Debug, Clone)]
pub struct Mount {
    /// Файл архива.
    pub archive: PathBuf,
    /// Временная директория с его содержимым.
    pub root: PathBuf,
}

/// Общая временная директория для распакованных архивов этого процесса.
pub fn mount_base() -> PathBuf {
    std::env::temp_dir().join(format!("fm-{}", std::process::id()))
}

/// Путь лежит внутри распакованного архива: изменения там не попадут в архив.
pub fn in_archive(path: &Path) -> bool {
    path.starts_with(mount_base())
}

impl Panel {
    /// Создаёт панель и читает директорию.
    pub fn open(dir: PathBuf, columns: u8, show_hidden: bool) -> io::Result<Self> {
        let mut panel = Self {
            dir,
            entries: Vec::new(),
            cursor: 0,
            offset: 0,
            columns: clamp_columns(columns),
            rows: 1,
            marked: HashSet::new(),
            mounts: Vec::new(),
        };
        panel.reload(show_hidden)?;
        Ok(panel)
    }

    /// Пустая панель на случай, если директорию прочитать не удалось.
    pub fn empty(dir: PathBuf, columns: u8) -> Self {
        Self {
            dir,
            entries: Vec::new(),
            cursor: 0,
            offset: 0,
            columns: clamp_columns(columns),
            rows: 1,
            marked: HashSet::new(),
            mounts: Vec::new(),
        }
    }

    /// Элемент под курсором.
    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    /// Перечитывает директорию, по возможности оставляя курсор на том же имени.
    pub fn reload(&mut self, show_hidden: bool) -> io::Result<()> {
        let keep = self.current().map(|e| e.name.clone());
        self.entries = listing::read_dir(&self.dir, show_hidden)?;
        // Выделение исчезнувших (или спрятанных) элементов теряет смысл.
        let names: HashSet<&str> = self.entries.iter().map(|e| e.name.as_str()).collect();
        self.marked.retain(|name| names.contains(name.as_str()));
        match keep {
            Some(name) => self.select_name(&name),
            None => self.cursor = 0,
        }
        self.clamp_cursor();
        Ok(())
    }

    /// Переходит в директорию `dir`. При ошибке чтения панель не меняется.
    pub fn cd(&mut self, dir: PathBuf, show_hidden: bool) -> io::Result<()> {
        let entries = listing::read_dir(&dir, show_hidden)?;
        self.dir = dir;
        self.entries = entries;
        self.marked.clear();
        self.cursor = 0;
        self.offset = 0;
        self.unmount_left();
        Ok(())
    }

    /// Входит в распакованный архив: `root` — его временная директория.
    pub fn mount(&mut self, archive: PathBuf, root: PathBuf, show_hidden: bool) -> io::Result<()> {
        self.mounts.push(Mount {
            archive,
            root: root.clone(),
        });
        if let Err(err) = self.cd(root, show_hidden) {
            if let Some(mount) = self.mounts.pop() {
                let _ = std::fs::remove_dir_all(mount.root);
            }
            return Err(err);
        }
        Ok(())
    }

    /// Закрывает архивы, из которых панель ушла, и удаляет их временные папки.
    fn unmount_left(&mut self) {
        while let Some(mount) = self.mounts.last() {
            if self.dir.starts_with(&mount.root) {
                break;
            }
            let _ = std::fs::remove_dir_all(&mount.root);
            self.mounts.pop();
        }
    }

    /// Путь для показа: временные директории архивов заменены путями к архивам.
    pub fn display_path(&self, path: &Path) -> PathBuf {
        let mut path = path.to_path_buf();
        // Изнутри наружу: путь во вложенном архиве сначала становится путём
        // во внешнем, а тот — обычным путём.
        for mount in self.mounts.iter().rev() {
            if let Ok(rel) = path.strip_prefix(&mount.root) {
                path = mount.archive.join(rel);
            }
        }
        path
    }

    /// Текущая директория для показа.
    pub fn display_dir(&self) -> PathBuf {
        self.display_path(&self.dir)
    }

    /// Входит в директорию под курсором. `Ok(false)` — под курсором не директория.
    pub fn enter(&mut self, show_hidden: bool) -> io::Result<bool> {
        let Some(entry) = self.current() else {
            return Ok(false);
        };
        if !entry.kind.is_dir() {
            return Ok(false);
        }
        let dir = entry.path.clone();
        self.cd(dir, show_hidden)?;
        Ok(true)
    }

    /// Поднимается на уровень выше и ставит курсор на директорию, из которой
    /// вышли. `Ok(false)` — выше подниматься некуда.
    pub fn leave(&mut self, show_hidden: bool) -> io::Result<bool> {
        // Из корня архива выходим туда, где лежит сам архив.
        let from = match self.mounts.last() {
            Some(mount) if mount.root == self.dir => mount.archive.clone(),
            _ => self.dir.clone(),
        };
        let Some(parent) = from.parent().map(Path::to_path_buf) else {
            return Ok(false);
        };
        let came_from = from.file_name().map(|n| n.to_string_lossy().into_owned());
        self.cd(parent, show_hidden)?;
        if let Some(name) = came_from {
            self.select_name(&name);
        }
        Ok(true)
    }

    /// Переключает выделение элемента под курсором и сдвигает курсор вниз.
    pub fn toggle_mark(&mut self) {
        let Some(name) = self.current().map(|e| e.name.clone()) else {
            return;
        };
        if !self.marked.remove(&name) {
            self.marked.insert(name);
        }
        self.move_by(1);
    }

    /// Элемент выделен.
    pub fn is_marked(&self, entry: &Entry) -> bool {
        self.marked.contains(&entry.name)
    }

    /// Пути, к которым применяется операция: выделенные элементы в порядке
    /// показа, а если выделения нет — элемент под курсором.
    pub fn targets(&self) -> Vec<PathBuf> {
        if self.marked.is_empty() {
            return self.current().map(|e| e.path.clone()).into_iter().collect();
        }
        self.entries
            .iter()
            .filter(|e| self.is_marked(e))
            .map(|e| e.path.clone())
            .collect()
    }

    /// Суммарный размер выделенных файлов (без содержимого директорий).
    pub fn marked_size(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| self.is_marked(e) && !e.kind.is_dir())
            .map(|e| e.size)
            .sum()
    }

    /// Ставит курсор на элемент с данным именем, если он есть.
    pub fn select_name(&mut self, name: &str) {
        if let Some(i) = self.entries.iter().position(|e| e.name == name) {
            self.cursor = i;
        }
    }

    /// Сдвигает курсор на `delta` элементов с упором в края списка.
    pub fn move_by(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    /// Курсор на первый элемент.
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Курсор на последний элемент.
    pub fn end(&mut self) {
        self.cursor = self.entries.len().saturating_sub(1);
    }

    /// Задаёт число колонок.
    pub fn set_columns(&mut self, n: u8) {
        self.columns = clamp_columns(n);
    }

    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
    }

    /// Подгоняет прокрутку под видимую область `rows` строк и запоминает её
    /// высоту. Вызывается при отрисовке, когда размер области известен.
    pub fn fit(&mut self, rows: usize) {
        self.rows = rows.max(1);
        self.offset = scroll_offset(
            self.offset,
            self.cursor,
            self.entries.len(),
            self.rows,
            self.columns as usize,
        );
    }
}

impl Drop for Panel {
    /// Временные папки архивов не должны пережить панель.
    fn drop(&mut self) {
        for mount in &self.mounts {
            let _ = std::fs::remove_dir_all(&mount.root);
        }
    }
}

/// Смещение прокрутки для раскладки по колонкам.
///
/// Видно `columns` колонок по `rows` строк, начиная с элемента `offset`.
/// Смещение выравнивается на начало колонки, сдвигается ровно настолько, чтобы
/// курсор оказался на экране, и не оставляет пустых колонок справа, если
/// элементы можно показать и без них.
pub fn scroll_offset(
    offset: usize,
    cursor: usize,
    len: usize,
    rows: usize,
    columns: usize,
) -> usize {
    if len == 0 {
        return 0;
    }
    let rows = rows.max(1);
    let columns = columns.max(1);
    let cursor = cursor.min(len - 1);

    // Одна колонка прокручивается построчно: так глазу проще не терять курсор.
    if columns == 1 {
        let offset = if cursor < offset {
            cursor
        } else if cursor >= offset + rows {
            cursor + 1 - rows
        } else {
            offset
        };
        return offset.min(len.saturating_sub(rows));
    }

    let page = rows * columns;
    let mut offset = offset / rows * rows;
    if cursor < offset {
        offset = cursor / rows * rows;
    } else if cursor >= offset + page {
        offset = (cursor / rows + 1 - columns) * rows;
    }

    // Начало последней колонки: дальше него прокручивать незачем.
    let last_column = (len - 1) / rows * rows;
    let max_offset = last_column.saturating_sub((columns - 1) * rows);
    offset.min(max_offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn single_column_scrolls_line_by_line() {
        // 10 строк на экране, курсор уходит вниз — смещение ровно на выход.
        assert_eq!(scroll_offset(0, 9, 100, 10, 1), 0);
        assert_eq!(scroll_offset(0, 10, 100, 10, 1), 1);
        assert_eq!(scroll_offset(10, 9, 100, 10, 1), 9);
        assert_eq!(scroll_offset(0, 99, 100, 10, 1), 90);
        // Список короче экрана не прокручивается.
        assert_eq!(scroll_offset(3, 4, 5, 10, 1), 0);
    }

    #[test]
    fn columns_scroll_by_whole_column() {
        // 3 колонки по 10: видны 0..30; курсор 30 — сдвиг на одну колонку.
        assert_eq!(scroll_offset(0, 29, 100, 10, 3), 0);
        assert_eq!(scroll_offset(0, 30, 100, 10, 3), 10);
        assert_eq!(scroll_offset(10, 5, 100, 10, 3), 0);
    }

    #[test]
    fn no_empty_columns_when_list_fits() {
        // 25 элементов умещаются в 3 колонки по 10 — смещение сбрасывается.
        assert_eq!(scroll_offset(20, 24, 25, 10, 3), 0);
        // После уменьшения списка смещение не оставляет пустоту справа.
        assert_eq!(scroll_offset(60, 41, 45, 10, 3), 20);
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        assert_eq!(scroll_offset(5, 0, 0, 10, 3), 0);
        assert_eq!(scroll_offset(0, 50, 10, 0, 0), 9);
    }

    #[test]
    fn enter_and_leave_restore_cursor() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        for name in ["a", "b", "c"] {
            fs::create_dir(root.join(name)).unwrap();
        }
        fs::write(root.join("b").join("inner.txt"), "").unwrap();

        let mut panel = Panel::open(root.clone(), 1, true).unwrap();
        panel.select_name("b");
        assert!(panel.enter(true).unwrap());
        assert_eq!(panel.dir, root.join("b"));
        assert_eq!(panel.current().unwrap().name, "inner.txt");

        // В файл войти нельзя.
        assert!(!panel.enter(true).unwrap());

        assert!(panel.leave(true).unwrap());
        assert_eq!(panel.dir, root);
        assert_eq!(panel.current().unwrap().name, "b");
    }

    #[test]
    fn reload_keeps_cursor_on_same_name() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        fs::write(root.join(".dot"), "").unwrap();
        fs::write(root.join("x"), "").unwrap();

        let mut panel = Panel::open(root.clone(), 1, true).unwrap();
        panel.select_name("x");
        panel.reload(false).unwrap();
        assert_eq!(panel.entries.len(), 1);
        assert_eq!(panel.current().unwrap().name, "x");

        // Курсор на исчезнувшем элементе не выходит за границы списка.
        panel.reload(true).unwrap();
        panel.select_name(".dot");
        panel.reload(false).unwrap();
        assert!(panel.cursor < panel.entries.len());
    }

    #[test]
    fn move_by_clamps_to_bounds() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..5 {
            fs::write(tmp.path().join(format!("f{i}")), "").unwrap();
        }
        let mut panel = Panel::open(tmp.path().to_path_buf(), 1, true).unwrap();
        panel.move_by(-3);
        assert_eq!(panel.cursor, 0);
        panel.move_by(100);
        assert_eq!(panel.cursor, 4);
        panel.home();
        assert_eq!(panel.cursor, 0);
        panel.end();
        assert_eq!(panel.cursor, 4);
    }

    #[test]
    fn marks_select_targets_and_survive_reload() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["a", "b", "c"] {
            fs::write(tmp.path().join(name), "xy").unwrap();
        }
        let mut panel = Panel::open(tmp.path().to_path_buf(), 1, true).unwrap();
        // Без выделения цель — элемент под курсором.
        assert_eq!(panel.targets(), [tmp.path().join("a")]);

        panel.toggle_mark(); // a, курсор на b
        panel.move_by(1);
        panel.toggle_mark(); // c, курсор остаётся на последнем
        assert_eq!(panel.cursor, 2);
        assert_eq!(
            panel.targets(),
            [tmp.path().join("a"), tmp.path().join("c")]
        );
        assert_eq!(panel.marked_size(), 4);

        // Повторное нажатие снимает выделение.
        panel.toggle_mark();
        assert_eq!(panel.marked.len(), 1);

        fs::remove_file(tmp.path().join("a")).unwrap();
        panel.reload(true).unwrap();
        assert!(panel.marked.is_empty());
    }

    #[test]
    fn changing_directory_clears_marks() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        let mut panel = Panel::open(tmp.path().to_path_buf(), 1, true).unwrap();
        panel.marked.insert("sub".into());
        panel.enter(true).unwrap();
        assert!(panel.marked.is_empty());
    }

    #[test]
    fn mounted_archive_behaves_like_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let root = tmp.path().join("tmp-root");
        fs::create_dir_all(home.join("x")).unwrap();
        fs::write(home.join("a.zip"), "").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();

        let mut panel = Panel::open(home.clone(), 1, true).unwrap();
        panel.mount(home.join("a.zip"), root.clone(), true).unwrap();
        assert_eq!(panel.display_dir(), home.join("a.zip"));
        panel.enter(true).unwrap();
        assert_eq!(panel.display_dir(), home.join("a.zip/src"));

        // Выход из корня архива — к самому архиву, временная папка удаляется.
        panel.leave(true).unwrap();
        assert!(root.exists());
        panel.leave(true).unwrap();
        assert_eq!(panel.dir, home);
        assert_eq!(panel.current().unwrap().name, "a.zip");
        assert!(panel.mounts.is_empty());
        assert!(!root.exists());
    }

    #[test]
    fn jumping_out_of_archive_unmounts_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("r");
        fs::create_dir_all(&root).unwrap();
        let mut panel = Panel::open(tmp.path().to_path_buf(), 1, true).unwrap();
        panel
            .mount(tmp.path().join("a.tar"), root.clone(), true)
            .unwrap();
        panel.cd(tmp.path().to_path_buf(), true).unwrap();
        assert!(panel.mounts.is_empty());
        assert!(!root.exists());
    }

    #[test]
    fn dropping_panel_removes_mounts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("r");
        fs::create_dir_all(&root).unwrap();
        let mut panel = Panel::open(tmp.path().to_path_buf(), 1, true).unwrap();
        panel
            .mount(tmp.path().join("a.tar"), root.clone(), true)
            .unwrap();
        drop(panel);
        assert!(!root.exists());
    }
}
