//! Лёгкая подсветка синтаксиса для панели просмотра.
//!
//! Полноценный движок с грамматиками здесь избыточен: глазу достаточно
//! отличать комментарии, строки, числа и ключевые слова. Поэтому — небольшой
//! построчный лексер, которому хватает одного флага состояния (незакрытая
//! конструкция с прошлой строки). Основа взята из `flob` и дополнена логами,
//! ignore-файлами, `.properties` и `.env`.
//!
//! Модуль занимается разбором, но не оформлением: цвет каждому роду фрагмента
//! назначает `ui::theme`, где он согласован с остальной палитрой и умеет
//! деградировать под возможности терминала.

use std::path::Path;

/// Род фрагмента текста.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// Обычный текст: показывается цветом терминала по умолчанию.
    Plain,
    Comment,
    /// Строковый или символьный литерал.
    Str,
    Number,
    Keyword,
    /// Тип, класс, встроенное имя.
    Type,
    /// Имя поля, ключ, заголовок, тег.
    Key,
    /// Добавленная строка в патче.
    Added,
    /// Удалённая строка в патче.
    Removed,
    /// Уровень ERROR/FATAL в логе.
    Error,
    /// Уровень WARN в логе.
    Warn,
}

/// Фрагмент строки, окрашиваемый целиком.
#[derive(Debug, Clone)]
pub struct Piece {
    pub kind: Token,
    pub text: String,
}

/// Распознанный язык.
pub struct Language {
    /// Имя для показа в свойствах файла.
    pub name: &'static str,
    grammar: Grammar,
}

/// Способ разбора: у большинства языков он общий и настраивается правилами,
/// а несколько форматов проще разобрать построчно по своим приметам.
enum Grammar {
    /// Обычный язык программирования или конфигурации.
    Code(&'static Rules),
    Markdown,
    /// Вывод `diff`/`patch`.
    Diff,
    /// XML, HTML, SVG.
    Markup,
    /// Журналы: время, уровни, адреса, трассировки стека.
    Log,
    /// `.gitignore` и родня: шаблоны путей.
    Ignore,
    /// Java `.properties`: `ключ = значение` или `ключ: значение`.
    Properties,
}

/// Как в языке размечается ключ перед значением.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keys {
    /// Ключей нет.
    None,
    /// `ключ: значение` — JSON, YAML.
    Colon,
    /// `ключ = значение` — TOML, INI.
    Equals,
}

/// Настройки разбора одного языка.
struct Rules {
    /// Начала однострочных комментариев.
    line_comment: &'static [&'static str],
    /// Ограничители блочного комментария.
    block_comment: Option<(&'static str, &'static str)>,
    /// Кавычки строковых литералов.
    quotes: &'static [char],
    /// Тройные кавычки открывают многострочную строку (Python).
    triple: bool,
    keywords: &'static [&'static str],
    /// Встроенные типы и константы.
    builtins: &'static [&'static str],
    /// Имя с заглавной буквы — тип. Верно для Rust, Go, Java, Swift и родни,
    /// но не для языков, где так пишут обычные переменные.
    capital_is_type: bool,
    keys: Keys,
    /// Строка вида `[секция]` размечается целиком как ключ (TOML, INI).
    sections: bool,
}

/// Общая заготовка: дальше остаётся переопределить отличия.
const BASE: Rules = Rules {
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '\''],
    triple: false,
    keywords: &[],
    builtins: &[],
    capital_is_type: false,
    keys: Keys::None,
    sections: false,
};

// ── правила отдельных языков ─────────────────────────────────────────────────

const RUST_RULES: Rules = Rules {
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
        "true", "type", "union", "unsafe", "use", "where", "while", "yield",
    ],
    builtins: &[
        "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8",
        "u16", "u32", "u64", "u128", "usize", "String", "Vec", "Option", "Result", "Box", "Some",
        "None", "Ok", "Err",
    ],
    capital_is_type: true,
    ..BASE
};

const C_RULES: Rules = Rules {
    keywords: &[
        "auto",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "constexpr",
        "continue",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "extern",
        "false",
        "for",
        "goto",
        "if",
        "inline",
        "namespace",
        "new",
        "nullptr",
        "operator",
        "private",
        "protected",
        "public",
        "register",
        "return",
        "sizeof",
        "static",
        "struct",
        "switch",
        "template",
        "this",
        "throw",
        "true",
        "try",
        "typedef",
        "typename",
        "union",
        "using",
        "virtual",
        "volatile",
        "while",
    ],
    builtins: &[
        "bool", "char", "double", "float", "int", "long", "short", "signed", "size_t", "unsigned",
        "void", "wchar_t", "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t",
        "uint32_t", "uint64_t", "NULL",
    ],
    ..BASE
};

const GO_RULES: Rules = Rules {
    quotes: &['"', '\'', '`'],
    keywords: &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "type",
        "var",
    ],
    builtins: &[
        "bool",
        "byte",
        "complex64",
        "complex128",
        "error",
        "float32",
        "float64",
        "int",
        "int8",
        "int16",
        "int32",
        "int64",
        "rune",
        "string",
        "uint",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "uintptr",
        "nil",
        "true",
        "false",
        "iota",
        "make",
        "new",
        "len",
        "cap",
        "append",
    ],
    capital_is_type: true,
    ..BASE
};

const JVM_RULES: Rules = Rules {
    keywords: &[
        "abstract",
        "as",
        "break",
        "case",
        "catch",
        "class",
        "companion",
        "const",
        "continue",
        "data",
        "def",
        "default",
        "do",
        "else",
        "enum",
        "extends",
        "final",
        "finally",
        "for",
        "fun",
        "if",
        "implements",
        "import",
        "in",
        "interface",
        "internal",
        "is",
        "lateinit",
        "match",
        "new",
        "object",
        "open",
        "override",
        "package",
        "private",
        "protected",
        "public",
        "return",
        "sealed",
        "static",
        "super",
        "suspend",
        "switch",
        "this",
        "throw",
        "throws",
        "trait",
        "try",
        "val",
        "var",
        "when",
        "while",
        "yield",
    ],
    builtins: &[
        "boolean", "byte", "char", "double", "float", "int", "long", "short", "void", "Int",
        "Long", "Double", "Float", "Boolean", "String", "Unit", "Any", "null", "true", "false",
    ],
    capital_is_type: true,
    ..BASE
};

const JS_RULES: Rules = Rules {
    quotes: &['"', '\'', '`'],
    keywords: &[
        "as",
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "finally",
        "for",
        "from",
        "function",
        "if",
        "implements",
        "import",
        "in",
        "instanceof",
        "interface",
        "let",
        "new",
        "of",
        "private",
        "protected",
        "public",
        "readonly",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "try",
        "type",
        "typeof",
        "var",
        "void",
        "while",
        "yield",
    ],
    builtins: &[
        "any",
        "boolean",
        "never",
        "number",
        "string",
        "unknown",
        "null",
        "undefined",
        "true",
        "false",
        "NaN",
        "Infinity",
        "console",
        "document",
        "window",
        "Promise",
        "Array",
        "Object",
    ],
    capital_is_type: true,
    ..BASE
};

