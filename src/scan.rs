//! Параллельное сканирование файловой системы и модель дерева.
//!
//! Взято из `flob` без изменений логики: режим «большие объекты» — его
//! облегчённая версия внутри файлового менеджера.
//!
//! Размеры директорий агрегируются «снизу вверх» прямо на возврате из рекурсии,
//! поэтому обход дерева выполняется ровно один раз.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::Mutex;
use rayon::prelude::*;

/// Тип объекта файловой системы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Директория.
    Dir,
    /// Обычный файл.
    File,
    /// Символическая ссылка (не разыменовывается).
    Symlink,
    /// Прочее: сокет, FIFO, устройство.
    Other,
}

/// Узел дерева. Полный путь не хранится — он восстанавливается из стека навигации,
/// что экономит десятки байт и одну аллокацию на каждый объект.
#[derive(Debug)]
pub struct Node {
    /// Имя объекта без пути.
    pub name: Box<OsStr>,
    /// Размер: для файла — `len`, для директории — сумма поддерева.
    pub size: u64,
    /// Количество объектов в поддереве (сам узел не считается).
    pub items: u64,
    /// Тип объекта.
    pub kind: Kind,
    /// Директорию не удалось прочитать (нет прав) либо она на другой ФС.
    pub denied: bool,
    /// Дети; непусто только у директорий.
    pub children: Vec<Node>,
}

impl Node {
    /// Имя в виде строки для отображения.
    pub fn display_name(&self) -> std::borrow::Cow<'_, str> {
        self.name.to_string_lossy()
    }

    /// Является ли узел директорией, в которую можно провалиться.
    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }
}

/// Счётчики хода сканирования; читаются UI по таймеру.
#[derive(Debug, Default)]
pub struct Progress {
    /// Просмотрено файлов.
    pub files: AtomicU64,
    /// Просмотрено директорий.
    pub dirs: AtomicU64,
    /// Суммарный учтённый объём.
    pub bytes: AtomicU64,
    /// Запрос на отмену обхода.
    pub cancel: AtomicBool,
}

impl Progress {
    /// Просит сканер завершиться досрочно.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Снимок счётчиков: (файлы, директории, байты).
    pub fn snapshot(&self) -> (u64, u64, u64) {
        (
            self.files.load(Ordering::Relaxed),
            self.dirs.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
        )
    }
}

/// Параметры обхода.
#[derive(Debug, Clone, Copy)]
pub struct ScanOptions {
    /// Переходить на другие файловые системы.
    pub cross_fs: bool,
}

/// Разделяемый контекст одного сканирования.
struct Ctx {
    progress: Arc<Progress>,
    /// Уже учтённые жёсткие ссылки: (dev, inode).
    seen_links: Mutex<HashSet<(u64, u64)>>,
    /// Устройство корня — для отсечения чужих ФС.
    root_dev: u64,
    opts: ScanOptions,
}

impl Ctx {
    /// Проверяет, не отменено ли сканирование.
    fn cancelled(&self) -> bool {
        self.progress.cancel.load(Ordering::Relaxed)
    }

    /// Регистрирует жёсткую ссылку. `false` — этот inode уже был учтён.
    fn count_hardlink(&self, dev: u64, ino: u64) -> bool {
        self.seen_links.lock().insert((dev, ino))
    }
}

/// Сканирует `root` целиком и возвращает корневой узел.
///
/// Функция блокирующая — вызывать в отдельном потоке, следя за `progress`.
pub fn scan(root: &Path, opts: ScanOptions, progress: Arc<Progress>) -> Node {
    let root_dev = device_of(root).unwrap_or(0);
    let ctx = Ctx {
        progress,
        seen_links: Mutex::new(HashSet::new()),
        root_dev,
        opts,
    };

    let (children, size, items, denied) = scan_children(root, &ctx);
    Node {
        name: root_name(root),
        size,
        items,
        kind: Kind::Dir,
        denied,
        children,
    }
}

