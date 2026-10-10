//! Файловые операции: копирование, перемещение, удаление.
//!
//! Операции ничего не знают об интерфейсе. Прогресс, вопросы о конфликтах
//! имён и отмена идут через типаж [`Control`]: интерфейс реализует его поверх
//! каналов и запускает операцию в фоновом потоке, а тесты — синхронно, со
//! сценарием ответов.
//!
//! Ошибка на одном файле не прерывает операцию: она запоминается в отчёте,
//! и работа идёт дальше. Перемещение удаляет источник, только если всё его
//! содержимое скопировано без ошибок и пропусков — иначе данные можно потерять.

use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Вид переноса.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Copy,
    Move,
}

/// Ответ на конфликт имён.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Replace,
    Skip,
    ReplaceAll,
    SkipAll,
    Cancel,
}

/// Счётчики выполнения.
#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub items_total: u64,
    pub items_done: u64,
    /// Обрабатываемый сейчас путь.
    pub current: PathBuf,
}

/// Связь операции с тем, кто её запустил.
pub trait Control {
    /// Счётчики изменились. Вызывается часто: прореживать — забота реализации.
    fn progress(&mut self, progress: &Progress);
    /// По пути `target` уже что-то лежит: что делать?
    fn conflict(&mut self, target: &Path) -> Choice;
    /// Пользователь попросил прервать операцию.
    fn cancelled(&self) -> bool;
}

/// Итог операции.
#[derive(Debug, Default)]
pub struct Report {
    /// Элементов верхнего уровня, обработанных полностью.
    pub done: usize,
    /// Элементов верхнего уровня, пропущенных из-за конфликта.
    pub skipped: usize,
    /// Тексты ошибок в порядке появления.
    pub errors: Vec<String>,
    /// Операция прервана.
    pub cancelled: bool,
}

/// Почему перенос невозможен в принципе.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// Источник уже лежит в целевой директории.
    SameDir,
    /// Директория переносится внутрь самой себя.
    IntoItself,
}

/// Проверяет, что источники можно перенести в `dest`.
///
/// Пути сравниваются в каноническом виде: через ссылку можно попасть в ту же
/// директорию под другим именем, и бесконечное копирование внутрь себя
/// по такому пути было бы самым неприятным сюрпризом.
pub fn check(sources: &[PathBuf], dest: &Path) -> Result<(), Problem> {
    let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let dest = canon(dest);
    for src in sources {
        // Сам элемент не разыменовываем: ссылку копируем как ссылку.
        let parent = src.parent().map(canon);
        if parent.as_deref() == Some(dest.as_path()) {
            return Err(Problem::SameDir);
        }
        let is_dir = fs::symlink_metadata(src).is_ok_and(|m| m.is_dir());
        if is_dir && dest.starts_with(canon(src)) {
            return Err(Problem::IntoItself);
        }
    }
    Ok(())
}

/// Копирует или перемещает `sources` в директорию `dest`.
pub fn transfer<C: Control + ?Sized>(
    op: Transfer,
    sources: &[PathBuf],
    dest: &Path,
    ctl: &mut C,
) -> Report {
    let mut run = Runner::new(ctl);
    if let Err(problem) = check(sources, dest) {
        run.report.errors.push(format!("{problem:?}"));
        return run.report;
    }

    let sizes: Vec<(u64, u64)> = sources.iter().map(|s| measure(s)).collect();
    for (bytes, items) in &sizes {
        run.progress.bytes_total += bytes;
        run.progress.items_total += items;
    }
    run.ctl.progress(&run.progress);

    for (src, size) in sources.iter().zip(sizes) {
        if run.stop() {
            break;
        }
        let Some(name) = src.file_name() else {
            continue;
        };
        let target = dest.join(name);
        let outcome = match op {
            Transfer::Copy => run.copy(src, &target),
            Transfer::Move => run.relocate(src, &target, size),
        };
        run.count(outcome);
    }
    run.finish()
}