const SWIFT_RULES: Rules = Rules {
    quotes: &['"'],
    keywords: &[
        "as",
        "associatedtype",
        "break",
        "case",
        "catch",
        "class",
        "continue",
        "default",
        "defer",
        "deinit",
        "do",
        "else",
        "enum",
        "extension",
        "fallthrough",
        "false",
        "for",
        "func",
        "guard",
        "if",
        "import",
        "in",
        "init",
        "inout",
        "internal",
        "is",
        "let",
        "nil",
        "open",
        "operator",
        "private",
        "protocol",
        "public",
        "repeat",
        "return",
        "self",
        "static",
        "struct",
        "subscript",
        "super",
        "switch",
        "throw",
        "throws",
        "true",
        "try",
        "typealias",
        "var",
        "where",
        "while",
    ],
    builtins: &[
        "Bool",
        "Double",
        "Float",
        "Int",
        "String",
        "Array",
        "Dictionary",
        "Optional",
    ],
    capital_is_type: true,
    ..BASE
};

const PYTHON_RULES: Rules = Rules {
    line_comment: &["#"],
    block_comment: None,
    triple: true,
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
        "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
        "yield",
    ],
    builtins: &[
        "True", "False", "None", "self", "bool", "bytes", "dict", "float", "int", "list", "print",
        "range", "set", "str", "tuple", "len", "open", "super",
    ],
    ..BASE
};

const RUBY_RULES: Rules = Rules {
    line_comment: &["#"],
    block_comment: None,
    keywords: &[
        "alias",
        "and",
        "begin",
        "break",
        "case",
        "class",
        "def",
        "defined?",
        "do",
        "else",
        "elsif",
        "end",
        "ensure",
        "false",
        "for",
        "if",
        "in",
        "module",
        "next",
        "nil",
        "not",
        "or",
        "redo",
        "rescue",
        "retry",
        "return",
        "self",
        "super",
        "then",
        "true",
        "unless",
        "until",
        "when",
        "while",
        "yield",
        "require",
        "attr_accessor",
    ],
    capital_is_type: true,
    ..BASE
};

const PHP_RULES: Rules = Rules {
    line_comment: &["//", "#"],
    keywords: &[
        "abstract",
        "and",
        "array",
        "as",
        "break",
        "callable",
        "case",
        "catch",
        "class",
        "clone",
        "const",
        "continue",
        "declare",
        "default",
        "do",
        "echo",
        "else",
        "elseif",
        "empty",
        "endif",
        "extends",
        "final",
        "finally",
        "fn",
        "for",
        "foreach",
        "function",
        "global",
        "if",
        "implements",
        "include",
        "instanceof",
        "interface",
        "isset",
        "list",
        "namespace",
        "new",
        "or",
        "print",
        "private",
        "protected",
        "public",
        "require",
        "return",
        "static",
        "switch",
        "throw",
        "trait",
        "try",
        "unset",
        "use",
        "var",
        "while",
        "yield",
    ],
    builtins: &[
        "true", "false", "null", "bool", "int", "float", "string", "void", "self",
    ],
    ..BASE
};

const SHELL_RULES: Rules = Rules {
    line_comment: &["#"],
    block_comment: None,
    keywords: &[
        "case", "do", "done", "elif", "else", "esac", "exit", "export", "fi", "for", "function",
        "if", "in", "local", "read", "readonly", "return", "select", "shift", "then", "trap",
        "unset", "until", "while",
    ],
    builtins: &[
        "cd", "echo", "eval", "exec", "printf", "pwd", "set", "source", "test", "true", "false",
    ],
    ..BASE
};

const PERL_RULES: Rules = Rules {
    line_comment: &["#"],
    block_comment: None,
    keywords: &[
        "and", "cmp", "do", "else", "elsif", "eq", "for", "foreach", "if", "last", "local", "my",
        "ne", "next", "not", "or", "our", "package", "redo", "require", "return", "sub", "unless",
        "until", "use", "while",
    ],
    ..BASE
};

const LUA_RULES: Rules = Rules {
    line_comment: &["--"],
    block_comment: Some(("--[[", "]]")),
    keywords: &[
        "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if",
        "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
    ],
    builtins: &[
        "ipairs", "pairs", "print", "require", "self", "table", "tonumber", "tostring",
    ],
    ..BASE
};

const SQL_RULES: Rules = Rules {
    line_comment: &["--"],
    quotes: &['\'', '"', '`'],
    keywords: &[
        "ALTER",
        "AND",
        "AS",
        "ASC",
        "BEGIN",
        "BETWEEN",
        "BY",
        "CASE",
        "COMMIT",
        "CREATE",
        "DEFAULT",
        "DELETE",
        "DESC",
        "DISTINCT",
        "DROP",
        "ELSE",
        "END",
        "EXISTS",
        "FROM",
        "GROUP",
        "HAVING",
        "IN",
        "INDEX",
        "INNER",
        "INSERT",
        "INTO",
        "IS",
        "JOIN",
        "LEFT",
        "LIKE",
        "LIMIT",
        "NOT",
        "NULL",
        "OFFSET",
        "ON",
        "OR",
        "ORDER",
        "OUTER",
        "PRIMARY",
        "REFERENCES",
        "RIGHT",
        "ROLLBACK",
        "SELECT",
        "SET",
        "TABLE",
        "THEN",
        "UNION",
        "UNIQUE",
        "UPDATE",
        "VALUES",
        "VIEW",
        "WHEN",
        "WHERE",
        "WITH",
    ],
    builtins: &[
        "BIGINT",
        "BLOB",
        "BOOLEAN",
        "CHAR",
        "DATE",
        "DECIMAL",
        "FLOAT",
        "INTEGER",
        "REAL",
        "TEXT",
        "TIMESTAMP",
        "VARCHAR",
    ],
    ..BASE
};

const CSS_RULES: Rules = Rules {
    line_comment: &["//"],
    keys: Keys::Colon,
    ..BASE
};

const JSON_RULES: Rules = Rules {
    line_comment: &[],
    block_comment: None,
    quotes: &['"'],
    keywords: &["true", "false", "null"],
    keys: Keys::Colon,
    ..BASE
};

const YAML_RULES: Rules = Rules {
    line_comment: &["#"],
    block_comment: None,
    keywords: &["true", "false", "null", "yes", "no", "on", "off", "~"],
    keys: Keys::Colon,
    ..BASE
};

const TOML_RULES: Rules = Rules {
    line_comment: &["#"],
    block_comment: None,
    keywords: &["true", "false"],
    keys: Keys::Equals,
    sections: true,
    ..BASE
};

const INI_RULES: Rules = Rules {
    line_comment: &["#", ";"],
    block_comment: None,
    keywords: &["true", "false", "yes", "no", "on", "off"],
    keys: Keys::Equals,
    sections: true,
    ..BASE
};

const MAKE_RULES: Rules = Rules {
    line_comment: &["#"],
    block_comment: None,
    keywords: &[
        "define", "else", "endef", "endif", "export", "ifdef", "ifeq", "ifndef", "ifneq",
        "include", "override", "unexport",
    ],
    keys: Keys::Colon,
    ..BASE
};

const DOCKER_RULES: Rules = Rules {
    line_comment: &["#"],
    block_comment: None,
    keywords: &[
        "ADD",
        "ARG",
        "CMD",
        "COPY",
        "ENTRYPOINT",
        "ENV",
        "EXPOSE",
        "FROM",
        "HEALTHCHECK",
        "LABEL",
        "RUN",
        "SHELL",
        "STOPSIGNAL",
        "USER",
        "VOLUME",
        "WORKDIR",
        "AS",
    ],
    ..BASE
};

const ENV_RULES: Rules = Rules {
    line_comment: &["#"],
    block_comment: None,
    keywords: &["export", "true", "false"],
    keys: Keys::Equals,
    ..BASE
};

