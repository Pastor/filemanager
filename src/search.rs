//! Поиск по поддереву: по содержимому файлов и по именам.
//!
//! Поиск идёт в фоновом потоке и присылает находки пачками, чтобы первые
//! результаты появлялись сразу, а не после обхода всего дерева. Каждое
//! нажатие клавиши в строке запроса запускает новый поиск; прежний
//! останавливается по флагу, который он проверяет на каждом файле.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::listing::natural_cmp;

/// Файлы крупнее не просматриваются: в них редко ищут текст, а читать долго.
const FILE_LIMIT: u64 = 16 * 1024 * 1024;
/// По этому началу файла решается, текст ли он.
const SNIFF: usize = 8 * 1024;
/// Больше находок не собираем: столько всё равно никто не пролистает.
pub const HIT_LIMIT: usize = 10_000;
/// Длина фрагмента строки в результате.
const SNIPPET: usize = 300;
/// Как часто отправлять накопленные находки.
const BATCH_PERIOD: Duration = Duration::from_millis(50);

/// Подготовленный образец поиска.
///
/// Регистр учитывается, только если в запросе есть заглавные буквы: так
/// строчный запрос находит всё, а с заглавной можно искать точно.
pub struct Needle {
    chars: Vec<char>,
    exact: bool,
}

impl Needle {
    pub fn new(query: &str) -> Self {
        let exact = query.chars().any(char::is_uppercase);
        let mut needle = Self {
            chars: Vec::new(),
            exact,
        };
        needle.chars = query.chars().map(|c| needle.fold(c)).collect();
        needle
    }

    fn fold(&self, c: char) -> char {
        if self.exact {
            c
        } else {
            c.to_lowercase().next().unwrap_or(c)
        }
    }

    /// Непересекающиеся вхождения в строке: позиция и длина в символах.
    pub fn find(&self, text: &str) -> Vec<(usize, usize)> {
        let n = self.chars.len();
        if n == 0 {
            return Vec::new();
        }
        let hay: Vec<char> = text.chars().map(|c| self.fold(c)).collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i + n <= hay.len() {
            if hay[i..i + n] == self.chars[..] {
                out.push((i, n));
                i += n;
            } else {
                i += 1;
            }
        }
        out
    }
}

/// Где искать.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// В содержимом файлов.
    Content,
    /// В именах файлов и директорий.
    Name,
}

/// Находка.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: PathBuf,
    /// Номер строки (с нуля) — для поиска по содержимому.
    pub line: Option<usize>,
    /// Найденная строка (обрезанная) или имя файла.
    pub text: String,
    /// Первое вхождение в `text`: позиция и длина в символах.
    pub at: (usize, usize),
    /// Найденное — директория (при поиске по имени).
    pub is_dir: bool,
}

/// Сообщение фонового поиска.
pub enum Msg {
    Hits(Vec<Hit>),
    /// Поиск закончен: сколько файлов просмотрено и упёрся ли он в предел.
    Done {
        files: u64,
        limited: bool,
    },
    /// Сколько файлов просмотрено к этому моменту.
    Progress(u64),
}

/// Выполняющийся поиск.
pub struct Task {
    pub rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
}

impl Task {
    /// Запускает поиск `query` в поддереве `root`.
    pub fn spawn(kind: Kind, root: PathBuf, query: String, show_hidden: bool) -> Self {
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        std::thread::spawn(move || {
            let mut walker = Walker {
                kind,
                needle: Needle::new(&query),
                show_hidden,
                cancel: flag,
                tx,
                batch: Vec::new(),
                sent: 0,
                files: 0,
                last: Instant::now(),
            };
            walker.walk(&root);
            walker.flush();
            let limited = walker.sent >= HIT_LIMIT;
            let _ = walker.tx.send(Msg::Done {
                files: walker.files,
                limited,
            });
        });
        Self { rx, cancel }
    }
}

