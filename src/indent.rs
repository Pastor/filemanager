//! Размер и вид отступа для редактируемого файла.
//!
//! Порядок выбора, от самого точного к самому общему:
//! 1. `.editorconfig` — его написал автор проекта специально для этого;
//! 2. сам файл — уже имеющиеся отступы важнее любых умолчаний;
//! 3. принятое в языке — 4 пробела, 2 пробела или табуляция;
//! 4. значение из настроек.

use std::fs;
use std::path::{Path, PathBuf};

/// Отступ: табуляция или пробелы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indent {
    pub tabs: bool,
    /// Ширина одного уровня в колонках.
    pub size: usize,
}

impl Indent {
    pub const fn spaces(size: usize) -> Self {
        Self { tabs: false, size }
    }

    pub const fn tabs(size: usize) -> Self {
        Self { tabs: true, size }
    }

    /// Текст одного уровня отступа.
    pub fn unit(&self) -> String {
        if self.tabs {
            "\t".to_string()
        } else {
            " ".repeat(self.size)
        }
    }
}

/// Откуда взят отступ — показывается пользователю, чтобы выбор не был загадкой.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    EditorConfig,
    Detected,
    Language,
    Default,
}

/// Выбранный отступ и ширина табуляции для показа.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub indent: Indent,
    pub tab_width: usize,
    pub source: Source,
}

/// Выбирает отступ для файла `path` с содержимым `lines`.
pub fn choose(path: &Path, lines: &[String], language: Option<&str>, default: usize) -> Choice {
    let config = editorconfig(path);
    let tab_width = config.tab_width.or(config.size).unwrap_or(4);

    if let Some(tabs) = config.tabs {
        let size = if tabs {
            tab_width
        } else {
            config.size.unwrap_or(default)
        };
        return Choice {
            indent: Indent { tabs, size },
            tab_width,
            source: Source::EditorConfig,
        };
    }
    if let Some(indent) = detect(lines) {
        return Choice {
            indent: Indent {
                size: if indent.tabs { tab_width } else { indent.size },
                ..indent
            },
            tab_width,
            source: Source::Detected,
        };
    }
    if let Some(indent) = language.and_then(by_language) {
        return Choice {
            indent,
            tab_width: if indent.tabs { indent.size } else { tab_width },
            source: Source::Language,
        };
    }
    Choice {
        indent: Indent::spaces(default.max(1)),
        tab_width,
        source: Source::Default,
    }
}

/// Отступ, принятый в языке. Имена — как у [`crate::highlight::Language`].
pub fn by_language(name: &str) -> Option<Indent> {
    match name {
        "Go" | "Makefile" => Some(Indent::tabs(4)),
        "JavaScript/TypeScript"
        | "JSON"
        | "YAML"
        | "XML/HTML"
        | "CSS"
        | "Lua"
        | "Ruby"
        | "Markdown" => Some(Indent::spaces(2)),
        "Rust" | "C/C++" | "Java/Kotlin" | "Python" | "Swift" | "PHP" | "Shell" | "Perl"
        | "SQL" | "TOML" | "INI" | "Dockerfile" => Some(Indent::spaces(4)),
        _ => None,
    }
}

/// Угадывает отступ по строкам файла.
///
/// Табуляция побеждает, если строк с ней больше, чем строк с пробелами.
/// Для пробелов берётся самый частый прирост отступа между соседними
/// непустыми строками: абсолютные отступы врут (вложенность 3 при шаге 2 даёт
/// 6), а прирост на один уровень и есть искомый шаг.
pub fn detect(lines: &[String]) -> Option<Indent> {
    let mut tabs = 0;
    let mut spaced = 0;
    let mut deltas = [0usize; 9];
    let mut prev = 0;
    for line in lines.iter().take(10_000) {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('\t') {
            tabs += 1;
            continue;
        }
        let n = line.len() - line.trim_start_matches(' ').len();
        // Продолжение блочного комментария ` * ` сдвинуто на один пробел — шум.
        if line.trim_start().starts_with('*') {
            continue;
        }
        if n > 0 {
            spaced += 1;
        }
        if n > prev && n - prev <= 8 {
            deltas[n - prev] += 1;
        }
        prev = n;
    }
    if tabs == 0 && spaced == 0 {
        return None;
    }
    if tabs > spaced {
        return Some(Indent::tabs(4));
    }
    // При равенстве частот предпочитаем больший шаг: 4 при равном счёте с 2
    // чаще оказывается правдой (2 встречается как выравнивание продолжений).
    let best = (2..=8).max_by_key(|&d| (deltas[d], d))?;
    (deltas[best] > 0).then_some(Indent::spaces(best))
}