const ASM_RULES: Rules = Rules {
    line_comment: &[";", "#", "//"],
    keywords: &[
        "add", "and", "bl", "call", "cmp", "jmp", "je", "jne", "ldr", "lea", "mov", "movq", "pop",
        "push", "ret", "str", "sub", "test", "xor",
    ],
    ..BASE
};

// ── языки ────────────────────────────────────────────────────────────────────

static RUST: Language = Language {
    name: "Rust",
    grammar: Grammar::Code(&RUST_RULES),
};
static C: Language = Language {
    name: "C/C++",
    grammar: Grammar::Code(&C_RULES),
};
static GO: Language = Language {
    name: "Go",
    grammar: Grammar::Code(&GO_RULES),
};
static JVM: Language = Language {
    name: "Java/Kotlin",
    grammar: Grammar::Code(&JVM_RULES),
};
static JS: Language = Language {
    name: "JavaScript/TypeScript",
    grammar: Grammar::Code(&JS_RULES),
};
static SWIFT: Language = Language {
    name: "Swift",
    grammar: Grammar::Code(&SWIFT_RULES),
};
static PYTHON: Language = Language {
    name: "Python",
    grammar: Grammar::Code(&PYTHON_RULES),
};
static RUBY: Language = Language {
    name: "Ruby",
    grammar: Grammar::Code(&RUBY_RULES),
};
static PHP: Language = Language {
    name: "PHP",
    grammar: Grammar::Code(&PHP_RULES),
};
static SHELL: Language = Language {
    name: "Shell",
    grammar: Grammar::Code(&SHELL_RULES),
};
static PERL: Language = Language {
    name: "Perl",
    grammar: Grammar::Code(&PERL_RULES),
};
static LUA: Language = Language {
    name: "Lua",
    grammar: Grammar::Code(&LUA_RULES),
};
static SQL: Language = Language {
    name: "SQL",
    grammar: Grammar::Code(&SQL_RULES),
};
static CSS: Language = Language {
    name: "CSS",
    grammar: Grammar::Code(&CSS_RULES),
};
static JSON: Language = Language {
    name: "JSON",
    grammar: Grammar::Code(&JSON_RULES),
};
static YAML: Language = Language {
    name: "YAML",
    grammar: Grammar::Code(&YAML_RULES),
};
static TOML: Language = Language {
    name: "TOML",
    grammar: Grammar::Code(&TOML_RULES),
};
static INI: Language = Language {
    name: "INI",
    grammar: Grammar::Code(&INI_RULES),
};
static MAKE: Language = Language {
    name: "Makefile",
    grammar: Grammar::Code(&MAKE_RULES),
};
static DOCKER: Language = Language {
    name: "Dockerfile",
    grammar: Grammar::Code(&DOCKER_RULES),
};
static ASM: Language = Language {
    name: "Assembly",
    grammar: Grammar::Code(&ASM_RULES),
};
static MARKDOWN: Language = Language {
    name: "Markdown",
    grammar: Grammar::Markdown,
};
static DIFF: Language = Language {
    name: "Diff",
    grammar: Grammar::Diff,
};
static MARKUP: Language = Language {
    name: "XML/HTML",
    grammar: Grammar::Markup,
};
static LOG: Language = Language {
    name: "Log",
    grammar: Grammar::Log,
};
static IGNORE: Language = Language {
    name: "Ignore",
    grammar: Grammar::Ignore,
};
static PROPERTIES: Language = Language {
    name: "Properties",
    grammar: Grammar::Properties,
};
static ENV: Language = Language {
    name: "Env",
    grammar: Grammar::Code(&ENV_RULES),
};

/// Расширение файла → язык.
const BY_EXTENSION: &[(&str, &Language)] = &[
    ("rs", &RUST),
    ("c", &C),
    ("h", &C),
    ("cc", &C),
    ("cpp", &C),
    ("cxx", &C),
    ("hpp", &C),
    ("hh", &C),
    ("m", &C),
    ("mm", &C),
    ("go", &GO),
    ("java", &JVM),
    ("kt", &JVM),
    ("kts", &JVM),
    ("scala", &JVM),
    ("groovy", &JVM),
    ("cs", &JVM),
    ("js", &JS),
    ("mjs", &JS),
    ("cjs", &JS),
    ("jsx", &JS),
    ("ts", &JS),
    ("tsx", &JS),
    ("swift", &SWIFT),
    ("py", &PYTHON),
    ("pyi", &PYTHON),
    ("rb", &RUBY),
    ("php", &PHP),
    ("sh", &SHELL),
    ("bash", &SHELL),
    ("zsh", &SHELL),
    ("ksh", &SHELL),
    ("pl", &PERL),
    ("pm", &PERL),
    ("lua", &LUA),
    ("sql", &SQL),
    ("css", &CSS),
    ("scss", &CSS),
    ("less", &CSS),
    ("json", &JSON),
    ("yaml", &YAML),
    ("yml", &YAML),
    ("toml", &TOML),
    ("ini", &INI),
    ("cfg", &INI),
    ("conf", &INI),
    ("cnf", &INI),
    ("service", &INI),
    ("desktop", &INI),
    ("properties", &PROPERTIES),
    ("env", &ENV),
    ("log", &LOG),
    ("dockerfile", &DOCKER),
    ("tf", &TOML),
    ("hcl", &TOML),
    ("bat", &SHELL),
    ("ps1", &SHELL),
    ("fish", &SHELL),
    ("vue", &MARKUP),
    ("xhtml", &MARKUP),
    ("plist", &MARKUP),
    ("csproj", &MARKUP),
    ("pom", &MARKUP),
    ("gradle", &JVM),
    ("dart", &JVM),
    ("h++", &C),
    ("c++", &C),
    ("rst", &MARKDOWN),
    ("jsonc", &JSON),
    ("json5", &JSON),
    ("ipynb", &JSON),
    ("lock", &TOML),
    ("mk", &MAKE),
    ("s", &ASM),
    ("asm", &ASM),
    ("md", &MARKDOWN),
    ("markdown", &MARKDOWN),
    ("diff", &DIFF),
    ("patch", &DIFF),
    ("xml", &MARKUP),
    ("xsd", &MARKUP),
    ("svg", &MARKUP),
    ("html", &MARKUP),
    ("htm", &MARKUP),
];

/// Имя файла целиком → язык, для файлов без расширения.
const BY_NAME: &[(&str, &Language)] = &[
    ("makefile", &MAKE),
    ("gnumakefile", &MAKE),
    ("dockerfile", &DOCKER),
    ("containerfile", &DOCKER),
    ("cargo.lock", &TOML),
    ("gemfile", &RUBY),
    ("rakefile", &RUBY),
    (".gitconfig", &INI),
    (".editorconfig", &INI),
    (".bashrc", &SHELL),
    (".zshrc", &SHELL),
    (".profile", &SHELL),
    (".bash_profile", &SHELL),
    (".zprofile", &SHELL),
    (".npmrc", &INI),
    (".gitmodules", &INI),
    (".gitattributes", &IGNORE),
    ("codeowners", &IGNORE),
    ("package.json", &JSON),
    ("vagrantfile", &RUBY),
    ("jenkinsfile", &JVM),
    ("cmakelists.txt", &MAKE),
    ("syslog", &LOG),
];

/// Определяет язык по имени файла, расширению или первой строке.
pub fn detect(path: &Path, first_line: &str) -> Option<&'static Language> {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let name = name.to_ascii_lowercase();
        if let Some(language) = lookup(BY_NAME, &name).or_else(|| by_name_pattern(&name)) {
            return Some(language);
        }
    }
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext = ext.to_ascii_lowercase();
        if let Some(language) = lookup(BY_EXTENSION, &ext) {
            return Some(language);
        }
    }
    from_first_line(first_line)
}

