//! Состояние приложения, цикл событий и обработка клавиш.

mod finder;
mod job;
mod render;
pub mod theme;
mod viewer;

use std::path::PathBuf;

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, never, select, unbounded};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::config::{Config, LEFT_PCT_STEP, Side, clamp_left_pct};
use crate::editor::{Editor, OpenError};
use crate::i18n::Text;
use crate::ops::{self, Choice, Problem, Report, Transfer};
use crate::panel::Panel;
use crate::preview::{Body, Loader, Preview};
use crate::search::Kind as SearchKind;
use crate::ui::finder::Finder;
use crate::ui::job::{Job, JobKind, JobMsg};
use crate::ui::theme::Theme;
use crate::ui::viewer::{HSCROLL_STEP, Viewer};

/// Сообщение в строке состояния. Живёт до следующего нажатия клавиши.
pub struct Status {
    pub text: String,
    pub error: bool,
}

/// Что сейчас происходит на экране поверх панелей.
pub enum Mode {
    /// Обычная работа с панелями.
    Normal,
    /// Ожидание подтверждения удаления.
    ConfirmDelete {
        /// Безвозвратно, а не в корзину.
        permanent: bool,
        targets: Vec<PathBuf>,
    },
    /// Выполняется файловая операция.
    Job(Job),
    /// Подтверждение сохранения по Ctrl+S.
    EditorSave,
    /// Выход из редактора с несохранёнными изменениями.
    EditorExit,
}

/// Состояние приложения.
pub struct App {
    /// Левая и правая панели, индексируются через [`App::panel`].
    panels: [Panel; 2],
    /// Активная панель.
    pub active: Side,
    /// Скрытая панель. Скрыть можно только неактивную: активная всегда видна.
    pub hidden: Option<Side>,
    /// Доля ширины окна под левую панель, в процентах.
    pub left_pct: u16,
    pub cfg: Config,
    pub theme: Theme,
    pub status: Option<Status>,
    pub mode: Mode,
    /// Панель просмотра, если открыта. Она занимает место одной из панелей.
    pub viewer: Option<Viewer>,
    /// Редактор на месте просмотра, если открыт.
    pub editor: Option<Editor>,
    /// Поиск в панели, если открыт.
    pub finder: Option<Finder>,
    loader: Loader,
    pub quit: bool,
}

impl App {
    fn new(left: PathBuf, right: PathBuf, cfg: Config) -> Self {
        let theme = Theme::detect(cfg.theme);
        let session = cfg.session.clone();
        let mut status = None;
        let mut open = |dir: PathBuf, columns: u8| {
            Panel::open(dir.clone(), columns, cfg.show_hidden).unwrap_or_else(|err| {
                status = Some(Status {
                    text: format!("{}: {}: {err}", cfg.lang.text().cannot_open, dir.display()),
                    error: true,
                });
                Panel::empty(dir, columns)
            })
        };
        let panels = [
            open(left, session.left_columns),
            open(right, session.right_columns),
        ];
        Self {
            panels,
            active: Side::Left,
            hidden: session.hidden.filter(|&side| side != Side::Left),
            left_pct: clamp_left_pct(session.left_pct),
            cfg,
            theme,
            status,
            mode: Mode::Normal,
            viewer: None,
            editor: None,
            finder: None,
            loader: Loader::new(),
            quit: false,
        }
    }

    /// Приложение с обеими панелями в `dir` и настройками по умолчанию.
    #[cfg(test)]
    pub fn new_for_test(dir: &std::path::Path) -> Self {
        Self::new(dir.to_path_buf(), dir.to_path_buf(), Config::default())
    }

