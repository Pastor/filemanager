//! Архивы: распознавание, распаковка, упаковка, список содержимого.
//!
//! zip, tar, tar.gz, tar.zst и одиночный gz разбираются встроенными крейтами.
//! RAR и 7z свободных библиотек не имеют, поэтому для них вызывается внешняя
//! программа — `unrar`, `7z`/`7zz` или `bsdtar` (на macOS он есть всегда).
//!
//! Распаковка и упаковка — такие же фоновые операции, как копирование:
//! прогресс, вопросы и отмена идут через [`Control`], итог — [`Report`].

use std::cell::Cell;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

use crate::ops::{self, Choice, Control, Progress, Report};

/// Формат архива.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
    TarZst,
    /// Одиночный сжатый файл.
    Gz,
    /// RAR, 7z — через внешнюю программу.
    External,
}

impl Format {
    /// Форматы, в которые умеем упаковывать.
    pub const PACKABLE: [Format; 4] = [Format::Zip, Format::Tar, Format::TarGz, Format::TarZst];

    /// Распознаёт архив по имени файла.
    pub fn detect(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
        let ends = |s: &str| name.ends_with(s);
        Some(if ends(".tar.gz") || ends(".tgz") {
            Format::TarGz
        } else if ends(".tar.zst") || ends(".tzst") {
            Format::TarZst
        } else if ends(".tar") {
            Format::Tar
        } else if ends(".zip") || ends(".jar") || ends(".war") || ends(".apk") || ends(".whl") {
            Format::Zip
        } else if ends(".gz") {
            Format::Gz
        } else if ends(".rar") || ends(".7z") {
            Format::External
        } else {
            return None;
        })
    }

    /// Расширение файла архива.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::Tar => "tar",
            Format::TarGz => "tar.gz",
            Format::TarZst => "tar.zst",
            Format::Gz => "gz",
            Format::External => "",
        }
    }
}

/// Элемент архива для просмотра.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// Сколько элементов перечислять в просмотре.
const LIST_LIMIT: usize = 10_000;

/// Содержимое архива для просмотра.
pub fn list(path: &Path) -> io::Result<Vec<Item>> {
    let format = Format::detect(path).ok_or_else(|| io::Error::other("not an archive"))?;
    let mut items = Vec::new();
    match format {
        Format::Zip => {
            let mut zip = zip::ZipArchive::new(BufReader::new(File::open(path)?))
                .map_err(io::Error::other)?;
            for i in 0..zip.len().min(LIST_LIMIT) {
                let file = zip.by_index(i).map_err(io::Error::other)?;
                items.push(Item {
                    name: file.name().trim_end_matches('/').to_string(),
                    size: file.size(),
                    is_dir: file.is_dir(),
                });
            }
        }
        Format::Tar | Format::TarGz | Format::TarZst => {
            let reader = decoder(format, BufReader::new(File::open(path)?))?;
            let mut tar = tar::Archive::new(reader);
            for entry in tar.entries()?.take(LIST_LIMIT) {
                let entry = entry?;
                let is_dir = entry.header().entry_type().is_dir();
                items.push(Item {
                    name: entry
                        .path()?
                        .to_string_lossy()
                        .trim_end_matches('/')
                        .to_string(),
                    size: entry.header().size().unwrap_or(0),
                    is_dir,
                });
            }
        }
        Format::Gz => items.push(Item {
            name: gz_inner_name(path),
            size: gz_size(path).unwrap_or(0),
            is_dir: false,
        }),
        Format::External => items = external_list(path)?,
    }
    Ok(items)
}

/// Распаковывает архив в существующую директорию `dest`.
pub fn extract<C: Control + ?Sized>(archive: &Path, dest: &Path, ctl: &mut C) -> Report {
    let mut report = Report::default();
    let Some(format) = Format::detect(archive) else {
        report
            .errors
            .push(format!("{}: not an archive", archive.display()));
        return report;
    };
    let mut progress = Progress {
        bytes_total: fs::metadata(archive).map(|m| m.len()).unwrap_or(0),
        current: archive.to_path_buf(),
        ..Progress::default()
    };
    ctl.progress(&progress);
    let result = match format {
        Format::Zip => extract_zip(archive, dest, ctl, &mut progress),
        Format::Tar | Format::TarGz | Format::TarZst => {
            extract_tar(format, archive, dest, ctl, &mut progress)
        }
        Format::Gz => extract_gz(archive, dest, ctl, &mut progress),
        Format::External => extract_external(archive, dest, ctl),
    };
    match result {
        Ok(true) => report.done = 1,
        Ok(false) => report.cancelled = true,
        Err(err) => report.errors.push(format!("{}: {err}", archive.display())),
    }
    progress.bytes_done = progress.bytes_total;
    ctl.progress(&progress);
    report
}

