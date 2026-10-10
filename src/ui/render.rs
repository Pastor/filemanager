//! Отрисовка: две панели, строка подсказок и диалоги поверх них.

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::Side;
use crate::i18n::Text;
use crate::listing::{Entry, Kind};
use crate::ops::Progress;
use crate::ui::job::{Job, JobKind};
use crate::ui::theme::Theme;
use crate::ui::{App, Mode};

/// Ширина колонки размера в однострочном режиме: `1023.9 K`.
const SIZE_WIDTH: usize = 8;
/// Ширина колонки даты: `2026-10-10 12:34`.
const DATE_WIDTH: usize = 16;
/// Меньше этого под имя не отдаём: без имени размер и дата бесполезны.
const NAME_MIN: usize = 12;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Block::default().style(app.theme.base()), area);

    let [main, hints] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

    match app.hidden {
        Some(hidden) => draw_panel(frame, main, app, hidden.other()),
        None => {
            let [left, right] = Layout::horizontal([
                Constraint::Percentage(app.left_pct),
                Constraint::Percentage(100 - app.left_pct),
            ])
            .areas(main);
            draw_panel(frame, left, app, Side::Left);
            draw_panel(frame, right, app, Side::Right);
        }
    }

    draw_hints(frame, hints, app);

    match &app.mode {
        Mode::Normal => {}
        Mode::ConfirmDelete { permanent, targets } => {
            draw_confirm(frame, main, *permanent, targets, &app.theme, app.text());
        }
        Mode::Job(job) => draw_job(frame, main, job, &app.theme, app.text()),
    }
}

fn draw_panel(frame: &mut Frame, area: Rect, app: &mut App, side: Side) {
    let active = app.active == side;
    let theme = app.theme;
    let text = app.text();

    let title_width = (area.width as usize).saturating_sub(4);
    let title = shorten_path(&display_path(&app.panel(side).dir), title_width);
    let panel = app.panel(side);
    let counter = if panel.entries.is_empty() {
        String::new()
    } else {
        format!(" {}/{} ", panel.cursor + 1, panel.entries.len())
    };

    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border(active))
        .title(Line::styled(format!(" {title} "), theme.title(active)))
        .title_bottom(Line::styled(counter, theme.dim()).right_aligned());
    if !panel.marked.is_empty() {
        let marked = format!(
            " {}: {} · {} ",
            text.marked,
            panel.marked.len(),
            format_size(panel.marked_size())
        );
        block = block.title_bottom(Line::styled(marked, theme.marked()).left_aligned());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let panel = app.panel_mut(side);
    if panel.entries.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(format!(" {}", text.empty), theme.dim())),
            inner,
        );
        return;
    }

    let rows = inner.height as usize;
    panel.fit(rows);
    let panel = app.panel(side);
    let columns = panel.columns as usize;

    // Колонки равной ширины, разделённые вертикальной чертой.
    let mut constraints = Vec::with_capacity(columns * 2);
    for i in 0..columns {
        if i > 0 {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Fill(1));
    }
    let cells = Layout::horizontal(constraints).split(inner);

    for col in 0..columns {
        let rect = cells[col * 2];
        if col > 0 {
            let sep = cells[col * 2 - 1];
            let lines = vec![Line::styled("│", theme.border(false)); sep.height as usize];
            frame.render_widget(Paragraph::new(lines), sep);
        }
        let start = panel.offset + col * rows;
        let lines: Vec<Line> = panel
            .entries
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .map(|(i, entry)| {
                let cursor = (i == panel.cursor).then_some(active);
                let look = Look {
                    details: columns == 1,
                    cursor,
                    marked: panel.is_marked(entry),
                };
                entry_line(entry, rect.width as usize, look, &theme, text)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), rect);
    }
}

/// Как показать элемент.
#[derive(Debug, Clone, Copy)]
struct Look {
    /// Режим одной колонки: справа размер и дата, если хватает места.
    details: bool,
    /// `Some(активна ли панель)`, если на элементе стоит курсор.
    cursor: Option<bool>,
    marked: bool,
}