/// Удаляет `paths` в корзину или безвозвратно.
pub fn delete<C: Control + ?Sized>(paths: &[PathBuf], to_trash: bool, ctl: &mut C) -> Report {
    let mut run = Runner::new(ctl);
    run.progress.items_total = paths.len() as u64;
    for path in paths {
        if run.stop() {
            break;
        }
        run.progress.current = path.clone();
        run.ctl.progress(&run.progress);
        let result = if to_trash {
            trash::delete(path).map_err(|e| e.to_string())
        } else {
            fs::symlink_metadata(path)
                .and_then(|m| remove(path, &m))
                .map_err(|e| e.to_string())
        };
        match result {
            Ok(()) => run.report.done += 1,
            Err(err) => run.report.errors.push(format!("{}: {err}", path.display())),
        }
        run.progress.items_done += 1;
        run.ctl.progress(&run.progress);
    }
    run.finish()
}

/// Чем закончилась обработка одного элемента.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Обработан полностью.
    Done,
    /// Пропущен по решению пользователя.
    Skipped,
    /// Обработан частично: ошибка, пропуск внутри или отмена.
    Partial,
}

/// Место под новый элемент.
enum Room {
    /// Путь свободен (или освобождён).
    Ready,
    Skip,
    /// Освободить не удалось или операция отменена.
    Blocked,
}

struct Runner<'a, C: Control + ?Sized> {
    ctl: &'a mut C,
    progress: Progress,
    /// Ответ «для всех»: `Some(true)` — заменять, `Some(false)` — пропускать.
    policy: Option<bool>,
    report: Report,
    aborted: bool,
}

impl<'a, C: Control + ?Sized> Runner<'a, C> {
    fn new(ctl: &'a mut C) -> Self {
        Self {
            ctl,
            progress: Progress::default(),
            policy: None,
            report: Report::default(),
            aborted: false,
        }
    }

    /// Операцию пора прекращать.
    fn stop(&mut self) -> bool {
        if !self.aborted && self.ctl.cancelled() {
            self.aborted = true;
        }
        self.aborted
    }

    fn finish(mut self) -> Report {
        self.report.cancelled = self.aborted;
        self.ctl.progress(&self.progress);
        self.report
    }