/// Настройки отступа из `.editorconfig`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub tabs: Option<bool>,
    pub size: Option<usize>,
    pub tab_width: Option<usize>,
}

/// Собирает настройки `.editorconfig` для файла: от корня к самому файлу,
/// ближние файлы и поздние секции перекрывают дальние и ранние.
pub fn editorconfig(path: &Path) -> Config {
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut dir = path.parent();
    while let Some(d) = dir {
        let candidate = d.join(".editorconfig");
        if let Ok(text) = fs::read_to_string(&candidate) {
            let root = is_root(&text);
            files.push((d.to_path_buf(), text));
            if root {
                break;
            }
        }
        dir = d.parent();
    }

    let mut out = Config::default();
    for (dir, text) in files.iter().rev() {
        let rel = path.strip_prefix(dir).unwrap_or(path);
        apply(&mut out, text, rel);
    }
    out
}

fn is_root(text: &str) -> bool {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            return false;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case("root")
        {
            return v.trim().eq_ignore_ascii_case("true");
        }
    }
    false
}

/// Применяет секции одного файла, подходящие к пути `rel`.
fn apply(out: &mut Config, text: &str, rel: &Path) {
    let rel = rel.to_string_lossy().replace('\\', "/");
    let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
    let mut active = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            // Шаблон без `/` относится к имени файла в любой директории.
            active = if section.contains('/') {
                glob(section.trim_start_matches('/'), &rel)
            } else {
                glob(section, &name)
            };
            continue;
        }
        if !active {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().to_ascii_lowercase();
        match key.trim().to_ascii_lowercase().as_str() {
            "indent_style" => match value.as_str() {
                "tab" => out.tabs = Some(true),
                "space" => out.tabs = Some(false),
                _ => {}
            },
            "indent_size" => {
                if value == "tab" {
                    out.size = out.tab_width;
                } else if let Ok(n) = value.parse::<usize>() {
                    out.size = Some(n.clamp(1, 16));
                }
            }
            "tab_width" => {
                if let Ok(n) = value.parse::<usize>() {
                    out.tab_width = Some(n.clamp(1, 16));
                }
            }
            _ => {}
        }
    }
}

