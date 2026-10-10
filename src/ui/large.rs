//! Режим «большие объекты»: дерево размеров, как во `flob`.
//!
//! Директория сканируется целиком один раз в фоне; дальше по дереву можно
//! ходить без повторных обращений к диску. Содержимое каждого уровня
//! показывается по убыванию размера.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{Receiver, bounded};

use crate::listing::natural_cmp;
use crate::panel::scroll_offset;
use crate::scan::{Node, Progress, ScanOptions, scan};

/// Состояние режима.
pub struct Large {
    /// Сканируемая директория.
    pub root: PathBuf,
    pub progress: Arc<Progress>,
    /// Результат сканирования; `None`, пока оно идёт.
    pub tree: Option<Node>,
    rx: Receiver<Node>,
    pub started: Instant,
    /// Индексы детей по пути от корня к текущему уровню.
    pub stack: Vec<usize>,
    /// Позиция курсора на каждом пройденном уровне — для возврата назад.
    cursors: Vec<usize>,
    /// Индексы детей текущего уровня по убыванию размера.
    pub order: Vec<usize>,
    pub cursor: usize,
    pub offset: usize,
    pub rows: usize,
}

impl Large {
    /// Запускает сканирование `root` в фоне.
    pub fn start(root: PathBuf) -> Self {
        let progress = Arc::new(Progress::default());
        let (tx, rx) = bounded(1);
        let (path, shared) = (root.clone(), Arc::clone(&progress));
        std::thread::spawn(move || {
            let node = scan(&path, ScanOptions { cross_fs: false }, shared);
            let _ = tx.send(node);
        });
        Self {
            root,
            progress,
            tree: None,
            rx,
            started: Instant::now(),
            stack: Vec::new(),
            cursors: Vec::new(),
            order: Vec::new(),
            cursor: 0,
            offset: 0,
            rows: 1,
        }
    }

    /// Канал результата, пока сканирование идёт.
    pub fn rx(&self) -> Option<Receiver<Node>> {
        self.tree.is_none().then(|| self.rx.clone())
    }

    pub fn scanned(&mut self, tree: Node) {
        self.tree = Some(tree);
        self.rebuild();
    }

    /// Текущий уровень дерева.
    pub fn current(&self) -> Option<&Node> {
        let mut node = self.tree.as_ref()?;
        for &i in &self.stack {
            node = &node.children[i];
        }
        Some(node)
    }

    /// Путь текущего уровня.
    pub fn current_path(&self) -> PathBuf {
        let mut path = self.root.clone();
        let Some(mut node) = self.tree.as_ref() else {
            return path;
        };
        for &i in &self.stack {
            node = &node.children[i];
            path.push(&*node.name);
        }
        path
    }

    /// Ребёнок под курсором.
    pub fn selected(&self) -> Option<&Node> {
        let i = *self.order.get(self.cursor)?;
        self.current().map(|node| &node.children[i])
    }

    /// Пересобирает порядок текущего уровня: по размеру, при равенстве — по имени.
    fn rebuild(&mut self) {
        let Some(node) = self.current() else {
            self.order.clear();
            return;
        };
        let mut order: Vec<usize> = (0..node.children.len()).collect();
        order.sort_by(|&a, &b| {
            let (x, y) = (&node.children[a], &node.children[b]);
            y.size
                .cmp(&x.size)
                .then_with(|| natural_cmp(&x.display_name(), &y.display_name()))
        });
        self.order = order;
        self.cursor = self.cursor.min(self.order.len().saturating_sub(1));
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.order.is_empty() {
            return;
        }
        let last = self.order.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.order.len().saturating_sub(1);
    }

    /// Входит в директорию под курсором.
    pub fn enter(&mut self) -> bool {
        let Some(&i) = self.order.get(self.cursor) else {
            return false;
        };
        if !self.selected().is_some_and(Node::is_dir) {
            return false;
        }
        self.stack.push(i);
        self.cursors.push(self.cursor);
        self.cursor = 0;
        self.offset = 0;
        self.rebuild();
        true
    }

    /// Поднимается на уровень выше, курсор — на директорию, из которой вышли.
    pub fn leave(&mut self) -> bool {
        if self.stack.pop().is_none() {
            return false;
        }
        self.cursor = self.cursors.pop().unwrap_or(0);
        self.offset = 0;
        self.rebuild();
        true
    }

    /// Подгоняет прокрутку; вызывается при отрисовке.
    pub fn fit(&mut self, rows: usize) {
        self.rows = rows.max(1);
        self.offset = scroll_offset(self.offset, self.cursor, self.order.len(), self.rows, 1);
    }
}

impl Drop for Large {
    /// Брошенное сканирование не должно продолжать ходить по диску.
    fn drop(&mut self) {
        self.progress.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn scanned(root: PathBuf) -> Large {
        let mut large = Large::start(root);
        let tree = large
            .rx()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        large.scanned(tree);
        large
    }

    #[test]
    fn levels_are_sorted_by_size_and_navigable() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("big/inner")).unwrap();
        fs::write(root.join("big/inner/blob"), vec![0u8; 5000]).unwrap();
        fs::write(root.join("big/small"), vec![0u8; 10]).unwrap();
        fs::write(root.join("mid"), vec![0u8; 1000]).unwrap();
        fs::write(root.join("tiny"), vec![0u8; 1]).unwrap();

        let mut large = scanned(root.to_path_buf());
        let names: Vec<String> = large
            .order
            .iter()
            .map(|&i| {
                large.current().unwrap().children[i]
                    .display_name()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, ["big", "mid", "tiny"]);
        assert_eq!(large.current().unwrap().size, 6011);

        assert!(large.enter());
        assert_eq!(large.current_path(), root.join("big"));
        assert_eq!(large.selected().unwrap().display_name(), "inner");
        // В файл не войти.
        large.move_by(1);
        assert!(!large.enter());

        assert!(large.leave());
        assert_eq!(large.current_path(), root);
        assert_eq!(large.selected().unwrap().display_name(), "big");
        assert!(!large.leave());
    }
}
