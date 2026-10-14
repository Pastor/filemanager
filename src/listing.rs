//! Чтение содержимого директории и порядок его отображения.
//!
//! Панели нужны не только имена, но и то, как их раскрасить: директория,
//! исполняемый файл, ссылка (живая или битая). Всё это определяется один раз
//! при чтении, чтобы отрисовка не ходила в файловую систему на каждом кадре.

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Вид элемента директории.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    /// Символическая ссылка на директорию.
    LinkDir,
    /// Символическая ссылка на файл или иной не-каталог.
    LinkFile,
    /// Ссылка, цель которой не существует.
    LinkBroken,
    /// Сокет, устройство, FIFO.
    Special,
}

impl Kind {
    /// В элемент можно войти как в директорию.
    pub fn is_dir(self) -> bool {
        matches!(self, Kind::Dir | Kind::LinkDir)
    }

    /// Элемент — символическая ссылка.
    pub fn is_link(self) -> bool {
        matches!(self, Kind::LinkDir | Kind::LinkFile | Kind::LinkBroken)
    }
}

/// Элемент директории.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Имя для показа; невалидный UTF-8 заменяется символом-заменителем.
    pub name: String,
    /// Полный путь.
    pub path: PathBuf,
    pub kind: Kind,
    /// Исполняемый обычный файл (или ссылка на такой).
    pub exec: bool,
    /// Размер в байтах; для директорий — размер самой записи, не содержимого.
    pub size: u64,
    /// Время последнего изменения.
    pub modified: Option<SystemTime>,
    /// Куда указывает ссылка.
    pub target: Option<PathBuf>,
}

impl Entry {
    /// Скрытый файл в смысле Unix: имя начинается с точки.
    pub fn is_hidden(&self) -> bool {
        self.name.starts_with('.')
    }

    /// Читает сведения об одном элементе.
    pub fn read(path: PathBuf) -> io::Result<Self> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let lmeta = fs::symlink_metadata(&path)?;

        let (kind, meta, target) = if lmeta.file_type().is_symlink() {
            let target = fs::read_link(&path).ok();
            match fs::metadata(&path) {
                Ok(meta) if meta.is_dir() => (Kind::LinkDir, meta, target),
                Ok(meta) => (Kind::LinkFile, meta, target),
                Err(_) => (Kind::LinkBroken, lmeta, target),
            }
        } else if lmeta.is_dir() {
            (Kind::Dir, lmeta, None)
        } else if lmeta.is_file() {
            (Kind::File, lmeta, None)
        } else {
            (Kind::Special, lmeta, None)
        };

        let exec = matches!(kind, Kind::File | Kind::LinkFile) && is_executable(&meta);
        Ok(Self {
            name,
            path,
            kind,
            exec,
            size: meta.len(),
            modified: meta.modified().ok(),
            target,
        })
    }
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    false
}

/// Читает директорию и сортирует её содержимое для показа.
///
/// Элементы, которые не удалось прочитать (исчезли между `read_dir` и
/// `metadata`), молча пропускаются: в живой файловой системе это норма.
pub fn read_dir(dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let mut entries: Vec<Entry> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| Entry::read(e.path()).ok())
        .filter(|e| show_hidden || !e.is_hidden())
        .collect();
    entries.sort_by(compare);
    Ok(entries)
}

/// Порядок показа: директории (и ссылки на них) первыми, затем остальное;
/// внутри групп — естественный порядок имён без учёта регистра.
pub fn compare(a: &Entry, b: &Entry) -> Ordering {
    b.kind
        .is_dir()
        .cmp(&a.kind.is_dir())
        .then_with(|| natural_cmp(&a.name, &b.name))
}

/// Естественное сравнение строк: числа внутри имени сравниваются как числа
/// (`file2` < `file10`), буквы — без учёта регистра. При полном равенстве
/// решает обычное сравнение, чтобы порядок был однозначным.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ia = a.chars().peekable();
    let mut ib = b.chars().peekable();
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let na = take_number(&mut ia);
                let nb = take_number(&mut ib);
                // Сначала по значению (длина без ведущих нулей, затем цифры),
                // при равенстве — более короткая запись раньше: `1` < `01`.
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta
                    .len()
                    .cmp(&tb.len())
                    .then_with(|| ta.cmp(tb))
                    .then_with(|| na.len().cmp(&nb.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(ca), Some(cb)) => {
                let ord = ca.to_lowercase().cmp(cb.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                ia.next();
                ib.next();
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut out = String::new();
    while let Some(&c) = it.peek() {
        if !c.is_ascii_digit() {
            break;
        }
        out.push(c);
        it.next();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_compares_numbers_by_value() {
        let mut names = vec!["file10", "file2", "File1", "file02", "a", "B"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["a", "B", "File1", "file2", "file02", "file10"]);
    }

    #[test]
    fn natural_order_handles_prefixes() {
        assert_eq!(natural_cmp("abc", "abcd"), Ordering::Less);
        assert_eq!(natural_cmp("x9", "x9a"), Ordering::Less);
        assert_eq!(natural_cmp("Ёж", "ёж"), "Ёж".cmp("ёж"));
    }

    #[test]
    fn read_dir_classifies_and_orders() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("zdir")).unwrap();
        fs::write(root.join("a.txt"), "hello").unwrap();
        fs::write(root.join(".hidden"), "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(root.join("run.sh"), "#!/bin/sh").unwrap();
            fs::set_permissions(root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
            std::os::unix::fs::symlink(root.join("zdir"), root.join("link")).unwrap();
            std::os::unix::fs::symlink(root.join("nowhere"), root.join("broken")).unwrap();
        }

        let all = read_dir(root, true).unwrap();
        let names: Vec<&str> = all.iter().map(|e| e.name.as_str()).collect();
        #[cfg(unix)]
        assert_eq!(
            names,
            ["link", "zdir", ".hidden", "a.txt", "broken", "run.sh"]
        );
        #[cfg(not(unix))]
        assert_eq!(names, ["zdir", ".hidden", "a.txt"]);

        let find = |n: &str| all.iter().find(|e| e.name == n).unwrap();
        assert_eq!(find("zdir").kind, Kind::Dir);
        assert_eq!(find("a.txt").kind, Kind::File);
        assert_eq!(find("a.txt").size, 5);
        assert!(!find("a.txt").exec);
        #[cfg(unix)]
        {
            assert!(find("run.sh").exec);
            assert_eq!(find("link").kind, Kind::LinkDir);
            assert_eq!(find("broken").kind, Kind::LinkBroken);
        }

        let visible = read_dir(root, false).unwrap();
        assert!(visible.iter().all(|e| !e.is_hidden()));
    }
}