/// Упаковывает `sources` в файл `dest`. Пишет во временный файл рядом и
/// переименовывает в конце: прерванная упаковка не оставит битого архива.
pub fn pack<C: Control + ?Sized>(
    format: Format,
    sources: &[PathBuf],
    dest: &Path,
    ctl: &mut C,
) -> Report {
    let mut report = Report::default();
    if fs::symlink_metadata(dest).is_ok() {
        match ctl.conflict(dest) {
            Choice::Replace | Choice::ReplaceAll => {}
            Choice::Skip | Choice::SkipAll => {
                report.skipped = 1;
                return report;
            }
            Choice::Cancel => {
                report.cancelled = true;
                return report;
            }
        }
    }
    let mut progress = Progress::default();
    for src in sources {
        let (bytes, items) = ops::measure(src);
        progress.bytes_total += bytes;
        progress.items_total += items;
    }
    ctl.progress(&progress);

    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    let part = dest.with_file_name(format!(".{name}.part"));
    let mut packer = Packer {
        ctl,
        progress,
        cancelled: false,
    };
    let result = packer.write(format, sources, &part);
    let cancelled = packer.cancelled;
    match result {
        Ok(()) if !cancelled => match fs::rename(&part, dest) {
            Ok(()) => report.done = sources.len(),
            Err(err) => report.errors.push(format!("{}: {err}", dest.display())),
        },
        Ok(()) => report.cancelled = true,
        Err(err) => report.errors.push(format!("{}: {err}", dest.display())),
    }
    if report.done == 0 {
        let _ = fs::remove_file(&part);
    }
    report
}

// ── распаковка ──────────────────────────────────────────────────────────────

/// Считает прочитанные байты сжатого файла — по ним видно продвижение.
struct Counting<R> {
    inner: R,
    read: Rc<Cell<u64>>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read.set(self.read.get() + n as u64);
        Ok(n)
    }
}

/// Поток распаковки для tar-семейства.
fn decoder<'a, R: Read + 'a>(format: Format, reader: R) -> io::Result<Box<dyn Read + 'a>> {
    Ok(match format {
        Format::TarGz | Format::Gz => Box::new(flate2::read::MultiGzDecoder::new(reader)),
        Format::TarZst => Box::new(zstd::stream::read::Decoder::new(reader)?),
        _ => Box::new(reader),
    })
}

fn extract_tar<C: Control + ?Sized>(
    format: Format,
    archive: &Path,
    dest: &Path,
    ctl: &mut C,
    progress: &mut Progress,
) -> io::Result<bool> {
    let read = Rc::new(Cell::new(0));
    let counting = Counting {
        inner: BufReader::new(File::open(archive)?),
        read: Rc::clone(&read),
    };
    let mut tar = tar::Archive::new(decoder(format, counting)?);
    tar.set_preserve_permissions(true);
    tar.set_overwrite(true);
    for entry in tar.entries()? {
        if ctl.cancelled() {
            return Ok(false);
        }
        let mut entry = entry?;
        progress.current = entry.path()?.into_owned();
        // `unpack_in` отказывается писать за пределы `dest` (`../` в именах).
        entry.unpack_in(dest)?;
        progress.items_done += 1;
        progress.bytes_done = read.get();
        ctl.progress(progress);
    }
    Ok(true)
}

