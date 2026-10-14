//! Список дисков для окна выбора диска (`g`).
//!
//! В Windows у каждого диска своё дерево (`C:\`, `D:\`), и из корня одного диска
//! стрелкой `←` в другой не попасть — нужен явный выбор. Список берётся из
//! `GetLogicalDrives`, тип, метка и свободное место — из WinAPI. Функции
//! объявлены здесь же: ради пяти вызовов из `kernel32` не нужна отдельная
//! зависимость.
//!
//! В Unix дисков в этом смысле нет, но то же окно полезно как список точек
//! монтирования: корень, домашняя директория, флешки и внешние диски
//! (`/Volumes/*` в macOS, блочные устройства из `/proc/mounts` в Linux).

use std::path::{Path, PathBuf};

/// Что за диск — для подписи в списке.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Часть вариантов встречается только в Windows, `Home` — только в Unix.
#[allow(dead_code)]
pub enum Kind {
    /// Встроенный диск или корень файловой системы.
    Fixed,
    /// Флешка, карта памяти, внешний диск.
    Removable,
    /// Сетевой диск.
    Network,
    /// CD/DVD.
    Optical,
    /// Домашняя директория (только Unix).
    Home,
    Other,
}

/// Диск или точка монтирования.
#[derive(Debug, Clone)]
pub struct Drive {
    /// Куда перейти: `C:\`, `/`, `/Volumes/USB`.
    pub path: PathBuf,
    /// Короткое имя: `C:`, `/`, `USB`.
    pub name: String,
    /// Метка тома, если есть.
    pub label: String,
    pub kind: Kind,
    /// Объём и свободное место; `None` — узнать не удалось (нет носителя,
    /// сетевой диск, который не стоит будить).
    pub space: Option<Space>,
}

/// Объём тома в байтах.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    pub total: u64,
    pub free: u64,
}

/// Индекс диска, на котором лежит `dir`: самый длинный совпавший путь, чтобы
/// `/Volumes/USB/x` попал на флешку, а не на корень.
pub fn containing(drives: &[Drive], dir: &Path) -> Option<usize> {
    drives
        .iter()
        .enumerate()
        .filter(|(_, d)| starts_with(dir, &d.path))
        .max_by_key(|(_, d)| d.path.as_os_str().len())
        .map(|(i, _)| i)
}

/// `Path::starts_with` без учёта регистра в Windows: `c:\x` лежит на `C:\`.
fn starts_with(dir: &Path, root: &Path) -> bool {
    if cfg!(windows) {
        let dir = dir.to_string_lossy().to_lowercase();
        let root = root.to_string_lossy().to_lowercase();
        dir.starts_with(&root)
    } else {
        dir.starts_with(root)
    }
}

#[cfg(windows)]
pub use windows::list;

#[cfg(unix)]
pub use unix::list;

#[cfg(not(any(unix, windows)))]
pub fn list() -> Vec<Drive> {
    Vec::new()
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    use super::{Drive, Kind, Space};

    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    /// Не показывать системное окно «Вставьте диск» для пустого привода.
    const SEM_FAILCRITICALERRORS: u32 = 0x0001;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLogicalDrives() -> u32;
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn GetDiskFreeSpaceExW(
            root: *const u16,
            free_to_caller: *mut u64,
            total: *mut u64,
            free_total: *mut u64,
        ) -> i32;
        fn GetVolumeInformationW(
            root: *const u16,
            name: *mut u16,
            name_len: u32,
            serial: *mut u32,
            max_component: *mut u32,
            flags: *mut u32,
            fs_name: *mut u16,
            fs_name_len: u32,
        ) -> i32;
        fn SetErrorMode(mode: u32) -> u32;
    }

    /// Все диски из `GetLogicalDrives`, от `A:` до `Z:`.
    pub fn list() -> Vec<Drive> {
        // SAFETY: функции без указателей, меняют только режим ошибок процесса.
        let (mask, old_mode) =
            unsafe { (GetLogicalDrives(), SetErrorMode(SEM_FAILCRITICALERRORS)) };
        let drives = (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| drive(char::from(b'A' + i)))
            .collect();
        // SAFETY: возвращаем режим, который был до нас.
        unsafe { SetErrorMode(old_mode) };
        drives
    }

    fn drive(letter: char) -> Drive {
        let root = format!("{letter}:\\");
        let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `wide` — строка с завершающим нулём, живёт до конца функции.
        let kind = match unsafe { GetDriveTypeW(wide.as_ptr()) } {
            DRIVE_FIXED => Kind::Fixed,
            DRIVE_REMOVABLE => Kind::Removable,
            DRIVE_REMOTE => Kind::Network,
            DRIVE_CDROM => Kind::Optical,
            _ => Kind::Other,
        };
        // Отключённый сетевой диск отвечает по таймауту в несколько секунд:
        // окно выбора из-за него зависать не должно.
        let space = (kind != Kind::Network).then(|| space(&wide)).flatten();
        let label = if space.is_some() {
            label(&wide)
        } else {
            String::new()
        };
        Drive {
            path: PathBuf::from(&root),
            name: format!("{letter}:"),
            label,
            kind,
            space,
        }
    }

    fn space(root: &[u16]) -> Option<Space> {
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        // SAFETY: `root` завершается нулём, указатели на локальные переменные.
        let ok = unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut avail, &mut total, &mut free) };
        (ok != 0).then_some(Space { total, free: avail })
    }

    fn label(root: &[u16]) -> String {
        let mut name = [0u16; 261];
        // SAFETY: `root` завершается нулём, длина буфера передана верно,
        // необязательные выходные параметры — нулевые указатели.
        let ok = unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                name.as_mut_ptr(),
                name.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        if ok == 0 {
            return String::new();
        }
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        OsString::from_wide(&name[..len])
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(unix)]
mod unix {
    use std::collections::HashSet;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use super::{Drive, Kind, Space};