/// Определяет язык по тексту, когда имя ничего не сказало: сначала как
/// [`detect`], затем по содержимому — похоже ли оно на журнал.
pub fn detect_text(path: &Path, lines: &[&str]) -> Option<&'static Language> {
    let first = lines.first().copied().unwrap_or("");
    detect(path, first).or_else(|| looks_like_log(lines).then_some(&LOG))
}

/// Семейства имён, которые не перечислить списком.
fn by_name_pattern(name: &str) -> Option<&'static Language> {
    // `.gitignore`, `.dockerignore`, `.prettierignore`…
    if name.starts_with('.') && name.ends_with("ignore") {
        return Some(&IGNORE);
    }
    // `.env`, `.env.local`, `.env.production`.
    if name == ".env" || name.starts_with(".env.") {
        return Some(&ENV);
    }
    // `Dockerfile.dev`, `Containerfile.prod`.
    if name.starts_with("dockerfile.") || name.starts_with("containerfile.") {
        return Some(&DOCKER);
    }
    // Ротация логов: `app.log.1`, `app.log.2.gz` сюда не доходит — это архив.
    if name.contains(".log.") {
        return Some(&LOG);
    }
    None
}

/// Похоже ли начало файла на журнал: большинство непустых строк начинается
/// со времени или содержит уровень сообщения.
fn looks_like_log(lines: &[&str]) -> bool {
    let sample: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .take(30)
        .collect();
    if sample.len() < 2 {
        return false;
    }
    let hits = sample
        .iter()
        .filter(|line| {
            let chars: Vec<char> = line.chars().collect();
            timestamp_end(&chars) > 0
                || line
                    .split(|c: char| !c.is_ascii_alphabetic())
                    .any(is_level_word)
        })
        .count();
    hits * 3 >= sample.len() * 2
}

/// Слово уровня журнала, написанное заглавными: `ERROR`, `WARN`, `INFO`.
fn is_level_word(word: &str) -> bool {
    word.len() >= 4 && word.bytes().all(|b| b.is_ascii_uppercase()) && level_kind(word).is_some()
}