    fn count(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Done => self.report.done += 1,
            Outcome::Skipped => self.report.skipped += 1,
            Outcome::Partial => {}
        }
    }

    fn error(&mut self, path: &Path, err: impl std::fmt::Display) {
        self.report
            .errors
            .push(format!("{}: {err}", path.display()));
    }

    /// Учитывает в прогрессе обработанное без копирования (пропуск, переименование).
    fn advance(&mut self, (bytes, items): (u64, u64)) {
        self.progress.bytes_done += bytes;
        self.progress.items_done += items;
        self.ctl.progress(&self.progress);
    }

    /// Готовит `target` под запись: если там что-то есть, спрашивает, что делать,
    /// и при замене удаляет существующее.
    fn make_room(&mut self, target: &Path) -> Room {
        let Ok(meta) = fs::symlink_metadata(target) else {
            return Room::Ready;
        };
        let replace = match self.policy {
            Some(replace) => replace,
            None => match self.ctl.conflict(target) {
                Choice::Replace => true,
                Choice::Skip => false,
                Choice::ReplaceAll => {
                    self.policy = Some(true);
                    true
                }
                Choice::SkipAll => {
                    self.policy = Some(false);
                    false
                }
                Choice::Cancel => {
                    self.aborted = true;
                    return Room::Blocked;
                }
            },
        };
        if !replace {
            return Room::Skip;
        }
        match remove(target, &meta) {
            Ok(()) => Room::Ready,
            Err(err) => {
                self.error(target, err);
                Room::Blocked
            }
        }
    }

    /// Копирует `src` в `dst` рекурсивно. Существующая директория сливается
    /// с копируемой без вопросов; вопросы задаются по отдельным файлам.
    fn copy(&mut self, src: &Path, dst: &Path) -> Outcome {
        if self.stop() {
            return Outcome::Partial;
        }
        self.progress.current = src.to_path_buf();
        let meta = match fs::symlink_metadata(src) {
            Ok(meta) => meta,
            Err(err) => {
                self.error(src, err);
                return Outcome::Partial;
            }
        };

        if meta.is_dir() {
            self.copy_dir(src, dst, &meta)
        } else {
            match self.make_room(dst) {
                Room::Ready => {}
                Room::Skip => {
                    self.advance(measure(src));
                    return Outcome::Skipped;
                }
                Room::Blocked => return Outcome::Partial,
            }
            let result = if meta.file_type().is_symlink() {
                copy_link(src, dst).map(|()| true)
            } else {
                self.copy_file(src, dst, &meta)
            };
            match result {
                Ok(true) => {
                    self.advance((0, 1));
                    Outcome::Done
                }
                // Отмена посреди файла: недописанная копия никому не нужна.
                Ok(false) => {
                    let _ = fs::remove_file(dst);
                    Outcome::Partial
                }
                Err(err) => {
                    self.error(src, err);
                    let _ = fs::remove_file(dst);
                    Outcome::Partial
                }
            }
        }
    }

    fn copy_dir(&mut self, src: &Path, dst: &Path, meta: &Metadata) -> Outcome {
        let merge = fs::symlink_metadata(dst).is_ok_and(|m| m.is_dir());
        if !merge {
            match self.make_room(dst) {
                Room::Ready => {}
                Room::Skip => {
                    self.advance(measure(src));
                    return Outcome::Skipped;
                }
                Room::Blocked => return Outcome::Partial,
            }
            if let Err(err) = fs::create_dir(dst) {
                self.error(dst, err);
                return Outcome::Partial;
            }
        }
        self.advance((0, 1));

        let children = match fs::read_dir(src) {
            Ok(children) => children,
            Err(err) => {
                self.error(src, err);
                return Outcome::Partial;
            }
        };
        let mut outcome = Outcome::Done;
        for child in children {
            if self.stop() {
                return Outcome::Partial;
            }
            match child {
                Ok(child) => {
                    let name = child.file_name();
                    if self.copy(&child.path(), &dst.join(&name)) != Outcome::Done {
                        outcome = Outcome::Partial;
                    }
                }
                Err(err) => {
                    self.error(src, err);
                    outcome = Outcome::Partial;
                }
            }
        }
        if !merge {
            let _ = fs::set_permissions(dst, meta.permissions());
        }
        outcome
    }

    /// Копирует содержимое файла блоками, сообщая о прогрессе.
    /// `Ok(false)` — копирование прервано отменой.
    fn copy_file(&mut self, src: &Path, dst: &Path, meta: &Metadata) -> io::Result<bool> {
        let mut input = File::open(src)?;
        // `create_new` не даёт записать сквозь ссылку, оказавшуюся на месте
        // цели: место под файл уже освобождено, и появиться там ничего не должно.
        let mut output = OpenOptions::new().write(true).create_new(true).open(dst)?;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            if self.stop() {
                return Ok(false);
            }
            let n = match input.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            };
            output.write_all(&buf[..n])?;
            self.progress.bytes_done += n as u64;
            self.ctl.progress(&self.progress);
        }
        // Права и время изменения — не повод считать копию неудачной.
        let _ = output.set_permissions(meta.permissions());
        if let Ok(modified) = meta.modified() {
            let _ = output.set_modified(modified);
        }
        Ok(true)
    }

    /// Перемещает `src` в `dst`. В пределах одной файловой системы это
    /// переименование; между файловыми системами — копия и удаление источника.
    fn relocate(&mut self, src: &Path, dst: &Path, size: (u64, u64)) -> Outcome {
        self.progress.current = src.to_path_buf();
        let src_is_dir = fs::symlink_metadata(src).is_ok_and(|m| m.is_dir());
        let dst_is_dir = fs::symlink_metadata(dst).is_ok_and(|m| m.is_dir());

        // Две директории сливаются: переименованием этого не сделать.
        if !(src_is_dir && dst_is_dir) {
            match self.make_room(dst) {
                Room::Ready => {}
                Room::Skip => {
                    self.advance(size);
                    return Outcome::Skipped;
                }
                Room::Blocked => return Outcome::Partial,
            }
            if fs::rename(src, dst).is_ok() {
                self.advance(size);
                return Outcome::Done;
            }
        }

        let outcome = self.copy(src, dst);
        if outcome != Outcome::Done {
            return outcome;
        }
        match fs::symlink_metadata(src).and_then(|m| remove(src, &m)) {
            Ok(()) => Outcome::Done,
            Err(err) => {
                self.error(src, err);
                Outcome::Partial
            }
        }
    }
}

/// Удаляет путь: директорию — целиком, ссылку — саму, не трогая цель.
fn remove(path: &Path, meta: &Metadata) -> io::Result<()> {
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(unix)]
fn copy_link(src: &Path, dst: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(src)?, dst)
}

