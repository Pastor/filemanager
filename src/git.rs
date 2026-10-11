//! Незакоммиченные изменения в git: какие элементы директории подсветить.
//!
//! Подсвечивается элемент, за которым числятся изменения, которых нет в
//! последнем коммите: изменённые, добавленные в индекс, удалённые внутри
//! директории и неотслеживаемые файлы. Внутри рабочего дерева это файлы и
//! поддиректории с такими изменениями; снаружи — директории, которые сами
//! являются репозиториями с изменениями.
//!
//! Статус считается вызовом `git`: своя реализация индекса и `.gitignore`
//! была бы большой и всё равно отставала бы от настоящего git. Вызов может
//! быть долгим на большом репозитории, поэтому панель запрашивает его в
//! фоне ([`spawn`]), а не при чтении директории.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crossbeam_channel::Sender;
use rayon::prelude::*;

/// Ответ фонового подсчёта: для какой панели и какого чтения директории.
pub struct Reply<K> {
    pub key: K,
    pub generation: u64,
    pub dirty: HashSet<String>,
}

/// Считает статус в фоне и присылает ответ в `tx`. `dirs` — имена
/// поддиректорий, которые стоит проверить как отдельные репозитории.
pub fn spawn<K: Send + 'static>(
    key: K,
    generation: u64,
    dir: PathBuf,
    dirs: Vec<String>,
    tx: Sender<Reply<K>>,
) {
    rayon::spawn(move || {
        let dirty = dirty_names(&dir, &dirs);
        // Панель могла уйти дальше, а приложение — закрыться: ответ не нужен.
        let _ = tx.send(Reply {
            key,
            generation,
            dirty,
        });
    });
}

/// Имена элементов `dir` с незакоммиченными изменениями.
pub fn dirty_names(dir: &Path, dirs: &[String]) -> HashSet<String> {
    let mut dirty = match git(dir, &["rev-parse", "--show-prefix"]) {
        Some(prefix) => worktree_names(dir, prefix.trim_end_matches('\n')),
        None => HashSet::new(),
    };
    // Вложенный репозиторий внутри рабочего дерева родитель уже показал как
    // неотслеживаемый — второй раз его не спрашиваем.
    let nested: Vec<&String> = dirs
        .iter()
        .filter(|name| !dirty.contains(*name) && dir.join(name).join(".git").exists())
        .collect();
    let found: Vec<String> = nested
        .into_par_iter()
        .filter(|name| has_changes(&dir.join(name)))
        .cloned()
        .collect();
    dirty.extend(found);
    dirty
}

/// Изменения рабочего дерева, сведённые к элементам `dir`. `prefix` — путь
/// `dir` от корня репозитория с `/` в конце (пустой в самом корне).
fn worktree_names(dir: &Path, prefix: &str) -> HashSet<String> {
    let Some(out) = git(
        dir,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=normal",
            "--",
            ".",
        ],
    ) else {
        return HashSet::new();
    };
    parse_status(&out, prefix)
}

/// Разбирает `git status --porcelain=v1 -z`: пути в нём идут от корня
/// репозитория, от каждого берётся первый компонент после `prefix`.
fn parse_status(out: &str, prefix: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut records = out.split('\0');
    while let Some(record) = records.next() {
        // Запись: два символа статуса, пробел, путь.
        let (Some(code), Some(path)) = (record.get(..2), record.get(3..)) else {
            continue;
        };
        // У переименования и копии следом идёт старый путь; его в директории
        // уже нет, подсвечивать нечего.
        if code.contains(['R', 'C']) {
            records.next();
        }
        let Some(rel) = path.strip_prefix(prefix) else {
            continue;
        };
        if let Some(first) = rel.split('/').next().filter(|s| !s.is_empty()) {
            names.insert(first.to_string());
        }
    }
    names
}

/// В репозитории `dir` есть незакоммиченные изменения.
fn has_changes(dir: &Path) -> bool {
    git(
        dir,
        &["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
    )
    .is_some_and(|out| !out.is_empty())
}

/// Запускает git в `dir`; `None` — git нет, это не репозиторий или ошибка.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        // Статус без этого обновляет индекс и берёт index.lock — это может
        // помешать git, запущенному пользователем в то же время.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn status_maps_paths_to_first_component() {
        let out = " M src/main.rs\0?? new.txt\0A  docs/a/b.md\0R  src/x.rs\0old.rs\0";
        let root = parse_status(out, "");
        let expected: HashSet<String> = ["src", "new.txt", "docs"].map(String::from).into();
        assert_eq!(root, expected);

        // Изнутри `src/` видны только её элементы.
        let sub = parse_status(out, "src/");
        let expected: HashSet<String> = ["main.rs", "x.rs"].map(String::from).into();
        assert_eq!(sub, expected);
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn finds_changes_inside_and_outside_worktree() {
        if git(Path::new("."), &["--version"]).is_none() {
            return; // git не установлен
        }
        let tmp = tempfile::tempdir().unwrap();
        let outer = tmp.path();
        let clean = outer.join("clean");
        let messy = outer.join("messy");
        for repo in [&clean, &messy] {
            fs::create_dir_all(repo.join("sub")).unwrap();
            fs::write(repo.join("sub/a.txt"), "a").unwrap();
            fs::write(repo.join("b.txt"), "b").unwrap();
            git_ok(repo, &["init", "-q"]);
            git_ok(repo, &["add", "."]);
            git_ok(repo, &["commit", "-q", "-m", "init"]);
        }
        fs::create_dir(outer.join("plain")).unwrap();
        fs::write(messy.join("sub/a.txt"), "changed").unwrap();
        fs::write(messy.join("untracked"), "").unwrap();

        // Снаружи — только репозиторий с изменениями.
        let dirs = ["clean", "messy", "plain"].map(String::from);
        let expected: HashSet<String> = ["messy".to_string()].into();
        assert_eq!(dirty_names(outer, &dirs), expected);

        // Внутри — изменённая поддиректория и новый файл.
        let expected: HashSet<String> = ["sub", "untracked"].map(String::from).into();
        assert_eq!(dirty_names(&messy, &["sub".into()]), expected);
        let expected: HashSet<String> = ["a.txt".to_string()].into();
        assert_eq!(dirty_names(&messy.join("sub"), &[]), expected);
        assert!(dirty_names(&clean, &["sub".into()]).is_empty());
    }
}