/// Род слова-уровня журнала.
fn level_kind(word: &str) -> Option<Token> {
    match word.to_ascii_uppercase().as_str() {
        "ERROR" | "ERR" | "FATAL" | "CRITICAL" | "CRIT" | "PANIC" | "SEVERE" | "EMERG"
        | "ALERT" => Some(Token::Error),
        "WARN" | "WARNING" => Some(Token::Warn),
        "INFO" | "NOTICE" => Some(Token::Key),
        "DEBUG" | "TRACE" | "VERBOSE" => Some(Token::Comment),
        _ => None,
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Длина метки времени в начале строки (0 — метки нет).
///
/// Понимает ISO-подобные даты (`2026-10-10 12:34:56,789`, `2026-10-10T12:34Z`),
/// время без даты (`12:34:56.123`), формат syslog (`Oct 10 12:34:56`) и всё
/// это же в квадратных скобках.
fn timestamp_end(chars: &[char]) -> usize {
    let bracket = chars.first() == Some(&'[');
    let start = usize::from(bracket);
    let mut i = start;

    // syslog: месяц словом, затем день.
    let head: String = chars.iter().skip(start).take(3).collect();
    let syslog = MONTHS.contains(&head.as_str()) && chars.get(start + 3) == Some(&' ');
    if syslog {
        i = start + 4;
        while chars.get(i) == Some(&' ') {
            i += 1;
        }
    }

    let begin = i;
    let mut digits = 0;
    while i < chars.len() {
        let ch = chars[i];
        let ok = ch.is_ascii_digit()
            || matches!(ch, '-' | ':' | '.' | ',' | '/' | 'T' | 'Z' | '+')
            // Пробел — только между датой и временем, перед цифрой.
            // В syslog за днём месяца сразу идёт время.
            || (ch == ' '
                && chars.get(i + 1).is_some_and(char::is_ascii_digit)
                && (digits >= 4 || syslog));
        if !ok {
            break;
        }
        if ch.is_ascii_digit() {
            digits += 1;
        }
        i += 1;
    }
    // Метка начинается с цифры и содержит разделитель времени или даты.
    let stamp: String = chars[begin..i].iter().collect();
    let plausible = chars.get(begin).is_some_and(char::is_ascii_digit)
        && digits >= 4
        && (stamp.contains(':')
            || stamp.matches('-').count() >= 2
            || stamp.matches('/').count() >= 2);
    if !plausible {
        return 0;
    }
    // Хвостовые разделители к метке не относятся.
    while i > begin && matches!(chars[i - 1], ' ' | ',' | '.' | '-') {
        i -= 1;
    }
    if bracket {
        match chars[i..].iter().position(|&c| c == ']') {
            Some(at) => i + at + 1,
            None => 0,
        }
    } else {
        i
    }
}

fn lookup(table: &[(&str, &'static Language)], key: &str) -> Option<&'static Language> {
    table
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, language)| *language)
}

/// Последняя попытка: shebang или объявление разметки в первой строке.
fn from_first_line(line: &str) -> Option<&'static Language> {
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("<?xml") || lower.starts_with("<!doctype html") {
        return Some(&MARKUP);
    }
    if !lower.starts_with("#!") {
        return None;
    }
    for (needle, language) in [
        ("python", &PYTHON),
        ("ruby", &RUBY),
        ("perl", &PERL),
        ("node", &JS),
        ("lua", &LUA),
        ("bash", &SHELL),
        ("zsh", &SHELL),
        ("/sh", &SHELL),
        ("env sh", &SHELL),
    ] {
        if lower.contains(needle) {
            return Some(language);
        }
    }
    None
}

// ── разбор ───────────────────────────────────────────────────────────────────

/// Незакрытая конструкция, продолжающаяся на следующую строку.
#[derive(Clone, Copy)]
struct Open {
    /// Последовательность, которая её закроет.
    end: &'static str,
    kind: Token,
}

/// Построчный разметчик. Состояние между строками — одна незакрытая
/// конструкция и признак огороженного блока в Markdown.
#[derive(Clone)]
pub struct Highlighter {
    grammar: Option<&'static Grammar>,
    open: Option<Open>,
    /// Markdown: строки внутри ``` ``` ``` показываются как код.
    fenced: bool,
    /// Разметка: тег не закрылся на этой строке.
    in_tag: bool,
}

impl Highlighter {
    /// Разметчик для языка; `None` — текст без подсветки.
    pub fn new(language: Option<&'static Language>) -> Self {
        Self {
            grammar: language.map(|language| &language.grammar),
            open: None,
            fenced: false,
            in_tag: false,
        }
    }

    /// Размечает очередную строку исходного текста.
    pub fn line(&mut self, text: &str) -> Vec<Piece> {
        match self.grammar {
            None => vec![whole(Token::Plain, text)],
            Some(Grammar::Code(rules)) => self.code(text, rules),
            Some(Grammar::Diff) => vec![whole(diff_kind(text), text)],
            Some(Grammar::Markdown) => self.markdown(text),
            Some(Grammar::Markup) => self.markup(text),
            Some(Grammar::Log) => log_line(text),
            Some(Grammar::Ignore) => ignore_line(text),
            Some(Grammar::Properties) => properties_line(text),
        }
    }

    /// Основной разбор: комментарии, строки, числа, слова.
    fn code(&mut self, text: &str, rules: &Rules) -> Vec<Piece> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Pieces::default();
        let mut i = 0;

        if let Some(open) = self.open {
            match find(&chars, 0, open.end) {
                Some(at) => {
                    i = at + open.end.chars().count();
                    out.push(open.kind, &chars[..i]);
                    self.open = None;
                }
                None => {
                    out.push(open.kind, &chars);
                    return out.done();
                }
            }
        }

        // Заголовок секции размечается целиком: внутри него нет ни строк,
        // ни чисел, которые стоило бы разбирать отдельно.
        if rules.sections && i == 0 && text.trim_start().starts_with('[') {
            out.push(Token::Key, &chars);
            return out.done();
        }

        // Пока не встретилось ни одной значащей лексемы, следующее слово или
        // строка могут оказаться ключом.
        let mut fresh = true;
        while i < chars.len() {
            if rules
                .line_comment
                .iter()
                .any(|prefix| starts_with(&chars, i, prefix))
            {
                out.push(Token::Comment, &chars[i..]);
                break;
            }

            if let Some((open, close)) = rules.block_comment
                && starts_with(&chars, i, open)
            {
                let from = i + open.chars().count();
                match find(&chars, from, close) {
                    Some(at) => {
                        let to = at + close.chars().count();
                        out.push(Token::Comment, &chars[i..to]);
                        i = to;
                    }
                    None => {
                        out.push(Token::Comment, &chars[i..]);
                        self.open = Some(Open {
                            end: close,
                            kind: Token::Comment,
                        });
                        i = chars.len();
                    }
                }
                fresh = false;
                continue;
            }

            if rules.quotes.contains(&chars[i]) {
                let (end, kind) = self.string(&chars, i, rules, fresh);
                out.push(kind, &chars[i..end]);
                i = end;
                fresh = false;
                continue;
            }

            if chars[i].is_ascii_digit() {
                let end = number_end(&chars, i);
                out.push(Token::Number, &chars[i..end]);
                i = end;
                fresh = false;
                continue;
            }

            if is_ident_start(chars[i]) {
                let end = ident_end(&chars, i);
                let word: String = chars[i..end].iter().collect();
                out.push(word_kind(&word, rules, fresh, &chars, end), &chars[i..end]);
                i = end;
                // `export KEY=value` в `.env` и shell: ключ идёт после export.
                fresh = fresh && word == "export";
                continue;
            }

            if fresh && !keeps_fresh(chars[i], rules.keys) {
                fresh = false;
            }
            out.push(Token::Plain, &chars[i..i + 1]);
            i += 1;
        }
        out.done()
    }

    /// Границы строкового литерала, начинающегося на позиции `at`.
    fn string(&mut self, chars: &[char], at: usize, rules: &Rules, fresh: bool) -> (usize, Token) {
        let quote = chars[at];
        // Тройные кавычки открывают литерал, который может занять много строк.
        if rules.triple && starts_with(chars, at + 1, &quote.to_string().repeat(2)) {
            let end: &'static str = if quote == '"' { "\"\"\"" } else { "'''" };
            return match find(chars, at + 3, end) {
                Some(found) => (found + 3, Token::Str),
                None => {
                    self.open = Some(Open {
                        end,
                        kind: Token::Str,
                    });
                    (chars.len(), Token::Str)
                }
            };
        }

        let mut i = at + 1;
        while i < chars.len() {
            if chars[i] == '\\' {
                i += 2;
                continue;
            }
            if chars[i] == quote {
                i += 1;
                break;
            }
            i += 1;
        }
        let end = i.min(chars.len());
        // Строка в начале строчки перед двоеточием — ключ объекта JSON.
        let kind = if fresh && rules.keys == Keys::Colon && next_visible(chars, end) == Some(':') {
            Token::Key
        } else {
            Token::Str
        };
        (end, kind)
    }

    fn markdown(&mut self, text: &str) -> Vec<Piece> {
        let trimmed = text.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            self.fenced = !self.fenced;
            return vec![whole(Token::Keyword, text)];
        }
        if self.fenced {
            return vec![whole(Token::Str, text)];
        }
        if trimmed.starts_with('#') {
            return vec![whole(Token::Keyword, text)];
        }
        if trimmed.starts_with('>') {
            return vec![whole(Token::Comment, text)];
        }
        if trimmed.starts_with("---") || trimmed.starts_with("===") {
            return vec![whole(Token::Comment, text)];
        }

        let chars: Vec<char> = text.chars().collect();
        let mut out = Pieces::default();
        let mut i = 0;
        // Маркер списка перед текстом пункта.
        let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
        if matches!(chars.get(indent), Some('-' | '*' | '+')) && chars.get(indent + 1) == Some(&' ')
        {
            out.push(Token::Plain, &chars[..indent]);
            out.push(Token::Number, &chars[indent..indent + 1]);
            i = indent + 1;
        }

        while i < chars.len() {
            match chars[i] {
                '`' => {
                    let end = find(&chars, i + 1, "`").map_or(chars.len(), |at| at + 1);
                    out.push(Token::Str, &chars[i..end]);
                    i = end;
                }
                '[' => {
                    let end = find(&chars, i + 1, "]").map_or(chars.len(), |at| at + 1);
                    out.push(Token::Type, &chars[i..end]);
                    i = end;
                }
                '(' if out.last_kind() == Some(Token::Type) => {
                    let end = find(&chars, i + 1, ")").map_or(chars.len(), |at| at + 1);
                    out.push(Token::Key, &chars[i..end]);
                    i = end;
                }
                _ => {
                    out.push(Token::Plain, &chars[i..i + 1]);
                    i += 1;
                }
            }
        }
        out.done()
    }

    fn markup(&mut self, text: &str) -> Vec<Piece> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Pieces::default();
        let mut i = 0;

        if let Some(open) = self.open {
            match find(&chars, 0, open.end) {
                Some(at) => {
                    i = at + open.end.chars().count();
                    out.push(open.kind, &chars[..i]);
                    self.open = None;
                }
                None => {
                    out.push(open.kind, &chars);
                    return out.done();
                }
            }
        }

        while i < chars.len() {
            if !self.in_tag {
                if starts_with(&chars, i, "<!--") {
                    match find(&chars, i + 4, "-->") {
                        Some(at) => {
                            out.push(Token::Comment, &chars[i..at + 3]);
                            i = at + 3;
                        }
                        None => {
                            out.push(Token::Comment, &chars[i..]);
                            self.open = Some(Open {
                                end: "-->",
                                kind: Token::Comment,
                            });
                            i = chars.len();
                        }
                    }
                    continue;
                }
                if chars[i] == '<' {
                    // Открывающая скобка и имя тега: `<div`, `</div`, `<?xml`.
                    let name_at =
                        i + 1 + usize::from(matches!(chars.get(i + 1), Some('/' | '?' | '!')));
                    let end = ident_end(&chars, name_at.min(chars.len()));
                    out.push(Token::Plain, &chars[i..name_at.min(chars.len())]);
                    out.push(Token::Keyword, &chars[name_at.min(chars.len())..end]);
                    i = end;
                    self.in_tag = true;
                    continue;
                }
                let end = chars[i..]
                    .iter()
                    .position(|&c| c == '<')
                    .map_or(chars.len(), |at| i + at);
                out.push(Token::Plain, &chars[i..end]);
                i = end;
                continue;
            }

            match chars[i] {
                '>' => {
                    out.push(Token::Plain, &chars[i..i + 1]);
                    self.in_tag = false;
                    i += 1;
                }
                '"' | '\'' => {
                    let quote = chars[i];
                    let end = chars[i + 1..]
                        .iter()
                        .position(|&c| c == quote)
                        .map_or(chars.len(), |at| i + at + 2);
                    out.push(Token::Str, &chars[i..end]);
                    i = end;
                }
                c if is_ident_start(c) => {
                    let end = ident_end(&chars, i);
                    out.push(Token::Key, &chars[i..end]);
                    i = end;
                }
                _ => {
                    out.push(Token::Plain, &chars[i..i + 1]);
                    i += 1;
                }
            }
        }
        out.done()
    }
}