/// Обходит содержимое директории. Возвращает `(дети, размер, объекты, отказ_в_доступе)`.
fn scan_children(dir: &Path, ctx: &Ctx) -> (Vec<Node>, u64, u64, bool) {
    if ctx.cancelled() {
        return (Vec::new(), 0, 0, false);
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        // Нет прав или директория исчезла — помечаем, но обход не прерываем.
        Err(_) => return (Vec::new(), 0, 0, true),
    };

    let mut leaves: Vec<Node> = Vec::new();
    // Для каждой поддиректории: путь, имя и её устройство (нужно, только если
    // границы ФС пересекать нельзя).
    let mut subdirs: Vec<(PathBuf, OsString, Option<u64>)> = Vec::new();
    // Счётчики набираются локально и публикуются одним fetch_add на директорию.
    let mut local_files = 0u64;
    let mut local_bytes = 0u64;
    let mut size = 0u64;
    let mut items = 0u64;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Ok(ft) = entry.file_type() else { continue };

        if ft.is_symlink() {
            // Не разыменовываем: иначе легко зациклиться и удвоить учёт.
            leaves.push(Node {
                name: name.into_boxed_os_str(),
                size: 0,
                items: 0,
                kind: Kind::Symlink,
                denied: false,
                children: Vec::new(),
            });
            items += 1;
            local_files += 1;
            continue;
        }

        if ft.is_dir() {
            // `DirEntry::metadata` на Unix идёт через fstatat относительно уже
            // открытой директории — дешевле, чем stat по полному пути.
            let dev = if ctx.opts.cross_fs {
                None
            } else {
                entry.metadata().ok().map(|m| device_of_meta(&m))
            };
            subdirs.push((entry.path(), name, dev));
            continue;
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let kind = if ft.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        // Жёсткие ссылки учитываем один раз: мьютекс берём только при nlink > 1,
        // поэтому на типичных деревьях конкуренции за него нет.
        let counted = if nlink_of(&meta) > 1 {
            let (dev, ino) = dev_ino_of(&meta);
            ctx.count_hardlink(dev, ino)
        } else {
            true
        };
        let len = if counted { meta.len() } else { 0 };

        leaves.push(Node {
            name: name.into_boxed_os_str(),
            size: len,
            items: 0,
            kind,
            denied: false,
            children: Vec::new(),
        });
        size += len;
        items += 1;
        local_files += 1;
        local_bytes += len;
    }

    ctx.progress.files.fetch_add(local_files, Ordering::Relaxed);
    ctx.progress.bytes.fetch_add(local_bytes, Ordering::Relaxed);
    ctx.progress
        .dirs
        .fetch_add(subdirs.len() as u64, Ordering::Relaxed);

    // Поддиректории обходим параллельно: rayon сам держит пул по числу ядер
    // и не создаёт поток на каждый уровень вложенности.
    let dir_nodes: Vec<Node> = subdirs
        .into_par_iter()
        .map(|(path, name, dev)| {
            // Не уходим на чужую файловую систему, если это не разрешено явно:
            // иначе на macOS обход утекает в /System/Volumes.
            if !ctx.opts.cross_fs
                && let Some(dev) = dev
                && dev != ctx.root_dev
            {
                return Node {
                    name: name.into_boxed_os_str(),
                    size: 0,
                    items: 0,
                    kind: Kind::Dir,
                    denied: true,
                    children: Vec::new(),
                };
            }

            let (children, size, items, denied) = scan_children(&path, ctx);
            Node {
                name: name.into_boxed_os_str(),
                size,
                items,
                kind: Kind::Dir,
                denied,
                children,
            }
        })
        .collect();

    for node in &dir_nodes {
        size += node.size;
        items += node.items + 1;
    }

    leaves.extend(dir_nodes);
    (leaves, size, items, false)
}

/// Имя корня для шапки: последний компонент пути либо путь целиком.
fn root_name(path: &Path) -> Box<OsStr> {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_os_string()
        .into_boxed_os_str()
}

#[cfg(unix)]
fn device_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.dev())
}

#[cfg(not(unix))]
fn device_of(_path: &Path) -> Option<u64> {
    None
}

#[cfg(unix)]
fn device_of_meta(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.dev()
}