fn extract_zip<C: Control + ?Sized>(
    archive: &Path,
    dest: &Path,
    ctl: &mut C,
    progress: &mut Progress,
) -> io::Result<bool> {
    let mut zip =
        zip::ZipArchive::new(BufReader::new(File::open(archive)?)).map_err(io::Error::other)?;
    // Для zip продвижение честнее считать по распакованным байтам.
    progress.bytes_total = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.size()))
        .sum();
    progress.items_total = zip.len() as u64;
    let mut buf = vec![0u8; 256 * 1024];
    for i in 0..zip.len() {
        if ctl.cancelled() {
            return Ok(false);
        }
        let mut file = zip.by_index(i).map_err(io::Error::other)?;
        // Имя с `../` или абсолютным путём пропускаем: это попытка выйти из `dest`.
        let Some(name) = file.enclosed_name() else {
            continue;
        };
        let out = dest.join(&name);
        progress.current = name;
        if file.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            if file.is_symlink() {
                let mut target = String::new();
                file.read_to_string(&mut target)?;
                make_symlink(&target, &out)?;
            } else {
                let mut output = File::create(&out)?;
                loop {
                    let n = file.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    output.write_all(&buf[..n])?;
                    progress.bytes_done += n as u64;
                    ctl.progress(progress);
                    if ctl.cancelled() {
                        return Ok(false);
                    }
                }
                set_mode(&out, file.unix_mode());
            }
        }
        progress.items_done += 1;
        ctl.progress(progress);
    }
    Ok(true)
}

fn extract_gz<C: Control + ?Sized>(
    archive: &Path,
    dest: &Path,
    ctl: &mut C,
    progress: &mut Progress,
) -> io::Result<bool> {
    let read = Rc::new(Cell::new(0));
    let counting = Counting {
        inner: BufReader::new(File::open(archive)?),
        read: Rc::clone(&read),
    };
    let mut input = flate2::read::MultiGzDecoder::new(counting);
    let mut output = File::create(dest.join(gz_inner_name(archive)))?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if ctl.cancelled() {
            return Ok(false);
        }
        let n = input.read(&mut buf)?;
        if n == 0 {
            return Ok(true);
        }
        output.write_all(&buf[..n])?;
        progress.bytes_done = read.get();
        ctl.progress(progress);
    }
}

/// Имя файла внутри `.gz`: имя архива без расширения.
fn gz_inner_name(archive: &Path) -> String {
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    let lower = name.to_ascii_lowercase();
    match lower.strip_suffix(".gz") {
        Some(stem) if !stem.is_empty() => name[..stem.len()].to_string(),
        _ => format!("{name}.out"),
    }
}

/// Размер распакованного `.gz` из его хвоста (по модулю 4 ГиБ, как в формате).
fn gz_size(archive: &Path) -> io::Result<u64> {
    use std::io::{Seek, SeekFrom};
    let mut file = File::open(archive)?;
    file.seek(SeekFrom::End(-4))?;
    let mut tail = [0u8; 4];
    file.read_exact(&mut tail)?;
    Ok(u32::from_le_bytes(tail) as u64)
}

#[cfg(unix)]
fn make_symlink(target: &str, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn make_symlink(_target: &str, _link: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777));
    }
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: Option<u32>) {}

// ── внешние программы ───────────────────────────────────────────────────────

/// Программа для RAR и 7z.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Unrar,
    SevenZip,
    Bsdtar,
}

/// Ищет подходящую программу в `PATH`, начиная с самой надёжной.
fn find_tool(archive: &Path) -> Option<(PathBuf, Tool)> {
    let rar = archive
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("rar"));
    let mut candidates = Vec::new();
    if rar {
        candidates.push(("unrar", Tool::Unrar));
    }
    candidates.extend([
        ("7zz", Tool::SevenZip),
        ("7z", Tool::SevenZip),
        ("7za", Tool::SevenZip),
        ("bsdtar", Tool::Bsdtar),
    ]);
    let path = std::env::var_os("PATH")?;
    for (name, tool) in candidates {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some((candidate, tool));
            }
        }
    }
    None
}

fn no_tool() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no unrar, 7z or bsdtar found")
}