impl Language {
    /// Исходник, а не текст: рядом стоит показывать номера строк.
    pub fn numbered(&self) -> bool {
        !matches!(
            self.grammar,
            Grammar::Markdown | Grammar::Diff | Grammar::Log
        )
    }
}

/// Строка журнала: время, уровень, ссылки, строки, числа, пары `ключ=значение`.
fn log_line(text: &str) -> Vec<Piece> {
    let trimmed = text.trim_start();
    let indented = trimmed.len() != text.len();
    // Трассировка стека: Java, Python, Rust.
    if (indented && (trimmed.starts_with("at ") || trimmed.starts_with("File \"")))
        || trimmed.starts_with("Traceback ")
        || trimmed.starts_with("Caused by:")
        || trimmed.starts_with("stack backtrace:")
    {
        return vec![whole(Token::Comment, text)];
    }

    let chars: Vec<char> = text.chars().collect();
    let mut out = Pieces::default();
    let mut i = timestamp_end(&chars);
    out.push(Token::Number, &chars[..i]);

    while i < chars.len() {
        let ch = chars[i];
        if starts_with(&chars, i, "http://") || starts_with(&chars, i, "https://") {
            let end = chars[i..]
                .iter()
                .position(|c| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ']' | '>'))
                .map_or(chars.len(), |at| i + at);
            out.push(Token::Type, &chars[i..end]);
            i = end;
            continue;
        }
        if ch == '"' {
            let end = chars[i + 1..]
                .iter()
                .position(|&c| c == '"')
                .map_or(chars.len(), |at| i + at + 2);
            out.push(Token::Str, &chars[i..end]);
            i = end;
            continue;
        }
        if ch.is_ascii_digit() {
            // Адреса, версии и время целиком: `192.168.0.1`, `1.2.3`, `12:30`.
            let mut end = i;
            while end < chars.len()
                && (chars[end].is_ascii_alphanumeric()
                    || (matches!(chars[end], '.' | ':')
                        && chars.get(end + 1).is_some_and(char::is_ascii_digit)))
            {
                end += 1;
            }
            out.push(Token::Number, &chars[i..end]);
            i = end;
            continue;
        }
        if is_ident_start(ch) {
            let end = ident_end(&chars, i);
            let word: String = chars[i..end].iter().collect();
            let kind = level_kind(&word).unwrap_or(if chars.get(end) == Some(&'=') {
                Token::Key
            } else {
                Token::Plain
            });
            out.push(kind, &chars[i..end]);
            i = end;
            continue;
        }
        out.push(Token::Plain, &chars[i..i + 1]);
        i += 1;
    }
    out.done()
}

/// Строка ignore-файла: комментарий, отрицание `!`, маски, директории.
fn ignore_line(text: &str) -> Vec<Piece> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('#') {
        return vec![whole(Token::Comment, text)];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = Pieces::default();
    let lead = chars.len() - trimmed.chars().count();
    out.push(Token::Plain, &chars[..lead]);
    let mut i = lead;
    if chars.get(i) == Some(&'!') {
        out.push(Token::Removed, &chars[i..i + 1]);
        i += 1;
    }
    // Шаблон, оканчивающийся на `/`, относится только к директориям.
    let dir = trimmed.trim_end().ends_with('/');
    let base = if dir { Token::Type } else { Token::Plain };
    while i < chars.len() {
        let kind = if matches!(chars[i], '*' | '?' | '[' | ']') {
            Token::Keyword
        } else {
            base
        };
        out.push(kind, &chars[i..i + 1]);
        i += 1;
    }
    out.done()
}

/// Строка `.properties`: ключ до первого `=`, `:` или пробела, остальное — значение.
fn properties_line(text: &str) -> Vec<Piece> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with('!') {
        return vec![whole(Token::Comment, text)];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = Pieces::default();
    let lead = chars.len() - trimmed.chars().count();
    out.push(Token::Plain, &chars[..lead]);
    let mut i = lead;
    while i < chars.len() && !matches!(chars[i], '=' | ':' | ' ' | '\t') {
        // Экранированный разделитель входит в ключ.
        i += if chars[i] == '\\' { 2 } else { 1 };
    }
    let i = i.min(chars.len());
    out.push(Token::Key, &chars[lead..i]);
    let mut j = i;
    while j < chars.len() && matches!(chars[j], '=' | ':' | ' ' | '\t') {
        j += 1;
    }
    out.push(Token::Plain, &chars[i..j]);
    out.push(Token::Str, &chars[j..]);
    out.done()
}

/// Род строки в выводе `diff`.
fn diff_kind(text: &str) -> Token {
    if text.starts_with("+++") || text.starts_with("---") {
        return Token::Keyword;
    }
    match text.as_bytes().first() {
        Some(b'+') => Token::Added,
        Some(b'-') => Token::Removed,
        Some(b'@') => Token::Key,
        _ if text.starts_with("diff ") || text.starts_with("index ") => Token::Keyword,
        _ => Token::Plain,
    }
}

/// Род слова: ключ, ключевое слово, встроенное имя, тип или обычный текст.
fn word_kind(word: &str, rules: &Rules, fresh: bool, chars: &[char], end: usize) -> Token {
    let expects = match rules.keys {
        Keys::Colon => Some(':'),
        Keys::Equals => Some('='),
        Keys::None => None,
    };
    if fresh && expects.is_some() && next_visible(chars, end) == expects {
        return Token::Key;
    }
    if rules.keywords.contains(&word) {
        return Token::Keyword;
    }
    if rules.builtins.contains(&word) {
        return Token::Type;
    }
    // Регистронезависимые языки вроде SQL пишут ключевые слова как придётся.
    let upper = word.to_ascii_uppercase();
    if rules.keywords.contains(&upper.as_str()) || rules.builtins.contains(&upper.as_str()) {
        return Token::Keyword;
    }
    if rules.capital_is_type && word.chars().next().is_some_and(char::is_uppercase) {
        return Token::Type;
    }
    Token::Plain
}

/// Может ли символ стоять перед ключом, не отменяя его: отступ, дефис списка
/// YAML, скобки и запятые JSON.
fn keeps_fresh(ch: char, keys: Keys) -> bool {
    match keys {
        Keys::None => false,
        Keys::Colon => matches!(ch, ' ' | '\t' | '-' | '{' | '[' | ','),
        Keys::Equals => matches!(ch, ' ' | '\t'),
    }
}

/// Ближайший непробельный символ после позиции `from`.
fn next_visible(chars: &[char], from: usize) -> Option<char> {
    chars[from.min(chars.len())..]
        .iter()
        .find(|c| !c.is_whitespace())
        .copied()
}

fn is_ident_start(ch: char) -> bool {
    ch.is_alphabetic() || ch == '_' || ch == '$' || ch == '@'
}

fn is_ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$'
}