    /// Строки интерфейса на выбранном языке.
    pub fn text(&self) -> &'static Text {
        self.cfg.lang.text()
    }

    /// Панель на заданной стороне.
    pub fn panel(&self, side: Side) -> &Panel {
        &self.panels[side as usize]
    }

    pub fn panel_mut(&mut self, side: Side) -> &mut Panel {
        &mut self.panels[side as usize]
    }

    fn active_panel(&mut self) -> &mut Panel {
        self.panel_mut(self.active)
    }

    fn info(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: false,
        });
    }

    fn fail(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: true,
        });
    }

    /// Сообщает о неудачной попытке открыть директорию.
    fn fail_open(&mut self, dir: PathBuf, err: std::io::Error) {
        let text = format!("{}: {}: {err}", self.text().cannot_open, dir.display());
        self.fail(text);
    }

    fn on_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            // Операция сначала сворачивается: выход случится по её отчёту.
            if let Mode::Job(job) = &mut self.mode {
                job.cancel();
            }
            self.quit = true;
            return;
        }
        // Пока идёт операция или открыт диалог, клавиши принадлежат им.
        match self.mode {
            Mode::Normal => {}
            Mode::ConfirmDelete { .. } => return self.on_confirm_key(key.code),
            Mode::Job(_) => return self.on_job_key(key.code),
            Mode::EditorSave => return self.on_save_key(key.code),
            Mode::EditorExit => return self.on_exit_key(key.code),
        }
        // Редактор сам разбирает сочетания с Ctrl.
        if self.editor.is_some() {
            self.status = None;
            return self.on_editor_key(key);
        }
        // Сочетания с Ctrl и Alt пока не используются; Shift входит в сам символ.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return;
        }
        self.status = None;

        if self.finder.is_some() {
            self.on_finder_key(key.code);
        } else if self.viewer_focused() {
            self.on_viewer_key(key.code);
        } else {
            self.on_files_key(key.code);
        }
        self.sync_viewer();
    }

    /// Клавиши файловой панели.
    fn on_files_key(&mut self, code: KeyCode) {
        let show_hidden = self.cfg.show_hidden;
        match code {
            KeyCode::Up => self.active_panel().move_by(-1),
            KeyCode::Down => self.active_panel().move_by(1),
            KeyCode::PageUp => {
                let page = self.active_panel().rows as isize;
                self.active_panel().move_by(-page);
            }
            KeyCode::PageDown => {
                let page = self.active_panel().rows as isize;
                self.active_panel().move_by(page);
            }
            KeyCode::Home => self.active_panel().home(),
            KeyCode::End => self.active_panel().end(),
            // Enter на файле открывает его в просмотре и переводит туда фокус.
            KeyCode::Enter
                if self
                    .active_panel()
                    .current()
                    .is_some_and(|e| !e.kind.is_dir()) =>
            {
                if self.viewer.is_none() {
                    self.open_viewer();
                }
                if let Some(viewer) = &self.viewer {
                    self.active = viewer.side;
                }
            }
            KeyCode::Right | KeyCode::Enter => {
                let target = self.active_panel().current().map(|e| e.path.clone());
                if let Err(err) = self.active_panel().enter(show_hidden)
                    && let Some(dir) = target
                {
                    self.fail_open(dir, err);
                }
            }
            KeyCode::Left => {
                let target = self.active_panel().dir.parent().map(PathBuf::from);
                if let Err(err) = self.active_panel().leave(show_hidden)
                    && let Some(dir) = target
                {
                    self.fail_open(dir, err);
                }
            }
            KeyCode::Tab => {
                if self.hidden.is_none() {
                    self.active = self.active.other();
                }
            }
            KeyCode::Char('>') => self.widen(LEFT_PCT_STEP as i16),
            KeyCode::Char('<') => self.widen(-(LEFT_PCT_STEP as i16)),
            KeyCode::Char('p') => {
                if self.viewer.is_some() {
                    self.close_viewer();
                } else {
                    self.open_viewer();
                }
            }
            KeyCode::Esc => self.close_viewer(),
            KeyCode::Char('/') => self.open_finder(SearchKind::Content),
            KeyCode::Char('f') => self.open_finder(SearchKind::Name),
            KeyCode::Char('P') => {
                // Скрытие второй панели закрывает и просмотр на её месте.
                self.close_viewer();
                self.hidden = match self.hidden {
                    Some(_) => None,
                    None => Some(self.active.other()),
                };
            }
            KeyCode::Char(c @ '1'..='3') => {
                let n = c as u8 - b'0';
                self.active_panel().set_columns(n);
                let text = format!("{}: {n}", self.text().columns);
                self.info(text);
            }
            KeyCode::Char('.') => self.toggle_hidden(),
            KeyCode::Char(' ') => self.active_panel().toggle_mark(),
            KeyCode::Char('c') => self.start_transfer(Transfer::Copy),
            KeyCode::Char('m') => self.start_transfer(Transfer::Move),
            KeyCode::Char('d') => self.ask_delete(!self.cfg.use_trash),
            KeyCode::Char('D') => self.ask_delete(true),
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
    }

    /// Фокус в панели просмотра.
    fn viewer_focused(&self) -> bool {
        self.viewer.as_ref().is_some_and(|v| v.side == self.active)
    }

    /// Открывает просмотр на месте второй панели.
    fn open_viewer(&mut self) {
        self.hidden = None;
        self.viewer = Some(Viewer::new(self.active.other(), self.cfg.wrap));
        self.sync_viewer();
    }

    /// Закрывает просмотр; фокус возвращается в файловую панель.
    fn close_viewer(&mut self) {
        if let Some(viewer) = self.viewer.take() {
            self.active = viewer.side.other();
        }
    }

    /// Просмотр следует за курсором файловой панели: если под курсором
    /// другой путь, просит загрузчик прочитать его.
    fn sync_viewer(&mut self) {
        let Some(viewer) = &self.viewer else {
            return;
        };
        // Во время поиска просмотр показывает выбранную находку.
        if let Some(finder) = &self.finder {
            let Some(hit) = finder.current() else {
                return;
            };
            let wanted = (hit.path.clone(), hit.line);
            if finder.shown.as_ref() == Some(&wanted) {
                return;
            }
            let query = finder.query.clone();
            if viewer.path.as_ref() != Some(&wanted.0) {
                let request = self.loader.request(wanted.0.clone());
                if let Some(viewer) = &mut self.viewer {
                    viewer.show(wanted.0.clone(), request);
                }
            }
            if let (Some(viewer), Some(line)) = (&mut self.viewer, wanted.1) {
                viewer.focus(&query, line);
            }
            if let Some(finder) = &mut self.finder {
                finder.shown = Some(wanted);
            }
            return;
        }
        let panel = self.panel(viewer.side.other());
        let target = panel
            .current()
            .map(|e| e.path.clone())
            .unwrap_or_else(|| panel.dir.clone());
        if viewer.path.as_ref() == Some(&target) {
            return;
        }
        let request = self.loader.request(target.clone());
        if let Some(viewer) = &mut self.viewer {
            viewer.show(target, request);
        }
    }

    /// Открывает поиск в активной панели. Если вторая панель видна, в ней
    /// открывается просмотр находок.
    fn open_finder(&mut self, kind: SearchKind) {
        let root = self.panel(self.active).dir.clone();
        let mut finder = Finder::new(kind, self.active, root, self.cfg.show_hidden);
        if self.hidden.is_none() && self.viewer.is_none() {
            self.viewer = Some(Viewer::new(self.active.other(), self.cfg.wrap));
            finder.opened_viewer = true;
        }
        self.finder = Some(finder);
    }

    /// Закрывает поиск. `go` — перейти к выбранной находке в панели.
    fn close_finder(&mut self, go: bool) {
        let Some(finder) = self.finder.take() else {
            return;
        };
        let hit = finder.current().cloned().filter(|_| go);
        if let Some(hit) = &hit
            && let Some(dir) = hit.path.parent()
        {
            let show = self.cfg.show_hidden;
            let panel = self.panel_mut(finder.side);
            match panel.cd(dir.to_path_buf(), show) {
                Ok(()) => {
                    panel.select_name(&hit.path.file_name().unwrap_or_default().to_string_lossy())
                }
                Err(err) => self.fail_open(dir.to_path_buf(), err),
            }
        }
        // Просмотр, открытый поиском, уходит вместе с ним — если только
        // пользователь не перешёл к находке: тогда она остаётся на виду.
        if finder.opened_viewer && hit.is_none() {
            self.close_viewer();
        } else if let Some(viewer) = &mut self.viewer {
            viewer.search = None;
            if hit.is_none() {
                viewer.path = None;
            }
        }
        self.active = finder.side;
    }

    /// Клавиши поиска в панели.
    fn on_finder_key(&mut self, code: KeyCode) {
        let Some(finder) = &mut self.finder else {
            return;
        };
        match code {
            KeyCode::Char(c) => {
                let query = format!("{}{c}", finder.query);
                finder.set_query(query);
                finder.shown = None;
            }
            KeyCode::Backspace => {
                let mut query = finder.query.clone();
                query.pop();
                finder.set_query(query);
                finder.shown = None;
            }
            KeyCode::Up => finder.move_by(-1),
            KeyCode::Down => finder.move_by(1),
            KeyCode::PageUp => finder.move_by(-(finder.rows as isize)),
            KeyCode::PageDown => finder.move_by(finder.rows as isize),
            KeyCode::Home => finder.home(),
            KeyCode::End => finder.end(),
            KeyCode::Enter => self.close_finder(true),
            KeyCode::Esc => self.close_finder(false),
            _ => {}
        }
    }

    /// Сообщение фонового поиска.
    fn on_finder_msg(&mut self, msg: crate::search::Msg) {
        if let Some(finder) = &mut self.finder {
            finder.on_msg(msg);
        }
        // Первые находки сразу показываются в просмотре.
        self.sync_viewer();
    }

    /// Результат загрузчика.
    fn on_preview_loaded(&mut self, request: u64, preview: Preview) -> bool {
        self.viewer
            .as_mut()
            .is_some_and(|viewer| viewer.loaded(request, preview))
    }

    /// Клавиши панели просмотра.
    fn on_viewer_key(&mut self, code: KeyCode) {
        let Some(viewer) = &mut self.viewer else {
            return;
        };
        if viewer.search.is_some() {
            let query = viewer
                .search
                .as_ref()
                .map(|s| s.query.clone())
                .unwrap_or_default();
            match code {
                KeyCode::Char(c) => viewer.set_query(format!("{query}{c}")),
                KeyCode::Backspace => {
                    let mut query = query;
                    query.pop();
                    viewer.set_query(query);
                }
                KeyCode::Right | KeyCode::Enter => viewer.jump(1),
                KeyCode::Left => viewer.jump(-1),
                KeyCode::Up => viewer.scroll(-1),
                KeyCode::Down => viewer.scroll(1),
                KeyCode::PageUp => viewer.page(-1),
                KeyCode::PageDown => viewer.page(1),
                KeyCode::Esc => viewer.search = None,
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Up => viewer.scroll(-1),
            KeyCode::Down => viewer.scroll(1),
            KeyCode::PageUp => viewer.page(-1),
            KeyCode::PageDown => viewer.page(1),
            KeyCode::Home => viewer.home(),
            KeyCode::End => viewer.end(),
            KeyCode::Left => viewer.hscroll(-(HSCROLL_STEP as isize)),
            KeyCode::Right => viewer.hscroll(HSCROLL_STEP as isize),
            KeyCode::Char('w') => viewer.toggle_wrap(),
            KeyCode::Char('e') => self.start_editing(),
            KeyCode::Char('/') => {
                viewer.start_search();
            }
            KeyCode::Tab => self.active = self.active.other(),
            KeyCode::Esc | KeyCode::Char('p') => self.close_viewer(),
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
    }

    /// Открывает показанный в просмотре файл на правку.
    fn start_editing(&mut self) {
        let Some(viewer) = &self.viewer else {
            return;
        };
        let Some(content) = &viewer.content else {
            return;
        };
        if !matches!(content.body, Body::Text(_)) {
            let text = self.text().not_text;
            self.fail(text);
            return;
        }
        let (path, top) = (content.path.clone(), viewer.top);
        match Editor::open(&path, self.cfg.indent as usize) {
            Ok(mut editor) => {
                // Курсор там, где пользователь читал.
                editor.down(top);
                editor.top = top;
                self.editor = Some(editor);
            }
            Err(err) => {
                let t = self.text();
                let text = match err {
                    OpenError::NotText => t.not_text.to_string(),
                    OpenError::TooLarge => t.too_large.to_string(),
                    OpenError::Io(err) => format!("{}: {err}", t.cannot_read),
                };
                self.fail(text);
            }
        }
    }

    /// Клавиши редактора.
    fn on_editor_key(&mut self, key: KeyEvent) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.modifiers.contains(KeyModifiers::ALT) {
            return;
        }
        let page = editor.rows.max(1);
        match key.code {
            KeyCode::Char('s') if ctrl => {
                if editor.modified {
                    self.mode = Mode::EditorSave;
                } else {
                    let text = self.text().nothing_to_save;
                    self.info(text);
                }
            }
            KeyCode::Home if ctrl => editor.file_start(),
            KeyCode::End if ctrl => editor.file_end(),
            _ if ctrl => {}
            KeyCode::Esc => {
                if editor.modified {
                    self.mode = Mode::EditorExit;
                } else {
                    self.close_editor();
                }
            }
            KeyCode::Char(c) => editor.insert(c),
            KeyCode::Enter => editor.newline(),
            KeyCode::Backspace => editor.backspace(),
            KeyCode::Delete => editor.delete(),
            KeyCode::Tab => editor.tab(),
            KeyCode::BackTab => editor.backtab(),
            KeyCode::Up => editor.up(1),
            KeyCode::Down => editor.down(1),
            KeyCode::Left => editor.left(),
            KeyCode::Right => editor.right(),
            KeyCode::Home => editor.home(),
            KeyCode::End => editor.end(),
            KeyCode::PageUp => editor.up(page),
            KeyCode::PageDown => editor.down(page),
            _ => {}
        }
    }

    /// Подтверждение сохранения: `y` или Enter сохраняют, остальное отменяет.
    fn on_save_key(&mut self, code: KeyCode) {
        self.mode = Mode::Normal;
        if matches!(code, KeyCode::Char('y') | KeyCode::Enter) {
            self.save_editor();
        }
    }

    /// Выход с несохранёнными изменениями: `y` — сохранить и закрыть,
    /// `n` — закрыть без сохранения, Esc — вернуться к правке.
    fn on_exit_key(&mut self, code: KeyCode) {
        self.mode = Mode::Normal;
        match code {
            KeyCode::Char('y') | KeyCode::Enter => {
                if self.save_editor() {
                    self.close_editor();
                }
            }
            KeyCode::Char('n') => self.close_editor(),
            _ => {}
        }
    }

    /// Записывает файл редактора; `false` — не удалось.
    fn save_editor(&mut self) -> bool {
        let Some(editor) = &mut self.editor else {
            return false;
        };
        match editor.save() {
            Ok(()) => {
                // Размер и время файла изменились — панели и просмотр тоже.
                self.refresh_panels();
                let text = self.text().saved;
                self.info(text);
                true
            }
            Err(err) => {
                let text = format!("{}: {err}", self.text().save_failed);
                self.fail(text);
                false
            }
        }
    }

    /// Закрывает редактор и перечитывает просмотр: файл мог измениться.
    fn close_editor(&mut self) {
        let top = self.editor.take().map(|e| e.top);
        if let Some(viewer) = &mut self.viewer {
            viewer.path = None;
        }
        self.sync_viewer();
        if let (Some(viewer), Some(top)) = (&mut self.viewer, top) {
            viewer.top = top;
        }
    }

    /// Запускает копирование или перенос во вторую панель.
    fn start_transfer(&mut self, op: Transfer) {
        if self.hidden.is_some() || self.viewer.is_some() {
            let text = self.text().no_second_panel;
            self.fail(text);
            return;
        }
        let sources = self.panel(self.active).targets();
        if sources.is_empty() {
            return;
        }
        let dest = self.panel(self.active.other()).dir.clone();
        if let Err(problem) = ops::check(&sources, &dest) {
            let text = self.text();
            self.fail(match problem {
                Problem::SameDir => text.same_dir,
                Problem::IntoItself => text.into_itself,
            });
            return;
        }
        self.mode = Mode::Job(Job::transfer(op, sources, dest, self.active));
    }

    /// Открывает подтверждение удаления.
    fn ask_delete(&mut self, permanent: bool) {
        let targets = self.panel(self.active).targets();
        if !targets.is_empty() {
            self.mode = Mode::ConfirmDelete { permanent, targets };
        }
    }

    /// Клавиши диалога удаления: повтор той же клавиши подтверждает,
    /// любая другая отменяет.
    fn on_confirm_key(&mut self, code: KeyCode) {
        let Mode::ConfirmDelete { permanent, targets } =
            std::mem::replace(&mut self.mode, Mode::Normal)
        else {
            return;
        };
        let confirm = if permanent { 'D' } else { 'd' };
        if code == KeyCode::Char(confirm) {
            self.mode = Mode::Job(Job::delete(targets, !permanent, self.active));
        }
    }

    /// Клавиши во время операции: ответы на конфликт и отмена.
    fn on_job_key(&mut self, code: KeyCode) {
        let Mode::Job(job) = &mut self.mode else {
            return;
        };
        if job.conflict.is_some() {
            let choice = match code {
                KeyCode::Char('r') => Choice::Replace,
                KeyCode::Char('s') => Choice::Skip,
                KeyCode::Char('R') => Choice::ReplaceAll,
                KeyCode::Char('S') => Choice::SkipAll,
                KeyCode::Esc => Choice::Cancel,
                _ => return,
            };
            job.answer(choice);
        } else if code == KeyCode::Esc {
            job.cancel();
        }
    }

    /// Сообщение от фоновой операции.
    fn on_job_msg(&mut self, msg: JobMsg) {
        let Mode::Job(job) = &mut self.mode else {
            return;
        };
        match msg {
            JobMsg::Progress(progress) => job.progress = progress,
            JobMsg::Conflict(path, reply) => {
                job.conflict = Some((path, reply));
                // Отмену могли запросить, пока вопрос был в пути.
                if job.cancelling {
                    job.answer(Choice::Cancel);
                }
            }
            JobMsg::Done(report) => {
                let (kind, source) = (job.kind, job.source);
                self.mode = Mode::Normal;
                self.finish_job(kind, source, report);
            }
        }
    }

    /// Поток операции исчез, не прислав отчёта (например, паника).
    fn on_job_lost(&mut self) {
        if let Mode::Job(job) = std::mem::replace(&mut self.mode, Mode::Normal) {
            self.panel_mut(job.source).marked.clear();
        }
        self.refresh_panels();
        let text = self.text().job_lost;
        self.fail(text);
    }

    /// Обновляет панели после операции и сообщает итог.
    fn finish_job(&mut self, kind: JobKind, source: Side, report: Report) {
        self.panel_mut(source).marked.clear();
        self.refresh_panels();
        let text = summary(self.text(), kind, &report);
        if report.errors.is_empty() {
            self.info(text);
        } else {
            self.fail(text);
        }
    }

    /// Перечитывает обе панели. Если директория панели исчезла (её удалили
    /// или перенесли), панель поднимается до ближайшего существующего предка.
    fn refresh_panels(&mut self) {
        let show = self.cfg.show_hidden;
        for side in [Side::Left, Side::Right] {
            let panel = self.panel_mut(side);
            if panel.reload(show).is_ok() {
                continue;
            }
            let mut dir = panel.dir.clone();
            while !dir.is_dir() {
                match dir.parent() {
                    Some(parent) => dir = parent.to_path_buf(),
                    None => break,
                }
            }
            if let Err(err) = panel.cd(dir.clone(), show) {
                self.fail_open(dir, err);
            }
        }
        // Показанный файл мог измениться или исчезнуть — перечитываем.
        if let Some(viewer) = &mut self.viewer {
            viewer.path = None;
        }
        self.sync_viewer();
    }

    /// Расширяет активную панель на `delta` процентов (сужает при отрицательном).
    fn widen(&mut self, delta: i16) {
        if self.hidden.is_some() {
            return;
        }
        let delta = match self.active {
            Side::Left => delta,
            Side::Right => -delta,
        };
        self.left_pct = clamp_left_pct((self.left_pct as i16 + delta).max(0) as u16);
    }

    /// Переключает показ скрытых файлов в обеих панелях.
    fn toggle_hidden(&mut self) {
        self.cfg.show_hidden = !self.cfg.show_hidden;
        let show = self.cfg.show_hidden;
        for side in [Side::Left, Side::Right] {
            if let Err(err) = self.panel_mut(side).reload(show) {
                let dir = self.panel(side).dir.clone();
                self.fail_open(dir, err);
                return;
            }
        }
        let text = self.text();
        self.info(if show {
            text.hidden_shown
        } else {
            text.hidden_concealed
        });
    }

    /// Переносит состояние окна в конфиг перед сохранением.
    fn store_session(&mut self) {
        let session = &mut self.cfg.session;
        session.right = Some(self.panels[Side::Right as usize].dir.clone());
        session.left_pct = self.left_pct;
        session.left_columns = self.panels[Side::Left as usize].columns;
        session.right_columns = self.panels[Side::Right as usize].columns;
        session.hidden = self.hidden;
    }
}

/// Поток чтения событий терминала: главный цикл ждёт сразу несколько
/// источников (позже — фоновые операции), поэтому события идут через канал.
fn spawn_events() -> Receiver<Event> {
    let (tx, rx): (Sender<Event>, Receiver<Event>) = unbounded();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if tx.send(ev).is_err() {
                break;
            }
        }
    });
    rx
}

