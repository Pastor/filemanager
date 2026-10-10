//! Фоновая файловая операция и её связь с главным циклом.
//!
//! Операция из [`crate::ops`] выполняется в отдельном потоке и общается с
//! интерфейсом через канал: присылает прогресс, вопросы о конфликтах имён и
//! итоговый отчёт. На вопрос поток ждёт ответа по отдельному каналу, который
//! приходит вместе с вопросом, — так ответ не перепутается с другим сообщением.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use crate::archive::{self, Format};
use crate::config::Side;
use crate::ops::{self, Choice, Control, Progress, Report, Transfer};

/// Не чаще этого прогресс уходит в интерфейс: копирование присылает его на
/// каждый мегабайт, а перерисовывать экран сотни раз в секунду незачем.
const PROGRESS_PERIOD: Duration = Duration::from_millis(50);

/// Что делает операция.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Copy,
    Move,
    Trash,
    Delete,
    /// Распаковка архива для входа в него.
    Extract,
    Pack,
}

/// Сообщение из фонового потока.
pub enum JobMsg {
    Progress(Progress),
    /// Конфликт имён; ответ ждут в приложенном канале.
    Conflict(PathBuf, Sender<Choice>),
    Done(Report),
}

/// Выполняющаяся операция.
pub struct Job {
    pub kind: JobKind,
    /// Панель, из которой взяты источники.
    pub source: Side,
    pub rx: Receiver<JobMsg>,
    cancel: Arc<AtomicBool>,
    pub progress: Progress,
    /// Неотвеченный конфликт: путь и канал для ответа.
    pub conflict: Option<(PathBuf, Sender<Choice>)>,
    /// Пользователь попросил отмену, операция сворачивается.
    pub cancelling: bool,
    /// Для распаковки: файл архива и временная директория, куда он
    /// распаковывается, — панель войдёт в неё по завершении.
    pub mount: Option<(PathBuf, PathBuf)>,
}

impl Job {
    /// Запускает копирование или перемещение `sources` в `dest`.
    pub fn transfer(op: Transfer, sources: Vec<PathBuf>, dest: PathBuf, source: Side) -> Self {
        let kind = match op {
            Transfer::Copy => JobKind::Copy,
            Transfer::Move => JobKind::Move,
        };
        Self::spawn(kind, source, move |ctl| {
            ops::transfer(op, &sources, &dest, ctl)
        })
    }

    /// Запускает удаление `paths` в корзину или безвозвратно.
    pub fn delete(paths: Vec<PathBuf>, to_trash: bool, source: Side) -> Self {
        let kind = if to_trash {
            JobKind::Trash
        } else {
            JobKind::Delete
        };
        Self::spawn(kind, source, move |ctl| ops::delete(&paths, to_trash, ctl))
    }

    /// Распаковывает архив во временную директорию `root`.
    pub fn extract(archive: PathBuf, root: PathBuf, source: Side) -> Self {
        let (from, to) = (archive.clone(), root.clone());
        let mut job = Self::spawn(JobKind::Extract, source, move |ctl| {
            archive::extract(&from, &to, ctl)
        });
        job.mount = Some((archive, root));
        job
    }

    /// Упаковывает `sources` в файл `dest`.
    pub fn pack(format: Format, sources: Vec<PathBuf>, dest: PathBuf, source: Side) -> Self {
        Self::spawn(JobKind::Pack, source, move |ctl| {
            archive::pack(format, &sources, &dest, ctl)
        })
    }

    fn spawn(
        kind: JobKind,
        source: Side,
        work: impl FnOnce(&mut ChannelControl) -> Report + Send + 'static,
    ) -> Self {
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut ctl = ChannelControl {
            tx,
            cancel: Arc::clone(&cancel),
            last: None,
        };
        std::thread::spawn(move || {
            let report = work(&mut ctl);
            let _ = ctl.tx.send(JobMsg::Done(report));
        });
        Self {
            kind,
            source,
            rx,
            cancel,
            progress: Progress::default(),
            conflict: None,
            cancelling: false,
            mount: None,
        }
    }

    /// Отвечает на текущий конфликт.
    pub fn answer(&mut self, choice: Choice) {
        if let Some((_, reply)) = self.conflict.take() {
            // Поток мог уже завершиться — тогда ответ просто никому не нужен.
            let _ = reply.send(choice);
        }
        if choice == Choice::Cancel {
            self.cancel();
        }
    }

    /// Просит операцию прерваться при первой возможности.
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancelling = true;
        if let Some((_, reply)) = self.conflict.take() {
            let _ = reply.send(Choice::Cancel);
        }
    }
}

/// Реализация [`Control`] поверх каналов для фонового потока.
pub struct ChannelControl {
    tx: Sender<JobMsg>,
    cancel: Arc<AtomicBool>,
    last: Option<Instant>,
}

impl Control for ChannelControl {
    fn progress(&mut self, progress: &Progress) {
        let now = Instant::now();
        if self.last.is_some_and(|t| now - t < PROGRESS_PERIOD) {
            return;
        }
        self.last = Some(now);
        let _ = self.tx.send(JobMsg::Progress(progress.clone()));
    }

    fn conflict(&mut self, target: &std::path::Path) -> Choice {
        let (reply, answer) = bounded(1);
        if self
            .tx
            .send(JobMsg::Conflict(target.to_path_buf(), reply))
            .is_err()
        {
            return Choice::Cancel;
        }
        // Интерфейс закрылся, не ответив, — безопаснее всего остановиться.
        answer.recv().unwrap_or(Choice::Cancel)
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Прокручивает сообщения задачи, отвечая на конфликты `choice`.
    fn drive(mut job: Job, choice: Choice) -> (Report, usize) {
        let mut conflicts = 0;
        loop {
            match job.rx.recv().expect("поток должен прислать отчёт") {
                JobMsg::Progress(p) => job.progress = p,
                JobMsg::Conflict(path, reply) => {
                    conflicts += 1;
                    job.conflict = Some((path, reply));
                    job.answer(choice);
                }
                JobMsg::Done(report) => return (report, conflicts),
            }
        }
    }

    #[test]
    fn background_copy_asks_and_finishes() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("a"), "new").unwrap();
        fs::write(dst.join("a"), "old").unwrap();

        let job = Job::transfer(Transfer::Copy, vec![src.join("a")], dst.clone(), Side::Left);
        let (report, conflicts) = drive(job, Choice::Replace);
        assert_eq!(conflicts, 1);
        assert_eq!(report.done, 1);
        assert_eq!(fs::read_to_string(dst.join("a")).unwrap(), "new");
    }

    #[test]
    fn cancel_answer_stops_the_job() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("a"), "new").unwrap();
        fs::write(dst.join("a"), "old").unwrap();

        let job = Job::transfer(Transfer::Move, vec![src.join("a")], dst.clone(), Side::Left);
        let (report, _) = drive(job, Choice::Cancel);
        assert!(report.cancelled);
        assert!(src.join("a").exists());
    }

    #[test]
    fn background_delete() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("x"), "").unwrap();
        let job = Job::delete(vec![tmp.path().join("x")], false, Side::Right);
        let (report, _) = drive(job, Choice::Skip);
        assert_eq!(report.done, 1);
        assert!(!tmp.path().join("x").exists());
    }
}