fn ident_end(chars: &[char], from: usize) -> usize {
    let mut end = from;
    while end < chars.len() && is_ident_char(chars[end]) {
        end += 1;
    }
    end.max(from)
}

/// Конец числового литерала: цифры, буквы систем счисления, разделители и
/// точка, за которой снова идёт цифра.
fn number_end(chars: &[char], from: usize) -> usize {
    let mut end = from;
    while end < chars.len() {
        let ch = chars[end];
        // Точка входит в число только перед цифрой: `1.5` — литерал,
        // а `1.max(2)` — вызов метода.
        let part = ch.is_ascii_alphanumeric()
            || ch == '_'
            || (ch == '.' && chars.get(end + 1).is_some_and(char::is_ascii_digit));
        if !part {
            break;
        }
        end += 1;
    }
    end
}

/// Начинается ли последовательность с позиции `at` с образца.
fn starts_with(chars: &[char], at: usize, needle: &str) -> bool {
    let mut expected = needle.chars();
    let mut i = at;
    loop {
        match expected.next() {
            None => return true,
            Some(ch) => {
                if chars.get(i) != Some(&ch) {
                    return false;
                }
                i += 1;
            }
        }
    }
}

/// Позиция первого вхождения образца начиная с `from`.
fn find(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let len = needle.chars().count();
    if len == 0 || from > chars.len() {
        return None;
    }
    (from..=chars.len().saturating_sub(len)).find(|&at| starts_with(chars, at, needle))
}

/// Строка целиком одним фрагментом.
fn whole(kind: Token, text: &str) -> Piece {
    Piece {
        kind,
        text: text.to_string(),
    }
}

/// Накопитель фрагментов, склеивающий соседние одного рода.
#[derive(Default)]
struct Pieces(Vec<Piece>);

impl Pieces {
    fn push(&mut self, kind: Token, chars: &[char]) {
        if chars.is_empty() {
            return;
        }
        match self.0.last_mut() {
            Some(last) if last.kind == kind => last.text.extend(chars),
            _ => self.0.push(Piece {
                kind,
                text: chars.iter().collect(),
            }),
        }
    }

    fn last_kind(&self) -> Option<Token> {
        self.0.last().map(|piece| piece.kind)
    }

    fn done(self) -> Vec<Piece> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Размечает одну строку и возвращает пары «род — текст».
    fn mark(path: &str, line: &str) -> Vec<(Token, String)> {
        let language = detect(Path::new(path), line);
        let mut highlighter = Highlighter::new(language);
        highlighter
            .line(line)
            .into_iter()
            .map(|piece| (piece.kind, piece.text))
            .collect()
    }