impl Drop for Task {
    /// Брошенный поиск не должен молотить диск в фоне.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct Walker {
    kind: Kind,
    needle: Needle,
    show_hidden: bool,
    cancel: Arc<AtomicBool>,
    tx: Sender<Msg>,
    batch: Vec<Hit>,
    /// Отправлено находок.
    sent: usize,
    files: u64,
    last: Instant,
}

impl Walker {
    fn stopped(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || self.sent + self.batch.len() >= HIT_LIMIT
    }

    /// Обходит директорию: сначала её файлы, затем поддиректории — так
    /// результаты идут в том же порядке, что и в панели.
    fn walk(&mut self, dir: &Path) {
        let Ok(children) = fs::read_dir(dir) else {
            return;
        };
        let mut items: Vec<(String, PathBuf, bool)> = children
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if !self.show_hidden && name.starts_with('.') {
                    return None;
                }
                // Ссылки на директории не разыменовываем: они легко
                // зацикливают обход.
                let is_dir = e.file_type().ok()?.is_dir();
                Some((name, e.path(), is_dir))
            })
            .collect();
        items.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| natural_cmp(&a.0, &b.0)));

        for (name, path, is_dir) in items {
            if self.stopped() {
                return;
            }
            match self.kind {
                Kind::Name => {
                    if let Some(&at) = self.needle.find(&name).first() {
                        self.push(Hit {
                            path: path.clone(),
                            line: None,
                            text: name,
                            at,
                            is_dir,
                        });
                    }
                }
                Kind::Content if !is_dir => self.grep(&path),
                Kind::Content => {}
            }
            if is_dir {
                self.walk(&path);
            } else {
                self.files += 1;
            }
        }
    }

    /// Ищет образец в строках текстового файла.
    fn grep(&mut self, path: &Path) {
        let Ok(meta) = fs::metadata(path) else {
            return;
        };
        if !meta.is_file() || meta.len() > FILE_LIMIT {
            return;
        }
        let mut bytes = Vec::new();
        if File::open(path)
            .and_then(|mut f| f.read_to_end(&mut bytes))
            .is_err()
        {
            return;
        }
        if bytes[..bytes.len().min(SNIFF)].contains(&0) {
            return;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (line, raw) in text.lines().enumerate() {
            if let Some(&at) = self.needle.find(raw).first() {
                let (text, at) = snippet(raw, at);
                self.push(Hit {
                    path: path.to_path_buf(),
                    line: Some(line),
                    text,
                    at,
                    is_dir: false,
                });
                if self.stopped() {
                    return;
                }
            }
        }
    }

    fn push(&mut self, hit: Hit) {
        self.batch.push(hit);
        if self.last.elapsed() >= BATCH_PERIOD {
            self.flush();
        }
    }

    fn flush(&mut self) {
        self.last = Instant::now();
        if self.batch.is_empty() {
            let _ = self.tx.send(Msg::Progress(self.files));
            return;
        }
        self.sent += self.batch.len();
        let batch = std::mem::take(&mut self.batch);
        if self.tx.send(Msg::Hits(batch)).is_err() {
            // Интерфейсу результаты больше не нужны.
            self.cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// Фрагмент длинной строки вокруг найденного: без ведущих пробелов,
/// табуляция заменена пробелом, длина ограничена. Позиция пересчитывается.
fn snippet(line: &str, (start, len): (usize, usize)) -> (String, (usize, usize)) {
    let chars: Vec<char> = line
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .collect();
    let lead = chars
        .iter()
        .take_while(|c| c.is_whitespace())
        .count()
        .min(start);
    // Если совпадение далеко от начала, показываем немного контекста перед ним.
    let from = if start - lead > SNIPPET / 2 {
        start - 20
    } else {
        lead
    };
    let to = chars.len().min(from + SNIPPET);
    let text: String = chars[from..to].iter().collect();
    (text, (start - from, len.min(to.saturating_sub(start))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(kind: Kind, root: &Path, query: &str, hidden: bool) -> (Vec<Hit>, u64) {
        let task = Task::spawn(kind, root.to_path_buf(), query.to_string(), hidden);
        let mut hits = Vec::new();
        loop {
            match task.rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Msg::Hits(batch) => hits.extend(batch),
                Msg::Progress(_) => {}
                Msg::Done { files, .. } => return (hits, files),
            }
        }
    }

    fn tree() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("src/deep")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("README.md"), "Hello world\nnothing\n").unwrap();
        fs::write(
            root.join("src/main.rs"),
            "fn main() {\n\tprintln!(\"hello\");\n}\n",
        )
        .unwrap();
        fs::write(root.join("src/deep/hello_test.rs"), "// no match here\n").unwrap();
        fs::write(root.join(".git/config"), "hello = 1\n").unwrap();
        fs::write(root.join("blob.bin"), [b'h', b'e', 0, b'l', b'l', b'o']).unwrap();
        tmp
    }

    #[test]
    fn needle_is_smart_case() {
        assert_eq!(Needle::new("ab").find("xABab"), [(1, 2), (3, 2)]);
        assert_eq!(Needle::new("Ab").find("xABab Ab"), [(6, 2)]);
        assert!(Needle::new("").find("abc").is_empty());
    }

    #[test]
    fn content_search_finds_lines_in_order() {
        let tmp = tree();
        let (hits, files) = run(Kind::Content, tmp.path(), "hello", true);
        let found: Vec<(String, Option<usize>)> = hits
            .iter()
            .map(|h| {
                let rel = h.path.strip_prefix(tmp.path()).unwrap();
                (rel.display().to_string(), h.line)
            })
            .collect();
        // Сначала файлы корня, затем поддиректории; двоичный файл пропущен.
        assert_eq!(
            found,
            [
                ("README.md".to_string(), Some(0)),
                (".git/config".to_string(), Some(0)),
                ("src/main.rs".to_string(), Some(1)),
            ]
        );
        assert_eq!(files, 5);
        // Ведущая табуляция срезана, позиция пересчитана.
        let main = &hits[2];
        assert_eq!(main.text, "println!(\"hello\");");
        assert_eq!(main.at, (10, 5));
    }

    #[test]
    fn hidden_entries_follow_the_setting() {
        let tmp = tree();
        let (hits, _) = run(Kind::Content, tmp.path(), "hello", false);
        assert!(
            hits.iter()
                .all(|h| !h.path.to_string_lossy().contains(".git"))
        );
    }

    #[test]
    fn name_search_matches_files_and_dirs() {
        let tmp = tree();
        let (hits, _) = run(Kind::Name, tmp.path(), "e", true);
        let names: Vec<&str> = hits.iter().map(|h| h.text.as_str()).collect();
        assert!(names.contains(&"README.md"));
        assert!(names.contains(&"deep"));
        assert!(names.contains(&"hello_test.rs"));
        let deep = hits.iter().find(|h| h.text == "deep").unwrap();
        assert!(deep.is_dir);
        assert_eq!(deep.at, (1, 1));

        let (hits, _) = run(Kind::Name, tmp.path(), "HELLO", true);
        assert!(hits.is_empty(), "заглавные — точный регистр");
    }

    #[test]
    fn snippet_keeps_context_near_match() {
        let (text, at) = snippet("    abc", (4, 3));
        assert_eq!((text.as_str(), at), ("abc", (0, 3)));
        let long = format!("{}needle", "x".repeat(400));
        let (text, at) = snippet(&long, (400, 6));
        assert_eq!(&text[at.0..at.0 + 6], "needle");
        assert_eq!(at.0, 20);
    }

    #[test]
    fn dropping_task_cancels_it() {
        let tmp = tree();
        let task = Task::spawn(Kind::Content, tmp.path().to_path_buf(), "x".into(), true);
        let cancel = Arc::clone(&task.cancel);
        drop(task);
        assert!(cancel.load(Ordering::Relaxed));
    }
}