/// Строка одного элемента шириной ровно `width`.
fn entry_line<'a>(entry: &Entry, width: usize, look: Look, theme: &Theme, text: &Text) -> Line<'a> {
    let Look {
        details,
        cursor,
        marked,
    } = look;
    // Первая позиция — отметка выделения.
    let prefix = if marked { "*" } else { " " };
    let avail = width.saturating_sub(prefix.len());

    let mut right = String::new();
    if details {
        let size = size_label(entry, text);
        let date = entry.modified.map(format_time).unwrap_or_default();
        // Справа: пробел, размер, пробел, дата, пробел.
        let with_date = NAME_MIN + SIZE_WIDTH + DATE_WIDTH + 3;
        let with_size = NAME_MIN + SIZE_WIDTH + 2;
        if avail >= with_date {
            right = format!(" {size:>SIZE_WIDTH$} {date:<DATE_WIDTH$} ");
        } else if avail >= with_size {
            right = format!(" {size:>SIZE_WIDTH$} ");
        }
    }

    let name_width = avail.saturating_sub(right.width());
    let suffix = if entry.kind.is_dir() { "/" } else { "" };
    // В подробном режиме у ссылки видно, куда она ведёт; цель обрезается
    // первой, имя — только если не влезает даже оно само.
    let name = match (&entry.target, details && entry.kind.is_link()) {
        (Some(target), true) if entry.name.width() + suffix.width() < name_width => {
            let head = format!("{}{suffix}", entry.name);
            let tail = fit(
                &format!(" → {}", target.display()),
                "",
                name_width - head.width(),
            );
            format!("{head}{tail}")
        }
        _ => fit(&entry.name, suffix, name_width),
    };
    let pad = name_width.saturating_sub(name.width());

    // Выделение перекрашивает строку целиком: его должно быть видно сразу.
    let (name_style, right_style) = if marked {
        (theme.marked(), theme.marked())
    } else {
        (name_style(entry, theme), theme.dim())
    };
    let mut spans = vec![
        Span::styled(prefix, theme.marked()),
        Span::styled(name, name_style),
        Span::raw(" ".repeat(pad)),
    ];
    if !right.is_empty() {
        spans.push(Span::styled(right, right_style));
    }

    let line = Line::from(spans);
    match cursor {
        Some(active) => line.patch_style(theme.cursor(active)),
        None => line,
    }
}

/// Стиль имени по виду элемента.
fn name_style(entry: &Entry, theme: &Theme) -> Style {
    match entry.kind {
        Kind::Dir => theme.dir(),
        Kind::LinkDir => theme.link().patch(theme.dir()),
        Kind::LinkFile => theme.link(),
        Kind::LinkBroken => theme.broken(),
        Kind::File if entry.exec => theme.exec(),
        Kind::File | Kind::Special => Style::default(),
    }
}

/// Подпись в колонке размера.
fn size_label(entry: &Entry, text: &Text) -> String {
    match entry.kind {
        Kind::Dir => text.dir_label.to_string(),
        Kind::LinkDir => text.link_dir_label.to_string(),
        _ => format_size(entry.size),
    }
}

/// Компактный размер в двоичных единицах: `512 B`, `1.5 K`, `23.0 M`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn format_time(t: std::time::SystemTime) -> String {
    DateTime::<Local>::from(t)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// Вписывает имя с суффиксом в `width` колонок терминала. Длинное имя
/// обрезается многоточием, суффикс (`/` директории) сохраняется всегда.
pub fn fit(name: &str, suffix: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if name.width() + suffix.width() <= width {
        return format!("{name}{suffix}");
    }
    let budget = width.saturating_sub(suffix.width() + 1);
    let mut out = String::new();
    let mut used = 0;
    for ch in name.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out.push_str(suffix);
    // Суффикс мог не уместиться даже сам по себе в совсем узкой колонке.
    truncate_width(&out, width)
}

/// Отрезает строку по ширине без многоточия.
fn truncate_width(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// Путь для заголовка: домашняя директория заменяется на `~`.
fn display_path(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.display())
        };
    }
    path.display().to_string()
}