/// Распаковывает внешней программой. Прогресса она не сообщает, но отмену
/// поддерживаем: процесс завершается, как только её попросили.
fn extract_external<C: Control + ?Sized>(
    archive: &Path,
    dest: &Path,
    ctl: &mut C,
) -> io::Result<bool> {
    let (program, tool) = find_tool(archive).ok_or_else(no_tool)?;
    let mut command = Command::new(program);
    match tool {
        Tool::Unrar => command.arg("x").arg("-o+").arg("-y").arg(archive).arg(dest),
        Tool::SevenZip => command
            .arg("x")
            .arg("-y")
            .arg(format!("-o{}", dest.display()))
            .arg(archive),
        Tool::Bsdtar => command.arg("-xf").arg(archive).arg("-C").arg(dest),
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    loop {
        if ctl.cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(true);
            }
            let mut err = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_string(&mut err);
            }
            let last = err
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            return Err(io::Error::other(format!("{status}: {last}")));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Список содержимого внешней программой.
fn external_list(archive: &Path) -> io::Result<Vec<Item>> {
    let (program, tool) = find_tool(archive).ok_or_else(no_tool)?;
    let mut command = Command::new(program);
    match tool {
        Tool::Unrar => command.arg("lb").arg(archive),
        Tool::SevenZip => command.arg("l").arg("-ba").arg("-slt").arg(archive),
        Tool::Bsdtar => command.arg("-tf").arg(archive),
    };
    let output = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(output.status.to_string()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(match tool {
        Tool::SevenZip => parse_7z_slt(&text),
        _ => text
            .lines()
            .filter(|l| !l.is_empty())
            .take(LIST_LIMIT)
            .map(|l| Item {
                name: l.trim_end_matches('/').to_string(),
                size: 0,
                is_dir: l.ends_with('/'),
            })
            .collect(),
    })
}

/// Разбирает технический вывод `7z l -slt`: блоки `Ключ = значение`.
fn parse_7z_slt(text: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut current: Option<Item> = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(" = ") else {
            continue;
        };
        match key {
            "Path" => {
                items.extend(current.take());
                current = Some(Item {
                    name: value.to_string(),
                    size: 0,
                    is_dir: false,
                });
            }
            "Size" => {
                if let Some(item) = &mut current {
                    item.size = value.parse().unwrap_or(0);
                }
            }
            "Folder" | "Attributes" if value.starts_with('+') || value.starts_with('D') => {
                if let Some(item) = &mut current {
                    item.is_dir = true;
                }
            }
            _ => {}
        }
        if items.len() >= LIST_LIMIT {
            break;
        }
    }
    items.extend(current);
    items
}

// ── упаковка ────────────────────────────────────────────────────────────────

struct Packer<'a, C: Control + ?Sized> {
    ctl: &'a mut C,
    progress: Progress,
    cancelled: bool,
}

impl<C: Control + ?Sized> Packer<'_, C> {
    fn stop(&mut self) -> bool {
        if self.ctl.cancelled() {
            self.cancelled = true;
        }
        self.cancelled
    }

    fn write(&mut self, format: Format, sources: &[PathBuf], dest: &Path) -> io::Result<()> {
        let file = io::BufWriter::new(File::create(dest)?);
        match format {
            Format::Zip => {
                let mut zip = zip::ZipWriter::new(file);
                for src in sources {
                    self.zip_add(&mut zip, src, &top_name(src))?;
                }
                zip.finish().map_err(io::Error::other)?.flush()
            }
            Format::Tar => self.tar_all(file, sources)?.flush(),
            Format::TarGz => {
                let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
                self.tar_all(gz, sources)?.finish()?.flush()
            }
            Format::TarZst => {
                let zst = zstd::stream::write::Encoder::new(file, 3)?;
                self.tar_all(zst, sources)?.finish()?.flush()
            }
            Format::Gz | Format::External => Err(io::Error::other("unsupported format")),
        }
    }

    fn tar_all<W: Write>(&mut self, out: W, sources: &[PathBuf]) -> io::Result<W> {
        let mut tar = tar::Builder::new(out);
        // Ссылки остаются ссылками, как при копировании.
        tar.follow_symlinks(false);
        for src in sources {
            self.tar_add(&mut tar, src, &top_name(src))?;
        }
        tar.into_inner()
    }

    fn tar_add<W: Write>(
        &mut self,
        tar: &mut tar::Builder<W>,
        path: &Path,
        name: &str,
    ) -> io::Result<()> {
        if self.stop() {
            return Ok(());
        }
        self.progress.current = path.to_path_buf();
        let meta = fs::symlink_metadata(path)?;
        tar.append_path_with_name(path, name)?;
        self.progress.items_done += 1;
        if meta.is_file() {
            self.progress.bytes_done += meta.len();
        }
        self.ctl.progress(&self.progress);
        if meta.is_dir() {
            for child in sorted_children(path)? {
                let child_name = format!(
                    "{name}/{}",
                    child.file_name().unwrap_or_default().to_string_lossy()
                );
                self.tar_add(tar, &child, &child_name)?;
            }
        }
        Ok(())
    }

    fn zip_add<W: Write + io::Seek>(
        &mut self,
        zip: &mut zip::ZipWriter<W>,
        path: &Path,
        name: &str,
    ) -> io::Result<()> {
        if self.stop() {
            return Ok(());
        }
        self.progress.current = path.to_path_buf();
        let meta = fs::symlink_metadata(path)?;
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(mode_of(&meta))
            .large_file(meta.len() >= u32::MAX as u64);
        if meta.file_type().is_symlink() {
            let target = fs::read_link(path)?;
            zip.add_symlink(name, target.to_string_lossy(), options)
                .map_err(io::Error::other)?;
        } else if meta.is_dir() {
            zip.add_directory(format!("{name}/"), options)
                .map_err(io::Error::other)?;
            self.progress.items_done += 1;
            for child in sorted_children(path)? {
                let child_name = format!(
                    "{name}/{}",
                    child.file_name().unwrap_or_default().to_string_lossy()
                );
                self.zip_add(zip, &child, &child_name)?;
            }
            return Ok(());
        } else {
            zip.start_file(name, options).map_err(io::Error::other)?;
            let mut input = File::open(path)?;
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                if self.stop() {
                    return Ok(());
                }
                let n = input.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                zip.write_all(&buf[..n])?;
                self.progress.bytes_done += n as u64;
                self.ctl.progress(&self.progress);
            }
        }
        self.progress.items_done += 1;
        self.ctl.progress(&self.progress);
        Ok(())
    }
}