/// Точка входа TUI: инициализация терминала, главный цикл, сохранение сессии.
///
/// `cfg_ok` — конфиг прочитан без ошибок. Если нет, файл не перезаписывается:
/// иначе одна опечатка стоила бы пользователю всех его настроек.
pub fn run(left: PathBuf, right: PathBuf, cfg: Config, cfg_ok: bool) -> Result<()> {
    let mut app = App::new(left, right, cfg);
    let mut terminal = ratatui::init();
    let result = main_loop(&mut terminal, &mut app);
    ratatui::restore();
    result?;

    if cfg_ok {
        app.store_session();
        if let Err(err) = app.cfg.save() {
            eprintln!("{}: {err:#}", app.text().config_not_saved);
        }
    }
    Ok(())
}

/// Итог операции одной строкой: «скопировано: 3, пропущено: 1, ошибок: 2 — …».
fn summary(text: &Text, kind: JobKind, report: &Report) -> String {
    let verb = match kind {
        JobKind::Copy => text.done_copy,
        JobKind::Move => text.done_move,
        JobKind::Trash => text.done_trash,
        JobKind::Delete => text.done_delete,
    };
    let mut parts = vec![format!("{verb}: {}", report.done)];
    if report.skipped > 0 {
        parts.push(format!("{}: {}", text.skipped, report.skipped));
    }
    if report.cancelled {
        parts.push(text.cancelled.to_string());
    }
    if let Some(first) = report.errors.first() {
        parts.push(format!(
            "{}: {} — {first}",
            text.errors,
            report.errors.len()
        ));
    }
    parts.join(", ")
}

