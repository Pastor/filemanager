//! Интеграция с оболочкой: функция `f`, которая после выхода из программы
//! переходит в директорию активной панели.
//!
//! Процесс не может сменить текущую директорию родительской оболочки, поэтому
//! программа записывает директорию в файл (`--cwd-file`), а функция оболочки
//! читает его и делает `cd`. Подключается строкой в файле настроек оболочки:
//! `eval "$(fmgr --shell-init zsh)"`.

/// Поддерживаемые оболочки.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
}

/// Текст функции `f` для оболочки.
pub fn init(shell: Shell) -> &'static str {
    match shell {
        Shell::Zsh | Shell::Bash => POSIX,
        Shell::Fish => FISH,
    }
}

const POSIX: &str = r#"f() {
    local tmp dir
    tmp="$(mktemp -t fmgr-cwd.XXXXXX)" || return
    command fmgr --cwd-file "$tmp" "$@"
    dir="$(cat -- "$tmp" 2>/dev/null)"
    rm -f -- "$tmp"
    if [ -n "$dir" ] && [ -d "$dir" ] && [ "$dir" != "$PWD" ]; then
        cd -- "$dir" || return
    fi
}
"#;

const FISH: &str = r#"function f
    set -l tmp (mktemp -t fmgr-cwd.XXXXXX); or return
    command fmgr --cwd-file $tmp $argv
    set -l dir (cat -- $tmp 2>/dev/null)
    rm -f -- $tmp
    if test -n "$dir" -a -d "$dir" -a "$dir" != "$PWD"
        cd -- $dir
    end
end
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn functions_pass_arguments_and_use_cwd_file() {
        for shell in [Shell::Zsh, Shell::Bash, Shell::Fish] {
            let text = init(shell);
            assert!(text.contains("command fmgr --cwd-file"));
            assert!(text.contains("cd -- "));
        }
        assert!(init(Shell::Zsh).contains("\"$@\""));
        assert!(init(Shell::Fish).contains("$argv"));
    }

    /// Функция действительно работает в настоящей оболочке: подменяем fmgr
    /// скриптом, который пишет директорию в файл, и смотрим, куда попали.
    #[cfg(unix)]
    #[test]
    fn posix_function_changes_directory() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let target = tmp.path().join("target dir");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        let fake = bin.join("fmgr");
        std::fs::write(
            &fake,
            format!("#!/bin/sh\nprintf '%s' '{}' > \"$2\"\n", target.display()),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let script = format!("{}\nf\npwd -P\n", init(Shell::Bash));
        let out = std::process::Command::new("/bin/bash")
            .arg("-c")
            .arg(script)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let pwd = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(
            std::path::Path::new(&pwd),
            target.canonicalize().unwrap(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