/// Имя элемента верхнего уровня в архиве.
fn top_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string())
}

/// Содержимое директории в устойчивом порядке: архив не должен зависеть от
/// порядка, в котором файловая система отдаёт записи.
fn sorted_children(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut children: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    children.sort();
    Ok(children)
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn mode_of(meta: &fs::Metadata) -> u32 {
    if meta.is_dir() { 0o755 } else { 0o644 }
}

/// Заготовки для тестов других модулей.
#[cfg(test)]
pub mod tests_support {
    use super::*;

    /// Операция без вопросов: конфликты решаются заменой.
    pub struct Silent;

    impl Control for Silent {
        fn progress(&mut self, _: &Progress) {}
        fn conflict(&mut self, _: &Path) -> Choice {
            Choice::Replace
        }
        fn cancelled(&self) -> bool {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::Silent;
    use super::*;

    fn sample(root: &Path) -> PathBuf {
        let dir = root.join("proj");
        fs::create_dir_all(dir.join("src/empty")).unwrap();
        fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.join("README"), "read me").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(dir.join("run.sh"), "#!/bin/sh\n").unwrap();
            fs::set_permissions(dir.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
            std::os::unix::fs::symlink("README", dir.join("link")).unwrap();
        }
        dir
    }

    fn check_tree(out: &Path) {
        assert_eq!(
            fs::read_to_string(out.join("proj/src/main.rs")).unwrap(),
            "fn main() {}\n"
        );
        assert_eq!(
            fs::read_to_string(out.join("proj/README")).unwrap(),
            "read me"
        );
        assert!(out.join("proj/src/empty").is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(out.join("proj/run.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
            assert_eq!(
                fs::read_link(out.join("proj/link")).unwrap(),
                PathBuf::from("README")
            );
        }
    }

    #[test]
    fn detect_by_name() {
        let d = |n: &str| Format::detect(Path::new(n));
        assert_eq!(d("a.tar.gz"), Some(Format::TarGz));
        assert_eq!(d("A.TGZ"), Some(Format::TarGz));
        assert_eq!(d("a.tar.zst"), Some(Format::TarZst));
        assert_eq!(d("a.tar"), Some(Format::Tar));
        assert_eq!(d("a.zip"), Some(Format::Zip));
        assert_eq!(d("lib.jar"), Some(Format::Zip));
        assert_eq!(d("log.gz"), Some(Format::Gz));
        assert_eq!(d("a.rar"), Some(Format::External));
        assert_eq!(d("a.7z"), Some(Format::External));
        assert_eq!(d("a.txt"), None);
    }

    #[test]
    fn every_packable_format_roundtrips() {
        for format in Format::PACKABLE {
            let tmp = tempfile::tempdir().unwrap();
            let src = sample(tmp.path());
            let archive = tmp.path().join(format!("out.{}", format.extension()));
            let report = pack(format, std::slice::from_ref(&src), &archive, &mut Silent);
            assert_eq!(report.done, 1, "{format:?}: {:?}", report.errors);
            assert!(
                !tmp.path()
                    .join(format!(".out.{}.part", format.extension()))
                    .exists()
            );

            let names: Vec<String> = list(&archive)
                .unwrap()
                .into_iter()
                .map(|i| i.name)
                .collect();
            assert!(
                names.contains(&"proj/src/main.rs".to_string()),
                "{format:?}: {names:?}"
            );

            let out = tmp.path().join("x");
            fs::create_dir(&out).unwrap();
            let report = extract(&archive, &out, &mut Silent);
            assert_eq!(report.done, 1, "{format:?}: {:?}", report.errors);
            check_tree(&out);
        }
    }

    #[test]
    fn single_gz_is_decompressed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("app.log.gz");
        let mut enc = flate2::write::GzEncoder::new(
            File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        enc.write_all(b"line 1\nline 2\n").unwrap();
        enc.finish().unwrap();

        let items = list(&path).unwrap();
        assert_eq!(
            items,
            [Item {
                name: "app.log".into(),
                size: 14,
                is_dir: false
            }]
        );
        let out = tmp.path().join("out");
        fs::create_dir(&out).unwrap();
        assert_eq!(extract(&path, &out, &mut Silent).done, 1);
        assert_eq!(
            fs::read_to_string(out.join("app.log")).unwrap(),
            "line 1\nline 2\n"
        );
    }

    #[test]
    fn zip_slip_entries_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("evil.zip");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("../escape.txt", opts).unwrap();
        zip.write_all(b"x").unwrap();
        zip.start_file("ok.txt", opts).unwrap();
        zip.write_all(b"y").unwrap();
        zip.finish().unwrap();

        let out = tmp.path().join("out");
        fs::create_dir(&out).unwrap();
        extract(&path, &out, &mut Silent);
        assert!(out.join("ok.txt").exists());
        assert!(!tmp.path().join("escape.txt").exists());
    }

    #[test]
    fn existing_destination_asks_and_can_skip() {
        struct SkipAll;
        impl Control for SkipAll {
            fn progress(&mut self, _: &Progress) {}
            fn conflict(&mut self, _: &Path) -> Choice {
                Choice::Skip
            }
            fn cancelled(&self) -> bool {
                false
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let src = sample(tmp.path());
        let dest = tmp.path().join("a.zip");
        fs::write(&dest, "old").unwrap();
        let report = pack(Format::Zip, &[src], &dest, &mut SkipAll);
        assert_eq!(report.skipped, 1);
        assert_eq!(fs::read_to_string(&dest).unwrap(), "old");
    }

    #[test]
    fn corrupt_archive_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.zip");
        fs::write(&path, "not a zip").unwrap();
        let report = extract(&path, tmp.path(), &mut Silent);
        assert_eq!(report.errors.len(), 1);
        assert!(list(&path).is_err());
    }

    #[test]
    fn parses_7z_technical_listing() {
        let text =
            "Path = docs\nSize = 0\nFolder = +\n\nPath = docs/a.txt\nSize = 12\nFolder = -\n";
        assert_eq!(
            parse_7z_slt(text),
            [
                Item {
                    name: "docs".into(),
                    size: 0,
                    is_dir: true
                },
                Item {
                    name: "docs/a.txt".into(),
                    size: 12,
                    is_dir: false
                },
            ]
        );
    }

    #[test]
    fn external_tool_extracts_when_available() {
        // На машине без 7z/bsdtar проверять нечего.
        let tmp = tempfile::tempdir().unwrap();
        let fake = tmp.path().join("x.7z");
        fs::write(&fake, "garbage").unwrap();
        if find_tool(&fake).is_none() {
            return;
        }
        let report = extract(&fake, tmp.path(), &mut Silent);
        assert_eq!(
            report.errors.len(),
            1,
            "мусор вместо архива — ошибка программы"
        );
    }
}