fn main_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let events = spawn_events();
    // Канал загрузчика живёт столько же, сколько приложение.
    let previews = app.loader.rx.clone();
    let mut dirty = true;
    loop {
        if dirty {
            terminal.draw(|frame| render::draw(frame, app))?;
            dirty = false;
        }
        // Выход ждёт окончания операции: оборванное копирование хуже задержки.
        if app.quit && !matches!(app.mode, Mode::Job(_)) {
            return Ok(());
        }
        let jobs = match &app.mode {
            Mode::Job(job) => job.rx.clone(),
            _ => never(),
        };
        let found = app
            .finder
            .as_ref()
            .and_then(Finder::rx)
            .unwrap_or_else(never);
        select! {
            recv(found) -> msg => {
                if let Ok(msg) = msg {
                    app.on_finder_msg(msg);
                    dirty = true;
                }
            }
            recv(jobs) -> msg => {
                match msg {
                    Ok(msg) => app.on_job_msg(msg),
                    Err(_) => app.on_job_lost(),
                }
                dirty = true;
            }
            recv(previews) -> loaded => {
                if let Ok((request, preview)) = loaded {
                    dirty |= app.on_preview_loaded(request, preview);
                }
            }
            recv(events) -> ev => match ev {
                Ok(Event::Key(key)) => { app.on_key(key); dirty = true; }
                Ok(Event::Resize(_, _)) => dirty = true,
                Ok(_) => {}
                // Поток чтения событий умер — работать дальше нечем.
                Err(_) => return Ok(()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LEFT_PCT_MAX, LEFT_PCT_MIN};

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn app_in(dir: &std::path::Path) -> App {
        App::new_for_test(dir)
    }

    #[test]
    fn tab_switches_panels_unless_one_is_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.active, Side::Right);

        // Скрывается неактивная панель, переключение блокируется.
        press(&mut app, KeyCode::Char('P'));
        assert_eq!(app.hidden, Some(Side::Left));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.active, Side::Right);

        press(&mut app, KeyCode::Char('P'));
        assert_eq!(app.hidden, None);
    }

    #[test]
    fn width_keys_grow_the_active_panel() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        press(&mut app, KeyCode::Char('>'));
        assert_eq!(app.left_pct, 50 + LEFT_PCT_STEP);

        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('>'));
        press(&mut app, KeyCode::Char('>'));
        assert_eq!(app.left_pct, 50 - LEFT_PCT_STEP);

        for _ in 0..50 {
            press(&mut app, KeyCode::Char('>'));
        }
        assert_eq!(app.left_pct, LEFT_PCT_MIN);
        for _ in 0..50 {
            press(&mut app, KeyCode::Char('<'));
        }
        assert_eq!(app.left_pct, LEFT_PCT_MAX);
    }

    #[test]
    fn digits_set_columns_of_active_panel() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.panel(Side::Left).columns, 3);
        assert_eq!(app.panel(Side::Right).columns, 1);
    }

    #[test]
    fn dot_toggles_hidden_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".secret"), "").unwrap();
        std::fs::write(tmp.path().join("plain"), "").unwrap();
        let mut app = app_in(tmp.path());
        assert_eq!(app.panel(Side::Left).entries.len(), 2);
        press(&mut app, KeyCode::Char('.'));
        assert_eq!(app.panel(Side::Left).entries.len(), 1);
        assert_eq!(app.panel(Side::Right).entries.len(), 1);
        assert!(!app.cfg.show_hidden);
    }

    #[test]
    fn arrows_enter_and_leave_directories() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let mut app = app_in(tmp.path());
        press(&mut app, KeyCode::Right);
        assert_eq!(app.panel(Side::Left).dir, tmp.path().join("sub"));
        press(&mut app, KeyCode::Left);
        assert_eq!(app.panel(Side::Left).dir, tmp.path());
        assert_eq!(app.panel(Side::Left).current().unwrap().name, "sub");
    }

    #[test]
    fn session_is_stored_into_config() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = app_in(tmp.path());
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('P'));
        app.store_session();
        assert_eq!(app.cfg.session.left_columns, 2);
        assert_eq!(app.cfg.session.hidden, Some(Side::Right));
        assert_eq!(app.cfg.session.right.as_deref(), Some(tmp.path()));
    }

    /// Дожидается конца операции, отвечая на конфликты клавишей `answer`.
    fn finish(app: &mut App, answer: KeyCode) {
        while let Mode::Job(job) = &app.mode {
            let msg = job.rx.recv().expect("операция должна прислать отчёт");
            app.on_job_msg(msg);
            if matches!(&app.mode, Mode::Job(job) if job.conflict.is_some()) {
                press(app, answer);
            }
        }
    }

    /// Две директории: `left` с файлами и пустая `right`; правая панель в `right`.
    fn two_dirs() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        let (left, right) = (tmp.path().join("left"), tmp.path().join("right"));
        std::fs::create_dir_all(left.join("dir")).unwrap();
        std::fs::write(left.join("dir/inner"), "i").unwrap();
        std::fs::write(left.join("a"), "a").unwrap();
        std::fs::write(left.join("b"), "b").unwrap();
        std::fs::create_dir_all(&right).unwrap();
        let app = App::new(left, right, Config::default());
        (tmp, app)
    }

    #[test]
    fn copy_marked_files_to_other_panel() {
        let (tmp, mut app) = two_dirs();
        // Порядок: dir/, a, b. Выделяем dir и b.
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(app.panel(Side::Left).marked.len(), 2);
        press(&mut app, KeyCode::Char('c'));
        finish(&mut app, KeyCode::Esc);

        let right = tmp.path().join("right");
        assert!(right.join("dir/inner").exists());
        assert!(right.join("b").exists());
        assert!(!right.join("a").exists());
        // Панели перечитаны, выделение снято, итог в строке состояния.
        assert_eq!(app.panel(Side::Right).entries.len(), 2);
        assert!(app.panel(Side::Left).marked.is_empty());
        let status = app.status.as_ref().unwrap();
        assert!(!status.error);
        assert!(status.text.contains("copied: 2"), "{}", status.text);
    }

    #[test]
    fn move_without_marks_takes_item_under_cursor() {
        let (tmp, mut app) = two_dirs();
        press(&mut app, KeyCode::Down); // a
        press(&mut app, KeyCode::Char('m'));
        finish(&mut app, KeyCode::Esc);
        assert!(!tmp.path().join("left/a").exists());
        assert!(tmp.path().join("right/a").exists());
        assert_eq!(app.panel(Side::Left).entries.len(), 2);
    }

    #[test]
    fn conflict_is_answered_from_keyboard() {
        let (tmp, mut app) = two_dirs();
        std::fs::write(tmp.path().join("right/a"), "old").unwrap();
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('c'));
        finish(&mut app, KeyCode::Char('s'));
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("right/a")).unwrap(),
            "old"
        );
        assert!(app.status.as_ref().unwrap().text.contains("skipped: 1"));

        press(&mut app, KeyCode::Char('c'));
        finish(&mut app, KeyCode::Char('r'));
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("right/a")).unwrap(),
            "a"
        );
    }

    #[test]
    fn transfer_needs_a_visible_distinct_panel() {
        let (_tmp, mut app) = two_dirs();
        press(&mut app, KeyCode::Char('P'));
        press(&mut app, KeyCode::Char('c'));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.status.as_ref().unwrap().error);

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("f"), "").unwrap();
        let mut app = app_in(tmp.path());
        press(&mut app, KeyCode::Char('m'));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.status.as_ref().unwrap().text.contains("same directory"));
    }

    #[test]
    fn delete_requires_repeated_key() {
        let (tmp, mut app) = two_dirs();
        press(&mut app, KeyCode::Down); // a
        // Любая другая клавиша отменяет удаление.
        press(&mut app, KeyCode::Char('D'));
        assert!(matches!(
            app.mode,
            Mode::ConfirmDelete {
                permanent: true,
                ..
            }
        ));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert!(tmp.path().join("left/a").exists());

        // Для безвозвратного удаления нужно именно `D`, а не `d`.
        press(&mut app, KeyCode::Char('D'));
        press(&mut app, KeyCode::Char('d'));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(tmp.path().join("left/a").exists());

        press(&mut app, KeyCode::Char('D'));
        press(&mut app, KeyCode::Char('D'));
        finish(&mut app, KeyCode::Esc);
        assert!(!tmp.path().join("left/a").exists());
        assert!(app.status.as_ref().unwrap().text.contains("deleted: 1"));
    }

    #[test]
    fn panel_inside_deleted_directory_climbs_up() {
        let (tmp, mut app) = two_dirs();
        let dir = tmp.path().join("left/dir");
        app.panel_mut(Side::Right).cd(dir.clone(), true).unwrap();
        // Удаляем dir из левой панели безвозвратно.
        press(&mut app, KeyCode::Char('D'));
        press(&mut app, KeyCode::Char('D'));
        finish(&mut app, KeyCode::Esc);
        assert!(!dir.exists());
        assert_eq!(app.panel(Side::Right).dir, tmp.path().join("left"));
    }

    /// Дожидается загрузки просмотра.
    pub(super) fn wait_preview(app: &mut App) {
        let rx = app.loader.rx.clone();
        while app.viewer.as_ref().is_some_and(|v| v.content.is_none()) {
            let (request, preview) = rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("загрузчик должен ответить");
            app.on_preview_loaded(request, preview);
        }
    }

    fn viewer_dir() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        let code: String = (1..=60).map(|i| format!("let x{i} = {i};\n")).collect();
        std::fs::write(tmp.path().join("main.rs"), code).unwrap();
        std::fs::write(tmp.path().join("notes.txt"), "hello").unwrap();
        let app = app_in(tmp.path());
        (tmp, app)
    }

    #[test]
    fn p_opens_viewer_that_follows_cursor() {
        let (tmp, mut app) = viewer_dir();
        press(&mut app, KeyCode::Char('p'));
        let viewer = app.viewer.as_ref().unwrap();
        assert_eq!(viewer.side, Side::Right);
        assert_eq!(
            viewer.path.as_deref(),
            Some(tmp.path().join("sub").as_path())
        );
        // Фокус остаётся в списке файлов.
        assert_eq!(app.active, Side::Left);

        press(&mut app, KeyCode::Down);
        assert_eq!(
            app.viewer.as_ref().unwrap().path.as_deref(),
            Some(tmp.path().join("main.rs").as_path())
        );
        wait_preview(&mut app);
        assert!(app.viewer.as_ref().unwrap().content.is_some());

        // Копировать некуда: вторая панель занята просмотром.
        press(&mut app, KeyCode::Char('c'));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.status.as_ref().unwrap().error);

        press(&mut app, KeyCode::Char('p'));
        assert!(app.viewer.is_none());
    }

    #[test]
    fn enter_on_file_focuses_viewer_and_esc_returns() {
        let (_tmp, mut app) = viewer_dir();
        press(&mut app, KeyCode::Down); // main.rs
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.active, Side::Right);
        wait_preview(&mut app);

        // Клавиши идут в просмотр: прокрутка, перенос.
        app.viewer.as_mut().unwrap().fit(10, 50);
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.viewer.as_ref().unwrap().top, 10);
        press(&mut app, KeyCode::Char('w'));
        assert!(!app.viewer.as_ref().unwrap().wrap);

        // Tab — обратно в список, курсор там не двигался.
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.active, Side::Left);
        assert_eq!(app.panel(Side::Left).current().unwrap().name, "main.rs");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Esc);
        assert!(app.viewer.is_none());
        assert_eq!(app.active, Side::Left);
    }

    #[test]
    fn search_typed_in_viewer() {
        let (_tmp, mut app) = viewer_dir();
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        wait_preview(&mut app);
        press(&mut app, KeyCode::Char('/'));
        for c in "x5".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let search = app.viewer.as_ref().unwrap().search.as_ref().unwrap();
        // x5 и x50..x59.
        assert_eq!(search.matches.len(), 11);
        press(&mut app, KeyCode::Right);
        assert_eq!(
            app.viewer
                .as_ref()
                .unwrap()
                .search
                .as_ref()
                .unwrap()
                .current,
            1
        );
        press(&mut app, KeyCode::Backspace);
        assert_eq!(
            app.viewer.as_ref().unwrap().search.as_ref().unwrap().query,
            "x"
        );
        // Esc закрывает только поиск, просмотр остаётся.
        press(&mut app, KeyCode::Esc);
        assert!(app.viewer.as_ref().unwrap().search.is_none());
        assert!(app.viewer.is_some());
    }

    #[test]
    fn hiding_second_panel_closes_viewer() {
        let (_tmp, mut app) = viewer_dir();
        press(&mut app, KeyCode::Char('p'));
        press(&mut app, KeyCode::Char('P'));
        assert!(app.viewer.is_none());
        assert_eq!(app.hidden, Some(Side::Right));
        // Просмотр снова показывает вторую панель.
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.hidden, None);
    }

    fn ctrl(app: &mut App, c: char) {
        app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    /// Открывает `main.rs` в редакторе.
    fn editing() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("main.rs"), "fn main() {\n}\n").unwrap();
        let mut app = app_in(tmp.path());
        press(&mut app, KeyCode::Enter);
        wait_preview(&mut app);
        press(&mut app, KeyCode::Char('e'));
        assert!(app.editor.is_some());
        (tmp, app)
    }

    #[test]
    fn edit_and_save_with_confirmation() {
        let (tmp, mut app) = editing();
        press(&mut app, KeyCode::End);
        press(&mut app, KeyCode::Enter);
        for c in "run();".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        // Клавиши ушли в редактор, а не в панели: `e`, `p`, `q` — просто буквы.
        assert_eq!(app.editor.as_ref().unwrap().lines[1], "    run();");

        ctrl(&mut app, 's');
        assert!(matches!(app.mode, Mode::EditorSave));
        // Отказ — файл не тронут.
        press(&mut app, KeyCode::Esc);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("main.rs")).unwrap(),
            "fn main() {\n}\n"
        );

        ctrl(&mut app, 's');
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("main.rs")).unwrap(),
            "fn main() {\n    run();\n}\n"
        );
        assert!(!app.editor.as_ref().unwrap().modified);
        assert!(app.status.as_ref().unwrap().text.contains("saved"));

        // Без изменений Esc закрывает сразу; просмотр перечитывает файл.
        press(&mut app, KeyCode::Esc);
        assert!(app.editor.is_none());
        assert!(app.viewer.is_some());
        wait_preview(&mut app);
    }

    #[test]
    fn esc_with_changes_asks_and_can_discard() {
        let (tmp, mut app) = editing();
        press(&mut app, KeyCode::Char('x'));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::EditorExit));
        // Esc в диалоге — назад к правке.
        press(&mut app, KeyCode::Esc);
        assert!(app.editor.is_some());
        assert!(matches!(app.mode, Mode::Normal));

        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('n'));
        assert!(app.editor.is_none());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("main.rs")).unwrap(),
            "fn main() {\n}\n"
        );
    }

    #[test]
    fn exit_dialog_can_save() {
        let (tmp, mut app) = editing();
        press(&mut app, KeyCode::Char('x'));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('y'));
        assert!(app.editor.is_none());
        assert!(
            std::fs::read_to_string(tmp.path().join("main.rs"))
                .unwrap()
                .starts_with("xfn")
        );
    }

    #[test]
    fn directories_cannot_be_edited() {
        let (_tmp, mut app) = viewer_dir();
        press(&mut app, KeyCode::Char('p')); // курсор на sub/
        press(&mut app, KeyCode::Tab);
        wait_preview(&mut app);
        press(&mut app, KeyCode::Char('e'));
        assert!(app.editor.is_none());
        assert!(app.status.as_ref().unwrap().error);
    }

    /// Дожидается окончания поиска в панели.
    pub(super) fn wait_finder(app: &mut App) {
        while let Some(rx) = app.finder.as_ref().and_then(Finder::rx) {
            let msg = rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("поиск должен закончиться");
            app.on_finder_msg(msg);
        }
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn search_tree() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("deep/er")).unwrap();
        std::fs::write(tmp.path().join("top.txt"), "nothing\nTODO top\n").unwrap();
        std::fs::write(tmp.path().join("deep/er/inner.rs"), "// TODO inner\n").unwrap();
        let app = app_in(tmp.path());
        (tmp, app)
    }

    #[test]
    fn content_search_opens_viewer_and_esc_restores() {
        let (_tmp, mut app) = search_tree();
        press(&mut app, KeyCode::Char('/'));
        // Просмотр открыт поиском во второй панели.
        assert!(app.viewer.is_some());
        type_text(&mut app, "todo");
        wait_finder(&mut app);
        let finder = app.finder.as_ref().unwrap();
        assert_eq!(finder.hits.len(), 2);
        assert_eq!(finder.query, "todo");
        // Буквы ушли в запрос, а не в команды панели (`d`, `o`).
        assert!(matches!(app.mode, Mode::Normal));

        wait_preview(&mut app);
        let viewer = app.viewer.as_ref().unwrap();
        assert!(viewer.path.as_ref().unwrap().ends_with("top.txt"));
        assert_eq!(viewer.search.as_ref().unwrap().current().unwrap().line, 1);

        press(&mut app, KeyCode::Down);
        assert!(
            app.viewer
                .as_ref()
                .unwrap()
                .path
                .as_ref()
                .unwrap()
                .ends_with("inner.rs")
        );

        press(&mut app, KeyCode::Esc);
        assert!(app.finder.is_none());
        assert!(
            app.viewer.is_none(),
            "просмотр, открытый поиском, закрывается"
        );
    }

    #[test]
    fn enter_goes_to_found_file() {
        let (tmp, mut app) = search_tree();
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "inner");
        wait_finder(&mut app);
        press(&mut app, KeyCode::Enter);
        assert!(app.finder.is_none());
        let panel = app.panel(Side::Left);
        assert_eq!(panel.dir, tmp.path().join("deep/er"));
        assert_eq!(panel.current().unwrap().name, "inner.rs");
        // Просмотр остаётся на найденном файле, без режима поиска.
        let viewer = app.viewer.as_ref().unwrap();
        assert!(viewer.search.is_none());
        assert!(viewer.path.as_ref().unwrap().ends_with("inner.rs"));
    }

    #[test]
    fn name_search_finds_directories() {
        let (tmp, mut app) = search_tree();
        press(&mut app, KeyCode::Char('f'));
        type_text(&mut app, "er");
        wait_finder(&mut app);
        let names: Vec<String> = app
            .finder
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .map(|h| h.text.clone())
            .collect();
        assert_eq!(names, ["er", "inner.rs"]);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.panel(Side::Left).dir, tmp.path().join("deep"));
        assert_eq!(app.panel(Side::Left).current().unwrap().name, "er");
    }

    #[test]
    fn search_without_second_panel_keeps_it_hidden() {
        let (_tmp, mut app) = search_tree();
        press(&mut app, KeyCode::Char('P'));
        press(&mut app, KeyCode::Char('/'));
        assert!(app.viewer.is_none());
        assert_eq!(app.hidden, Some(Side::Right));
        type_text(&mut app, "x");
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.finder.as_ref().unwrap().query, "");
        press(&mut app, KeyCode::Esc);
        assert!(app.finder.is_none());
    }
}