#[cfg(not(unix))]
fn device_of_meta(_meta: &std::fs::Metadata) -> u64 {
    0
}

#[cfg(unix)]
fn nlink_of(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.nlink()
}

#[cfg(not(unix))]
fn nlink_of(_meta: &std::fs::Metadata) -> u64 {
    1
}

#[cfg(unix)]
fn dev_ino_of(meta: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn dev_ino_of(_meta: &std::fs::Metadata) -> (u64, u64) {
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Ищет ребёнка по имени.
    fn child<'a>(node: &'a Node, name: &str) -> &'a Node {
        node.children
            .iter()
            .find(|c| c.display_name() == name)
            .unwrap_or_else(|| panic!("нет ребёнка «{name}»"))
    }

    fn scan_default(root: &Path) -> Node {
        scan(
            root,
            ScanOptions { cross_fs: false },
            Arc::new(Progress::default()),
        )
    }

    #[test]
    fn aggregates_nested_sizes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("a.bin"), vec![0u8; 1000]).unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("b.bin"), vec![0u8; 2000]).unwrap();
        fs::create_dir(root.join("sub").join("deep")).unwrap();
        fs::write(root.join("sub").join("deep").join("c.bin"), vec![0u8; 500]).unwrap();

        let tree = scan_default(root);
        assert_eq!(tree.size, 3500);
        // 3 файла + 2 директории.
        assert_eq!(tree.items, 5);
        assert_eq!(child(&tree, "sub").size, 2500);
        assert_eq!(child(child(&tree, "sub"), "deep").size, 500);
    }

    #[test]
    fn counts_hidden_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".hidden"), vec![0u8; 777]).unwrap();

        let tree = scan_default(tmp.path());
        assert_eq!(tree.size, 777);
        assert_eq!(child(&tree, ".hidden").size, 777);
    }

    #[test]
    #[cfg(unix)]
    fn hardlink_counted_once() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("orig.bin");
        fs::write(&original, vec![0u8; 4096]).unwrap();
        fs::hard_link(&original, tmp.path().join("link.bin")).unwrap();

        let tree = scan_default(tmp.path());
        assert_eq!(tree.size, 4096, "жёсткая ссылка не должна удваивать размер");
        assert_eq!(tree.items, 2, "но оба объекта видны в списке");
    }

    #[test]
    #[cfg(unix)]
    fn symlink_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("real")).unwrap();
        fs::write(tmp.path().join("real").join("f.bin"), vec![0u8; 100]).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("link")).unwrap();

        let tree = scan_default(tmp.path());
        assert_eq!(
            tree.size, 100,
            "содержимое цели не должно учитываться дважды"
        );
        assert_eq!(child(&tree, "link").kind, Kind::Symlink);
    }

    #[test]
    #[cfg(unix)]
    fn unreadable_dir_is_marked_not_fatal() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let locked = tmp.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("x.bin"), vec![0u8; 10]).unwrap();
        fs::write(tmp.path().join("ok.bin"), vec![0u8; 20]).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        let tree = scan_default(tmp.path());
        // Права возвращаем сразу, иначе tempdir не сможет убраться.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(child(&tree, "locked").denied);
        assert_eq!(child(&tree, "ok.bin").size, 20, "обход продолжился");
    }

    #[test]
    fn progress_is_updated() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a"), vec![0u8; 10]).unwrap();
        fs::create_dir(tmp.path().join("d")).unwrap();
        fs::write(tmp.path().join("d").join("b"), vec![0u8; 20]).unwrap();

        let progress = Arc::new(Progress::default());
        scan(
            tmp.path(),
            ScanOptions { cross_fs: false },
            progress.clone(),
        );

        let (files, dirs, bytes) = progress.snapshot();
        assert_eq!(files, 2);
        assert_eq!(dirs, 1);
        assert_eq!(bytes, 30);
    }

    #[test]
    fn cancel_stops_walk() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a"), vec![0u8; 10]).unwrap();

        let progress = Arc::new(Progress::default());
        progress.cancel();
        let tree = scan(tmp.path(), ScanOptions { cross_fs: false }, progress);
        assert!(tree.children.is_empty());
    }
}