    /// Собирает текст фрагментов указанного рода.
    fn of_kind(marked: &[(Token, String)], kind: Token) -> Vec<&str> {
        marked
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, text)| text.as_str())
            .collect()
    }

    #[test]
    fn detects_language_by_extension_and_name() {
        assert_eq!(detect(Path::new("a/main.rs"), "").unwrap().name, "Rust");
        assert_eq!(detect(Path::new("Cargo.toml"), "").unwrap().name, "TOML");
        assert_eq!(detect(Path::new("Makefile"), "").unwrap().name, "Makefile");
        // Регистр имени не важен.
        assert_eq!(
            detect(Path::new("dockerfile"), "").unwrap().name,
            "Dockerfile"
        );
        assert!(detect(Path::new("notes.txt"), "просто текст").is_none());
    }

    #[test]
    fn detects_language_by_shebang() {
        assert_eq!(
            detect(Path::new("run"), "#!/usr/bin/env python3")
                .unwrap()
                .name,
            "Python"
        );
        assert_eq!(
            detect(Path::new("run"), "#!/bin/bash").unwrap().name,
            "Shell"
        );
        assert_eq!(
            detect(Path::new("f"), "<?xml version=\"1.0\"?>")
                .unwrap()
                .name,
            "XML/HTML"
        );
        assert!(detect(Path::new("run"), "не shebang").is_none());
    }

    #[test]
    fn rust_line_is_split_by_kind() {
        let marked = mark("main.rs", "let x: u32 = 42; // счётчик");
        assert_eq!(of_kind(&marked, Token::Keyword), vec!["let"]);
        assert_eq!(of_kind(&marked, Token::Type), vec!["u32"]);
        assert_eq!(of_kind(&marked, Token::Number), vec!["42"]);
        assert_eq!(of_kind(&marked, Token::Comment), vec!["// счётчик"]);
        // Фрагменты покрывают строку целиком и в исходном порядке.
        let joined: String = marked.iter().map(|(_, text)| text.as_str()).collect();
        assert_eq!(joined, "let x: u32 = 42; // счётчик");
    }

    #[test]
    fn strings_swallow_escapes_and_comment_markers() {
        let marked = mark("main.rs", r#"let s = "a\"// не комментарий";"#);
        assert_eq!(
            of_kind(&marked, Token::Str),
            vec![r#""a\"// не комментарий""#]
        );
        assert!(of_kind(&marked, Token::Comment).is_empty());
    }

    #[test]
    fn block_comment_spans_lines() {
        let mut h = Highlighter::new(detect(Path::new("a.c"), ""));
        let first = h.line("int x; /* начало");
        assert_eq!(first.last().unwrap().kind, Token::Comment);
        let middle = h.line("всё ещё комментарий");
        assert_eq!(middle[0].kind, Token::Comment);
        let last = h.line("конец */ int y;");
        assert_eq!(last[0].kind, Token::Comment);
        assert_eq!(last[0].text, "конец */");
        // После закрытия разбор продолжается как обычно.
        assert!(
            last.iter()
                .any(|p| p.kind == Token::Type && p.text == "int")
        );
    }

    #[test]
    fn python_triple_quotes_span_lines() {
        let mut h = Highlighter::new(detect(Path::new("a.py"), ""));
        let first = h.line("doc = \"\"\"начало");
        assert_eq!(first.last().unwrap().kind, Token::Str);
        let last = h.line("конец\"\"\" # хвост");
        assert_eq!(last[0].kind, Token::Str);
        assert_eq!(
            of_kind(
                &last
                    .iter()
                    .map(|p| (p.kind, p.text.clone()))
                    .collect::<Vec<_>>(),
                Token::Comment
            ),
            vec!["# хвост"]
        );
    }

    #[test]
    fn keys_are_marked_in_config_formats() {
        assert_eq!(
            of_kind(&mark("c.toml", "name = \"flob\""), Token::Key),
            vec!["name"]
        );
        assert_eq!(
            of_kind(&mark("c.toml", "[package]"), Token::Key),
            vec!["[package]"]
        );
        assert_eq!(
            of_kind(&mark("c.yaml", "  - name: сборка"), Token::Key),
            vec!["name"]
        );
        assert_eq!(
            of_kind(&mark("c.json", "  \"key\": 12"), Token::Key),
            vec!["\"key\""]
        );
        // Строка справа от двоеточия остаётся значением.
        assert_eq!(
            of_kind(&mark("c.json", "  \"key\": \"value\""), Token::Str),
            vec!["\"value\""]
        );
    }

    #[test]
    fn sql_keywords_are_case_insensitive() {
        let marked = mark("q.sql", "select * from users where id = 1");
        assert!(of_kind(&marked, Token::Keyword).contains(&"select"));
        assert!(of_kind(&marked, Token::Keyword).contains(&"where"));
    }

    #[test]
    fn diff_lines_are_coloured_by_prefix() {
        assert_eq!(mark("a.patch", "+добавлено")[0].0, Token::Added);
        assert_eq!(mark("a.patch", "-удалено")[0].0, Token::Removed);
        assert_eq!(mark("a.patch", "@@ -1,4 +1,6 @@")[0].0, Token::Key);
        assert_eq!(mark("a.patch", "--- a/файл")[0].0, Token::Keyword);
        assert_eq!(mark("a.patch", " без изменений")[0].0, Token::Plain);
    }

    #[test]
    fn markdown_structure_is_marked() {
        assert_eq!(mark("r.md", "## Заголовок")[0].0, Token::Keyword);
        assert_eq!(mark("r.md", "> цитата")[0].0, Token::Comment);
        let marked = mark("r.md", "- пункт с `кодом` и [ссылкой](http://x)");
        assert_eq!(of_kind(&marked, Token::Str), vec!["`кодом`"]);
        assert_eq!(of_kind(&marked, Token::Type), vec!["[ссылкой]"]);
        assert_eq!(of_kind(&marked, Token::Key), vec!["(http://x)"]);

        // Огороженный блок целиком показывается как код.
        let mut h = Highlighter::new(detect(Path::new("r.md"), ""));
        assert_eq!(h.line("```rust")[0].kind, Token::Keyword);
        assert_eq!(h.line("# это код, а не заголовок")[0].kind, Token::Str);
        assert_eq!(h.line("```")[0].kind, Token::Keyword);
        assert_eq!(h.line("# а это заголовок")[0].kind, Token::Keyword);
    }

    #[test]
    fn markup_tags_attributes_and_values() {
        let marked = mark("p.html", "<a href=\"/тут\">текст</a>");
        assert!(of_kind(&marked, Token::Keyword).contains(&"a"));
        assert_eq!(of_kind(&marked, Token::Key), vec!["href"]);
        assert_eq!(of_kind(&marked, Token::Str), vec!["\"/тут\""]);
        // Текст между тегами склеивается со скобками в один обычный фрагмент.
        assert!(
            of_kind(&marked, Token::Plain)
                .iter()
                .any(|piece| piece.contains("текст")),
            "{marked:?}"
        );
    }

    #[test]
    fn unknown_extension_yields_one_plain_piece() {
        let marked = mark("data.unknown", "какой-то текст 123");
        assert_eq!(marked.len(), 1);
        assert_eq!(marked[0].0, Token::Plain);
    }

    #[test]
    fn pieces_always_reconstruct_the_line() {
        let samples = [
            ("main.rs", "fn main() { println!(\"{}\", 1.5e3); }"),
            ("a.py", "def f(x=1): return 'строка' # хвост"),
            ("a.json", "{\"a\": [1, true, null]}"),
            ("a.yml", "key: значение # заметка"),
            ("a.html", "<div class='x'>привет</div>"),
            ("a.md", "текст `код` [ссылка](url) конец"),
            ("a.sh", "echo \"$HOME\" # дом"),
            ("a.unknown", ""),
        ];
        for (path, line) in samples {
            let joined: String = mark(path, line).iter().map(|(_, t)| t.as_str()).collect();
            assert_eq!(joined, line, "разметка потеряла текст в {path}");
        }
    }

    #[test]
    fn detects_config_families_by_name() {
        let name = |p: &str| detect(Path::new(p), "").map(|l| l.name);
        assert_eq!(name(".gitignore"), Some("Ignore"));
        assert_eq!(name(".dockerignore"), Some("Ignore"));
        assert_eq!(name(".env.local"), Some("Env"));
        assert_eq!(name("Dockerfile.dev"), Some("Dockerfile"));
        assert_eq!(name("app.properties"), Some("Properties"));
        assert_eq!(name("server.log"), Some("Log"));
        assert_eq!(name("server.log.1"), Some("Log"));
        assert_eq!(name("setup.ini"), Some("INI"));
    }

    #[test]
    fn log_is_detected_by_content() {
        let lines = [
            "2026-10-10 12:00:01,123 INFO  starting",
            "2026-10-10 12:00:02,456 WARN  slow disk",
            "",
            "2026-10-10 12:00:03,789 ERROR failed",
        ];
        assert_eq!(
            detect_text(Path::new("output"), &lines).unwrap().name,
            "Log"
        );
        let prose = ["Just some notes.", "Nothing to see here.", "Really."];
        assert!(detect_text(Path::new("notes"), &prose).is_none());
    }

    #[test]
    fn timestamps_of_common_formats() {
        let ts = |s: &str| {
            let chars: Vec<char> = s.chars().collect();
            chars[..timestamp_end(&chars)].iter().collect::<String>()
        };
        assert_eq!(
            ts("2026-10-10 12:34:56,789 INFO x"),
            "2026-10-10 12:34:56,789"
        );
        assert_eq!(
            ts("2026-10-10T12:34:56Z level=info"),
            "2026-10-10T12:34:56Z"
        );
        assert_eq!(ts("[2026-10-10 12:34:56] ERROR"), "[2026-10-10 12:34:56]");
        assert_eq!(ts("Oct 10 12:34:56 host sshd"), "Oct 10 12:34:56");
        assert_eq!(ts("12:34:56.001 tick"), "12:34:56.001");
        assert_eq!(ts("1234 apples"), "");
        assert_eq!(ts("version 2"), "");
    }

    #[test]
    fn log_levels_urls_and_keys() {
        let marked = mark(
            "a.log",
            "2026-10-10 12:00:00 ERROR request to https://x.io/a failed user=bob ip 10.0.0.1",
        );
        assert_eq!(of_kind(&marked, Token::Error), vec!["ERROR"]);
        assert_eq!(of_kind(&marked, Token::Type), vec!["https://x.io/a"]);
        assert_eq!(of_kind(&marked, Token::Key), vec!["user"]);
        assert!(of_kind(&marked, Token::Number).contains(&"10.0.0.1"));
        assert_eq!(mark("a.log", "WARN disk")[0].0, Token::Warn);
        assert_eq!(
            mark("a.log", "    at com.x.Main.run(Main.java:10)")[0].0,
            Token::Comment
        );
    }

    #[test]
    fn ignore_and_properties_lines() {
        let marked = mark(".gitignore", "!build/*.o");
        assert_eq!(marked[0], (Token::Removed, "!".to_string()));
        assert!(of_kind(&marked, Token::Keyword).contains(&"*"));
        assert_eq!(mark(".gitignore", "# c")[0].0, Token::Comment);

        let marked = mark("a.properties", "db.url = jdbc:x");
        assert_eq!(of_kind(&marked, Token::Key), vec!["db.url"]);
        assert_eq!(of_kind(&marked, Token::Str), vec!["jdbc:x"]);
        assert_eq!(mark("a.properties", "! note")[0].0, Token::Comment);

        let marked = mark(".env", "export API_URL=http://x # c");
        assert_eq!(of_kind(&marked, Token::Keyword), vec!["export"]);
        assert_eq!(of_kind(&marked, Token::Key), vec!["API_URL"]);
    }

    #[test]
    fn new_grammars_reconstruct_the_line() {
        for (path, line) in [
            ("a.log", "[2026-10-10 1:2:3] INFO \"q\" https://a b=c 1.2.3"),
            (".gitignore", "  /target/**/*.rs"),
            ("a.properties", "a\\=b : c"),
            ("a.log", "Oct  1 00:00:00 h k[1]: done"),
        ] {
            let joined: String = mark(path, line).iter().map(|(_, t)| t.as_str()).collect();
            assert_eq!(joined, line, "{path}");
        }
    }
}
