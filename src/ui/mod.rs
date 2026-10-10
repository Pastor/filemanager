//! Состояние приложения, цикл событий и обработка клавиш.

mod render;
pub mod theme;

use std::path::PathBuf;

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, select, unbounded};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::config::{Config, LEFT_PCT_STEP, Side, clamp_left_pct};
use crate::i18n::Text;
use crate::panel::Panel;
use crate::ui::theme::Theme;

/// Сообщение в строке состояния. Живёт до следующего нажатия клавиши.
pub struct Status {
    pub text: String,
    pub error: bool,
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
            self.quit = true;
            return;
        }
        // Сочетания с Ctrl и Alt пока не используются; Shift входит в сам символ.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return;
        }
        self.status = None;

        let show_hidden = self.cfg.show_hidden;
        match key.code {
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
            KeyCode::Char('P') => {
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
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
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

fn main_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let events = spawn_events();
    let mut dirty = true;
    loop {
        if dirty {
            terminal.draw(|frame| render::draw(frame, app))?;
            dirty = false;
        }
        if app.quit {
            return Ok(());
        }
        select! {
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
}