#[cfg(not(unix))]
fn copy_link(src: &Path, dst: &Path) -> io::Result<()> {
    fs::copy(src, dst).map(|_| ())
}

/// Байты и число элементов в поддереве (ссылки не разыменовываются).
pub fn measure(path: &Path) -> (u64, u64) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if !meta.is_dir() {
        let bytes = if meta.is_file() { meta.len() } else { 0 };
        return (bytes, 1);
    }
    let mut total = (0, 1);
    if let Ok(children) = fs::read_dir(path) {
        for child in children.flatten() {
            let (bytes, items) = measure(&child.path());
            total.0 += bytes;
            total.1 += items;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Сценарий: заранее заданные ответы на конфликты. Ответ по имени файла
    /// важнее очереди: порядок обхода директории не определён.
    #[derive(Default)]
    struct Script {
        by_name: Vec<(&'static str, Choice)>,
        answers: VecDeque<Choice>,
        asked: Vec<PathBuf>,
        last: Progress,
    }

    impl Control for Script {
        fn progress(&mut self, progress: &Progress) {
            self.last = progress.clone();
        }
        fn conflict(&mut self, target: &Path) -> Choice {
            self.asked.push(target.to_path_buf());
            let name = target.file_name().unwrap().to_string_lossy();
            if let Some((_, choice)) = self.by_name.iter().find(|(n, _)| *n == name) {
                return *choice;
            }
            self.answers.pop_front().unwrap_or(Choice::Cancel)
        }
        fn cancelled(&self) -> bool {
            false
        }
    }

    fn script(answers: &[Choice]) -> Script {
        Script {
            answers: answers.iter().copied().collect(),
            ..Script::default()
        }
    }

    fn tree(root: &Path) {
        fs::create_dir_all(root.join("dir/sub")).unwrap();
        fs::write(root.join("dir/a.txt"), "aaa").unwrap();
        fs::write(root.join("dir/sub/b.txt"), "bbbb").unwrap();
        fs::write(root.join("file.txt"), "hello").unwrap();
    }

    fn read(path: impl AsRef<Path>) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn copy_tree_and_report_progress() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        tree(&src);
        fs::create_dir(&dst).unwrap();

        let mut ctl = script(&[]);
        let report = transfer(
            Transfer::Copy,
            &[src.join("dir"), src.join("file.txt")],
            &dst,
            &mut ctl,
        );
        assert_eq!(report.done, 2);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(read(dst.join("dir/sub/b.txt")), "bbbb");
        assert_eq!(read(dst.join("file.txt")), "hello");
        // Источник на месте.
        assert!(src.join("dir/a.txt").exists());
        assert_eq!(ctl.last.bytes_done, 12);
        assert_eq!(ctl.last.bytes_total, 12);
        assert_eq!(ctl.last.items_done, ctl.last.items_total);
    }

    #[test]
    fn conflict_answers_are_respected() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        tree(&src);
        fs::create_dir_all(dst.join("dir/sub")).unwrap();
        fs::write(dst.join("dir/a.txt"), "old").unwrap();
        fs::write(dst.join("dir/sub/b.txt"), "old").unwrap();
        fs::write(dst.join("file.txt"), "old").unwrap();

        let mut ctl = Script {
            by_name: vec![
                ("a.txt", Choice::Replace),
                ("b.txt", Choice::Skip),
                ("file.txt", Choice::Replace),
            ],
            ..Script::default()
        };
        let report = transfer(
            Transfer::Copy,
            &[src.join("dir"), src.join("file.txt")],
            &dst,
            &mut ctl,
        );
        assert_eq!(ctl.asked.len(), 3);
        assert_eq!(read(dst.join("dir/a.txt")), "aaa");
        assert_eq!(read(dst.join("dir/sub/b.txt")), "old");
        assert_eq!(read(dst.join("file.txt")), "hello");
        // Директория обработана не полностью: внутри был пропуск.
        assert_eq!(report.done, 1);
    }

    #[test]
    fn answer_for_all_is_asked_once() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        tree(&src);
        tree(&dst);
        fs::write(dst.join("file.txt"), "old").unwrap();

        let mut ctl = script(&[Choice::SkipAll]);
        let report = transfer(
            Transfer::Copy,
            &[src.join("dir"), src.join("file.txt")],
            &dst,
            &mut ctl,
        );
        assert_eq!(ctl.asked.len(), 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(read(dst.join("file.txt")), "old");
    }

    #[test]
    fn cancel_in_conflict_stops_everything() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        tree(&src);
        fs::create_dir(&dst).unwrap();
        fs::write(dst.join("file.txt"), "old").unwrap();

        let mut ctl = script(&[Choice::Cancel]);
        let report = transfer(
            Transfer::Copy,
            &[src.join("file.txt"), src.join("dir")],
            &dst,
            &mut ctl,
        );
        assert!(report.cancelled);
        assert!(!dst.join("dir").exists());
    }

    #[test]
    fn move_renames_and_merges() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        tree(&src);
        fs::create_dir_all(dst.join("dir")).unwrap();
        fs::write(dst.join("dir/keep.txt"), "keep").unwrap();

        let mut ctl = script(&[]);
        let report = transfer(
            Transfer::Move,
            &[src.join("dir"), src.join("file.txt")],
            &dst,
            &mut ctl,
        );
        assert_eq!(report.done, 2, "{:?}", report.errors);
        assert!(!src.join("dir").exists());
        assert!(!src.join("file.txt").exists());
        assert_eq!(read(dst.join("dir/keep.txt")), "keep");
        assert_eq!(read(dst.join("dir/sub/b.txt")), "bbbb");
        assert_eq!(read(dst.join("file.txt")), "hello");
    }

    #[test]
    fn move_keeps_source_when_something_was_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        tree(&src);
        fs::create_dir_all(dst.join("dir")).unwrap();
        fs::write(dst.join("dir/a.txt"), "old").unwrap();

        let mut ctl = script(&[Choice::Skip]);
        transfer(Transfer::Move, &[src.join("dir")], &dst, &mut ctl);
        // a.txt не перенесён — значит, исходная директория должна остаться.
        assert_eq!(read(src.join("dir/a.txt")), "aaa");
        assert_eq!(read(dst.join("dir/a.txt")), "old");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_copied_as_links() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        std::os::unix::fs::symlink("/nonexistent/target", src.join("link")).unwrap();

        let report = transfer(Transfer::Copy, &[src.join("link")], &dst, &mut script(&[]));
        assert_eq!(report.done, 1, "{:?}", report.errors);
        assert_eq!(
            fs::read_link(dst.join("link")).unwrap(),
            PathBuf::from("/nonexistent/target")
        );
    }

    #[test]
    fn check_rejects_same_dir_and_recursion() {
        let tmp = tempfile::tempdir().unwrap();
        tree(tmp.path());
        let dir = tmp.path().join("dir");
        assert_eq!(
            check(&[tmp.path().join("file.txt")], tmp.path()),
            Err(Problem::SameDir)
        );
        assert_eq!(
            check(std::slice::from_ref(&dir), &dir.join("sub")),
            Err(Problem::IntoItself)
        );
        assert_eq!(
            check(std::slice::from_ref(&dir), &dir),
            Err(Problem::IntoItself)
        );
        assert_eq!(check(&[tmp.path().join("file.txt")], &dir), Ok(()));
        // Проверка срабатывает и внутри самой операции.
        let report = transfer(
            Transfer::Copy,
            std::slice::from_ref(&dir),
            &dir.join("sub"),
            &mut script(&[]),
        );
        assert_eq!(report.errors.len(), 1);
    }

    #[test]
    fn delete_permanently() {
        let tmp = tempfile::tempdir().unwrap();
        tree(tmp.path());
        let report = delete(
            &[tmp.path().join("dir"), tmp.path().join("file.txt")],
            false,
            &mut script(&[]),
        );
        assert_eq!(report.done, 2);
        assert!(!tmp.path().join("dir").exists());
        assert!(!tmp.path().join("file.txt").exists());

        let report = delete(&[tmp.path().join("missing")], false, &mut script(&[]));
        assert_eq!(report.errors.len(), 1);
    }

    #[test]
    fn measure_counts_bytes_and_items() {
        let tmp = tempfile::tempdir().unwrap();
        tree(tmp.path());
        // dir, dir/a.txt, dir/sub, dir/sub/b.txt
        assert_eq!(measure(&tmp.path().join("dir")), (7, 4));
        assert_eq!(measure(&tmp.path().join("file.txt")), (5, 1));
        assert_eq!(measure(&tmp.path().join("missing")), (0, 0));
    }
}