/// Укорачивает путь слева: конец пути важнее начала.
pub fn shorten_path(path: &str, width: usize) -> String {
    if path.width() <= width {
        return path.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut used = 1; // многоточие
    for ch in path.chars().rev() {
        let w = ch.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        tail.push(ch);
        used += w;
    }
    let mut out = String::from("…");
    out.extend(tail.into_iter().rev());
    out
}

fn draw_hints(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme;
    if let Some(status) = &app.status {
        let style = if status.error {
            theme.error()
        } else {
            Style::default()
        };
        let text = truncate_width(&format!(" {}", status.text), area.width as usize);
        frame.render_widget(Paragraph::new(Line::styled(text, style)), area);
        return;
    }

    let hints = mode_hints(app);
    let line = hint_line(&hints, (area.width as usize).saturating_sub(1), &theme);
    frame.render_widget(Paragraph::new(line), area.inner(Margin::new(1, 0)));
}

/// Клавиши текущего режима для строки подсказок.
fn mode_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    let t = app.text();
    match &app.mode {
        Mode::Normal => vec![
            ("↑↓", t.hint_move),
            ("→←", t.hint_in_out),
            ("Tab", t.hint_panel),
            ("Space", t.hint_mark),
            ("c", t.hint_copy),
            ("m", t.hint_move_to),
            ("d", t.hint_delete),
            ("P", t.hint_hide),
            ("<>", t.hint_width),
            ("1-3", t.hint_columns),
            (".", t.hint_hidden),
            ("q", t.hint_quit),
        ],
        Mode::ConfirmDelete { permanent, .. } => vec![
            (if *permanent { "D" } else { "d" }, t.confirm),
            ("Esc", t.cancel),
        ],
        Mode::Job(job) if job.conflict.is_some() => conflict_hints(t).concat(),
        Mode::Job(_) => vec![("Esc", t.cancel)],
    }
}

/// Ответы на конфликт имён двумя строками: частые и «для всех».
fn conflict_hints(t: &'static Text) -> [Vec<(&'static str, &'static str)>; 2] {
    [
        vec![("r", t.replace), ("s", t.skip), ("Esc", t.cancel)],
        vec![("R", t.replace_all), ("S", t.skip_all)],
    ]
}

/// Наибольшая ширина диалога: шире длинные пути читать неудобно.
const DIALOG_MAX_WIDTH: u16 = 72;

/// Рисует диалог по центру `area`. Ширина подбирается под содержимое.
fn dialog(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    body: Vec<Line>,
    border: Style,
    theme: &Theme,
) {
    // Рамка и поля по одному символу с каждой стороны.
    let content = body.iter().map(Line::width).max().unwrap_or(0) as u16;
    let width = (content + 4)
        .max(title.width() as u16 + 6)
        .clamp(30, DIALOG_MAX_WIDTH)
        .min(area.width);
    let height = (body.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::styled(format!(" {title} "), border))
        .padding(Padding::horizontal(1))
        .style(theme.base());
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(body).block(block), rect);
}

/// Ширина текста внутри диалога наибольшей ширины.
fn dialog_inner(area: Rect) -> usize {
    (DIALOG_MAX_WIDTH.min(area.width) as usize).saturating_sub(4)
}