/// Сопоставление с шаблоном EditorConfig: `*`, `**`, `?`, `[abc]`, `{a,b}`.
pub fn glob(pattern: &str, text: &str) -> bool {
    // Фигурные скобки раскрываются в варианты заранее.
    if let Some(open) = pattern.find('{')
        && let Some(close) = pattern[open..].find('}').map(|c| open + c)
    {
        let (head, tail) = (&pattern[..open], &pattern[close + 1..]);
        return pattern[open + 1..close]
            .split(',')
            .any(|alt| glob(&format!("{head}{alt}{tail}"), text));
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_at(&p, &t)
}

fn glob_at(p: &[char], t: &[char]) -> bool {
    match p.first() {
        None => t.is_empty(),
        Some('*') if p.get(1) == Some(&'*') => {
            let rest = &p[2..];
            (0..=t.len()).any(|i| glob_at(rest, &t[i..]))
        }
        Some('*') => {
            let rest = &p[1..];
            // Одна звёздочка не переходит границу директории.
            for i in 0..=t.len() {
                if glob_at(rest, &t[i..]) {
                    return true;
                }
                if i < t.len() && t[i] == '/' {
                    break;
                }
            }
            false
        }
        Some('?') => !t.is_empty() && t[0] != '/' && glob_at(&p[1..], &t[1..]),
        Some('[') => {
            let Some(close) = p.iter().position(|&c| c == ']') else {
                return !t.is_empty() && t[0] == '[' && glob_at(&p[1..], &t[1..]);
            };
            let set = &p[1..close];
            let (negate, set) = match set.first() {
                Some('!') => (true, &set[1..]),
                _ => (false, set),
            };
            !t.is_empty() && (set.contains(&t[0]) != negate) && glob_at(&p[close + 1..], &t[1..])
        }
        Some(&c) => !t.is_empty() && t[0] == c && glob_at(&p[1..], &t[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(String::from).collect()
    }

    #[test]
    fn detects_spaces_by_step() {
        let two = lines("a:\n  b:\n    c: 1\n  d: 2\ne:\n  f: 3\n");
        assert_eq!(detect(&two), Some(Indent::spaces(2)));
        let four = lines("fn x() {\n    if y {\n        z();\n    }\n}\n");
        assert_eq!(detect(&four), Some(Indent::spaces(4)));
    }

    #[test]
    fn detects_tabs_and_ignores_comment_stars() {
        let tabs = lines("func x() {\n\tif y {\n\t\tz()\n\t}\n}\n");
        assert_eq!(detect(&tabs), Some(Indent::tabs(4)));
        let doc = lines("/**\n * doc\n */\nint x() {\n    return 1;\n}\n");
        assert_eq!(detect(&doc), Some(Indent::spaces(4)));
        assert_eq!(detect(&lines("flat\ntext\n")), None);
    }

    #[test]
    fn glob_patterns() {
        assert!(glob("*", "main.rs"));
        assert!(glob("*.rs", "main.rs"));
        assert!(!glob("*.rs", "main.rsx"));
        assert!(glob("*.{js,ts}", "app.ts"));
        assert!(!glob("*.{js,ts}", "app.py"));
        assert!(glob("Makefile", "Makefile"));
        assert!(glob("src/**/*.rs", "src/a/b/c.rs"));
        assert!(!glob("src/*.rs", "src/a/c.rs"));
        assert!(glob("file?.[ch]", "file1.h"));
        assert!(!glob("file?.[!ch]", "file1.h"));
    }

    #[test]
    fn editorconfig_sections_and_hierarchy() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("web/src")).unwrap();
        fs::write(
            root.join(".editorconfig"),
            "root = true\n[*]\nindent_style = space\nindent_size = 4\n[Makefile]\nindent_style = tab\n",
        )
        .unwrap();
        fs::write(
            root.join("web/.editorconfig"),
            "[*.{js,ts}]\nindent_size = 2\n",
        )
        .unwrap();

        let rs = editorconfig(&root.join("main.rs"));
        assert_eq!(rs.tabs, Some(false));
        assert_eq!(rs.size, Some(4));
        // Ближний файл перекрывает размер, стиль наследуется от корневого.
        let ts = editorconfig(&root.join("web/src/app.ts"));
        assert_eq!((ts.tabs, ts.size), (Some(false), Some(2)));
        assert_eq!(editorconfig(&root.join("Makefile")).tabs, Some(true));
    }

    #[test]
    fn choose_respects_priority() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.js");
        // Файл с отступом 4, язык подсказывает 2 — побеждает файл.
        let body = lines("function f() {\n    return 1;\n}\n");
        let c = choose(&path, &body, Some("JavaScript/TypeScript"), 4);
        assert_eq!((c.indent, c.source), (Indent::spaces(4), Source::Detected));

        // Пустой файл — по языку.
        let c = choose(&path, &[], Some("JavaScript/TypeScript"), 4);
        assert_eq!((c.indent, c.source), (Indent::spaces(2), Source::Language));
        let c = choose(&tmp.path().join("main.go"), &[], Some("Go"), 4);
        assert!(c.indent.tabs);

        // Неизвестный язык — значение из настроек.
        let c = choose(&tmp.path().join("x.txt"), &[], None, 3);
        assert_eq!((c.indent, c.source), (Indent::spaces(3), Source::Default));

        // .editorconfig важнее файла.
        fs::write(
            tmp.path().join(".editorconfig"),
            "root=true\n[*.js]\nindent_style=tab\ntab_width=8\n",
        )
        .unwrap();
        let c = choose(&path, &body, Some("JavaScript/TypeScript"), 4);
        assert_eq!(c.source, Source::EditorConfig);
        assert_eq!(c.indent, Indent::tabs(8));
        assert_eq!(c.tab_width, 8);
    }
}