    /// Корень, домашняя директория и подключённые тома.
    pub fn list() -> Vec<Drive> {
        let mut drives = vec![mount(PathBuf::from("/"), "/".to_string(), Kind::Fixed)];
        if let Some(home) = dirs::home_dir() {
            drives.push(mount(home, "~".to_string(), Kind::Home));
        }
        let mut seen: HashSet<PathBuf> = drives.iter().map(|d| d.path.clone()).collect();
        for (path, kind) in volumes() {
            if seen.insert(path.clone()) {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                drives.push(mount(path, name, kind));
            }
        }
        drives
    }

    fn mount(path: PathBuf, name: String, kind: Kind) -> Drive {
        Drive {
            space: space(&path),
            path,
            name,
            label: String::new(),
            kind,
        }
    }

    /// Тома macOS: всё в `/Volumes`, кроме ссылки на системный диск.
    #[cfg(target_os = "macos")]
    fn volumes() -> Vec<(PathBuf, Kind)> {
        let Ok(dir) = std::fs::read_dir("/Volumes") else {
            return Vec::new();
        };
        let mut found: Vec<(PathBuf, Kind)> = dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| !p.is_symlink() && p.is_dir())
            .map(|p| (p, Kind::Removable))
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    }

    /// Тома Linux: блочные устройства из `/proc/mounts`, кроме служебных
    /// разделов вроде `/boot` и пакетов snap.
    #[cfg(not(target_os = "macos"))]
    fn volumes() -> Vec<(PathBuf, Kind)> {
        let Ok(mounts) = std::fs::read_to_string("/proc/mounts") else {
            return Vec::new();
        };
        let mut found: Vec<(PathBuf, Kind)> = mounts
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let device = fields.next()?;
                let point = unescape(fields.next()?);
                if !device.starts_with("/dev/") || is_service(&point) {
                    return None;
                }
                let removable = ["/media/", "/run/media/", "/mnt/"]
                    .iter()
                    .any(|p| point.starts_with(p));
                let kind = if removable {
                    Kind::Removable
                } else {
                    Kind::Fixed
                };
                Some((PathBuf::from(point), kind))
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    }

    #[cfg(not(target_os = "macos"))]
    fn is_service(point: &str) -> bool {
        ["/boot", "/snap", "/var/snap", "/efi"]
            .iter()
            .any(|p| point == *p || point.starts_with(&format!("{p}/")))
    }

    /// В `/proc/mounts` пробелы и спецсимволы записаны как `\040` (восьмерично).
    #[cfg(not(target_os = "macos"))]
    fn unescape(field: &str) -> String {
        let bytes = field.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            let octal = bytes
                .get(i + 1..i + 4)
                .filter(|d| d.iter().all(|b| (b'0'..=b'7').contains(b)));
            match octal {
                Some(d) if bytes[i] == b'\\' => {
                    out.push((d[0] - b'0') * 64 + (d[1] - b'0') * 8 + (d[2] - b'0'));
                    i += 4;
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn space(path: &Path) -> Option<Space> {
        let c = CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` завершается нулём, `st` — локальная структура нужного типа.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        let block = st.f_frsize as u64;
        Some(Space {
            total: st.f_blocks as u64 * block,
            free: st.f_bavail as u64 * block,
        })
    }

    #[cfg(all(test, not(target_os = "macos")))]
    mod tests {
        use super::*;

        #[test]
        fn mount_points_are_unescaped() {
            assert_eq!(unescape(r"/media/My\040Disk"), "/media/My Disk");
            assert_eq!(unescape(r"/mnt/plain"), "/mnt/plain");
        }

        #[test]
        fn service_mounts_are_skipped() {
            assert!(is_service("/boot/efi"));
            assert!(is_service("/snap"));
            assert!(!is_service("/bootstrap"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(path: &str) -> Drive {
        Drive {
            path: PathBuf::from(path),
            name: path.to_string(),
            label: String::new(),
            kind: Kind::Fixed,
            space: None,
        }
    }

    #[test]
    fn containing_prefers_longest_root() {
        let drives = [drive("/"), drive("/Volumes/USB")];
        assert_eq!(containing(&drives, Path::new("/Volumes/USB/docs")), Some(1));
        assert_eq!(containing(&drives, Path::new("/etc")), Some(0));
        assert_eq!(containing(&drives, Path::new("/Volumes/USB2")), Some(0));
    }

    #[test]
    fn list_has_root() {
        let drives = list();
        assert!(!drives.is_empty());
        assert!(drives.iter().all(|d| d.path.is_absolute()));
    }
}