fn draw_confirm(
    frame: &mut Frame,
    area: Rect,
    permanent: bool,
    targets: &[std::path::PathBuf],
    theme: &Theme,
    t: &Text,
) {
    let inner = dialog_inner(area);
    let (title, question, border) = if permanent {
        (t.delete_forever_title, t.delete_forever, theme.error())
    } else {
        (t.delete_title, t.delete_to_trash, theme.border(true))
    };
    let what = match targets {
        [one] => shorten_path(&display_path(one), inner),
        many => format!("{}: {}", t.items, many.len()),
    };
    let key = if permanent { "D" } else { "d" };
    let body = vec![
        Line::raw(question),
        Line::raw(""),
        Line::styled(
            what,
            Style::default().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Line::raw(""),
        hint_line(&[(key, t.confirm), ("Esc", t.cancel)], inner, theme),
    ];
    dialog(frame, area, title, body, border, theme);
}

fn draw_job(frame: &mut Frame, area: Rect, job: &Job, theme: &Theme, t: &'static Text) {
    let inner = dialog_inner(area);
    let title = match job.kind {
        JobKind::Copy => t.job_copy,
        JobKind::Move => t.job_move,
        JobKind::Trash => t.job_trash,
        JobKind::Delete => t.job_delete,
    };
    let p = &job.progress;
    let current = shorten_path(&display_path(&p.current), inner);
    let footer = if job.cancelling {
        Line::styled(t.cancelling, theme.dim())
    } else {
        hint_line(&[("Esc", t.cancel)], inner, theme)
    };
    // Вопрос о конфликте заменяет окно прогресса: два окна друг на друге
    // читаются хуже, а прогресс, пока операция ждёт ответа, всё равно стоит.
    if let Some((path, _)) = &job.conflict {
        let mut body = vec![
            Line::styled(
                shorten_path(&display_path(path), inner),
                Style::default().add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Line::raw(""),
        ];
        for row in conflict_hints(t) {
            body.push(hint_line(&row, inner, theme));
        }
        dialog(frame, area, t.conflict_exists, body, theme.error(), theme);
        return;
    }
    let body = vec![
        Line::styled(current, theme.dim()),
        gauge(p, inner, theme),
        Line::styled(counters(p), theme.dim()),
        Line::raw(""),
        footer,
    ];
    dialog(frame, area, title, body, theme.border(true), theme);
}

/// Полоса прогресса с процентом. Доля считается по байтам, а если байтов
/// нет (пустые файлы, удаление) — по числу элементов.
fn gauge<'a>(p: &Progress, width: usize, theme: &Theme) -> Line<'a> {
    let ratio = if p.bytes_total > 0 {
        p.bytes_done as f64 / p.bytes_total as f64
    } else if p.items_total > 0 {
        p.items_done as f64 / p.items_total as f64
    } else {
        0.0
    };
    let ratio = ratio.clamp(0.0, 1.0);
    let bar = width.saturating_sub(5);
    let filled = ((bar as f64) * ratio).round() as usize;
    Line::from(vec![
        Span::styled("█".repeat(filled), theme.border(true)),
        Span::styled("░".repeat(bar - filled), theme.dim()),
        Span::raw(format!(" {:>3}%", (ratio * 100.0).round() as u32)),
    ])
}

/// Счётчики операции: «12/40 · 1.5 M/20.0 M».
fn counters(p: &Progress) -> String {
    let mut out = format!("{}/{}", p.items_done, p.items_total);
    if p.bytes_total > 0 {
        out.push_str(&format!(
            " · {}/{}",
            format_size(p.bytes_done),
            format_size(p.bytes_total)
        ));
    }
    out
}

/// Собирает строку подсказок; то, что не влезает, заменяется многоточием.
fn hint_line<'a>(hints: &[(&'a str, &'a str)], width: usize, theme: &Theme) -> Line<'a> {
    let mut spans = Vec::new();
    let mut used = 0;
    for (i, (key, desc)) in hints.iter().enumerate() {
        let item = key.width() + 1 + desc.width() + 2;
        // Последний элемент не обязан оставлять место под многоточие.
        let reserve = if i + 1 == hints.len() { 0 } else { 1 };
        if used + item + reserve > width {
            if used < width {
                spans.push(Span::styled("…", theme.dim()));
            }
            break;
        }
        spans.push(Span::styled(*key, theme.key()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(*desc, theme.dim()));
        spans.push(Span::raw("  "));
        used += item;
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_short_names() {
        assert_eq!(fit("src", "/", 10), "src/");
        assert_eq!(fit("main.rs", "", 7), "main.rs");
    }

    #[test]
    fn fit_truncates_with_ellipsis_and_keeps_suffix() {
        assert_eq!(fit("verylongname", "/", 8), "verylo…/");
        assert_eq!(fit("verylongname", "", 5), "very…");
        assert_eq!(fit("abc", "/", 1), "…");
        assert_eq!(fit("abc", "", 0), "");
    }

    #[test]
    fn fit_respects_wide_characters() {
        // Иероглифы занимают по две колонки.
        let s = fit("日本語のファイル", "", 7);
        assert!(s.width() <= 7, "{s}");
        assert!(s.ends_with('…'));
    }

    #[test]
    fn shorten_path_keeps_the_tail() {
        assert_eq!(shorten_path("/a/b", 10), "/a/b");
        assert_eq!(shorten_path("/home/user/projects/fm", 10), "…ojects/fm");
        assert_eq!(shorten_path("/x", 0), "");
    }

    #[test]
    fn format_size_is_compact() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 K");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 G");
        // Самое длинное значение умещается в колонку размера.
        assert!(format_size(1024 * 1024 - 1).len() <= SIZE_WIDTH);
    }

    #[test]
    fn entry_line_has_exact_width() {
        let theme = Theme {
            depth: crate::ui::theme::ColorDepth::True,
            name: crate::ui::theme::ThemeName::Dark,
        };
        let entry = Entry {
            name: "Cargo.toml".into(),
            path: "/x/Cargo.toml".into(),
            kind: Kind::File,
            exec: false,
            size: 2048,
            modified: None,
            target: None,
        };
        for width in [0, 5, 20, 40, 80] {
            for details in [false, true] {
                let look = Look {
                    details,
                    cursor: Some(true),
                    marked: details,
                };
                let line = entry_line(&entry, width, look, &theme, crate::i18n::Lang::En.text());
                assert_eq!(
                    line.width(),
                    width.max(1),
                    "width {width}, details {details}"
                );
            }
        }
    }

    #[test]
    fn hint_line_fits_width() {
        let theme = Theme {
            depth: crate::ui::theme::ColorDepth::True,
            name: crate::ui::theme::ThemeName::Dark,
        };
        let hints = [("a", "alpha"), ("b", "beta"), ("c", "gamma")];
        for width in 0..40 {
            assert!(hint_line(&hints, width, &theme).width() <= width);
        }
    }

    /// Текст кадра построчно — для проверок содержимого экрана.
    fn screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn sample_dir() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        for i in 0..30 {
            std::fs::write(tmp.path().join(format!("file{i:02}.txt")), "").unwrap();
        }
        tmp
    }

    #[test]
    fn frame_shows_both_panels_and_hints() {
        let tmp = sample_dir();
        let mut app = App::new_for_test(tmp.path());
        let lines = screen(&mut app, 100, 12);
        println!("{}", lines.join("\n"));
        assert!(lines[1].matches("src/").count() == 2, "{}", lines[1]);
        assert!(lines[1].contains("<DIR>"));
        assert!(lines[11].contains("move"));
        assert!(lines[11].contains("copy"));
        // Счётчик позиции в нижней рамке.
        assert!(lines[10].contains("1/32"));
    }

    #[test]
    fn frame_with_columns_and_hidden_panel() {
        let tmp = sample_dir();
        let mut app = App::new_for_test(tmp.path());
        app.panel_mut(Side::Left).set_columns(3);
        app.hidden = Some(Side::Right);
        // Курсор за пределами первой страницы: 3 колонки по 8 строк.
        app.panel_mut(Side::Left).cursor = 30;
        let lines = screen(&mut app, 90, 11);
        println!("{}", lines.join("\n"));
        assert_eq!(lines[1].matches('│').count(), 4, "{}", lines[1]);
        assert!(lines.iter().any(|l| l.contains("file29.txt")));
        assert!(!lines.iter().any(|l| l.contains("Cargo.toml")));
    }

    #[test]
    fn frame_marks_and_delete_dialog() {
        let tmp = sample_dir();
        let mut app = App::new_for_test(tmp.path());
        let targets = app.panel(Side::Left).targets();
        app.panel_mut(Side::Left).toggle_mark();
        app.panel_mut(Side::Left).toggle_mark();
        app.mode = Mode::ConfirmDelete {
            permanent: true,
            targets,
        };
        let lines = screen(&mut app, 100, 16);
        println!("{}", lines.join("\n"));
        let all = lines.join("\n");
        assert!(all.contains("Delete permanently"));
        assert!(all.contains("cannot be undone"));
        assert!(lines[2].starts_with("│*Cargo.toml"), "{}", lines[2]);
        assert!(lines[15].contains("D confirm"));
        // Счётчик выделенного в нижней рамке: src/ и Cargo.toml (9 байт).
        assert!(lines[14].contains("marked: 2 · 9 B"), "{}", lines[14]);
    }

    #[test]
    fn frame_job_with_conflict() {
        let tmp = sample_dir();
        let mut app = App::new_for_test(tmp.path());
        let mut job = Job::delete(Vec::new(), false, Side::Left);
        job.progress = Progress {
            bytes_total: 200,
            bytes_done: 50,
            items_total: 4,
            items_done: 1,
            current: tmp.path().join("Cargo.toml"),
        };
        let (reply, _answers) = crossbeam_channel::bounded(1);
        job.conflict = Some((tmp.path().join("src"), reply));
        app.mode = Mode::Job(job);
        let lines = screen(&mut app, 100, 16);
        println!("{}", lines.join("\n"));
        let all = lines.join("\n");
        assert!(all.contains("Already exists"));
        assert!(all.contains("R replace all"));
        assert!(lines[15].contains("r replace"));

        // Без вопроса виден прогресс: доля по байтам и счётчики.
        if let Mode::Job(job) = &mut app.mode {
            job.conflict = None;
        }
        let lines = screen(&mut app, 100, 16);
        println!("{}", lines.join("\n"));
        let all = lines.join("\n");
        assert!(all.contains("Deleting"));
        assert!(all.contains(" 25%"));
        assert!(all.contains("1/4 · 50 B/200 B"));
    }
}
