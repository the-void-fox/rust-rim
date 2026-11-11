//! Свои префиксы для параллельных прогонов.
//!
//! Два экземпляра игры в одном префиксе мешают друг другу. Проверено на
//! живых прогонах: один из пары встал на старте с нулём CPU и так и не
//! написал лог, у других в выводе было «fsync: a previous shm file … was not
//! properly removed». Wine держит объекты синхронизации в /dev/shm под
//! именем, выведенным из префикса, и два wineserver одного префикса — каждый
//! в своём контейнере umu — портят их друг другу.
//!
//! Поэтому у каждого рабочего места, кроме первого, — своя копия префикса.
//! Копия делается один раз и дальше переиспользуется: Proton сам обновит её,
//! если сменится версия.

use std::io;
use std::path::{Path, PathBuf};

/// Метка копии: внутри — путь к исходному префиксу.
const MARKER: &str = ".rustrim-clone";

/// Папки внутри LocalLow, которые не копируются: это данные самой игры —
/// сохранения и настройки игрока, сотни мегабайт. Прогону они не нужны:
/// у него своя папка данных.
const SKIPPED_DATA: [&str; 2] = ["Ludeon Studios", "RimWorld by Ludeon Studios"];

/// Где лежит копия префикса для рабочего места `worker` (с 1).
pub fn clone_dir(main: &Path, worker: usize) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let name = main.file_name()?.to_string_lossy().into_owned();
    Some(data.join("rustrim/prefixes").join(format!("{name}-{}", worker + 1)))
}

/// Готова ли копия именно этого префикса.
pub fn is_ready(main: &Path, dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(MARKER)).is_ok_and(|source| Path::new(source.trim()) == main)
}

/// Копирует префикс `main` в `dir`, если там ещё нет готовой копии.
///
/// Копия собирается рядом и переименовывается в конце: оборванное
/// копирование не оставит полуготовый префикс, который сошёл бы за целый.
pub fn ensure_clone(main: &Path, dir: &Path) -> io::Result<()> {
    if is_ready(main, dir) {
        return Ok(());
    }
    if dir.exists() {
        // Стирать можно только свою копию — например, другого префикса.
        if !dir.join(MARKER).is_file() {
            return Err(io::Error::other(format!("{} exists and is not a rust-rim prefix copy", dir.display())));
        }
        std::fs::remove_dir_all(dir)?;
    }
    let partial = dir.with_extension("partial");
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial)?;
    copy_tree(main, &partial, Path::new(""))?;
    std::fs::write(partial.join(MARKER), main.to_string_lossy().as_bytes())?;
    std::fs::rename(&partial, dir)
}

/// Копирует дерево, сохраняя символические ссылки как ссылки.
///
/// В префиксе Proton их больше тысячи: системные DLL ведут в дистрибутив
/// Proton, а `pfx` — на сам префикс. Пройти по ним значило бы скопировать
/// Proton целиком, а по `pfx` — уйти в бесконечную рекурсию.
fn copy_tree(src: &Path, dst: &Path, rel: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let rel = rel.join(entry.file_name());
        if is_game_data(&rel) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            symlink(&std::fs::read_link(&from)?, &to)?;
        } else if kind.is_dir() {
            std::fs::create_dir_all(&to)?;
            copy_tree(&from, &to, &rel)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// `drive_c/users/<кто угодно>/AppData/LocalLow/<данные игры>`.
fn is_game_data(rel: &Path) -> bool {
    let parts: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect();
    parts.len() == 6
        && parts[0] == "drive_c"
        && parts[1] == "users"
        && parts[3] == "AppData"
        && parts[4] == "LocalLow"
        && SKIPPED_DATA.contains(&parts[5].as_ref())
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_target: &Path, _link: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rustrim_workers_{}_{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, rel: &str, body: &str) {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_prefix_copy_keeps_links_and_skips_game_data() {
        let t = Temp::new("clone");
        let main = t.0.join("294100");
        t.file("294100/system.reg", "reg");
        t.file("294100/drive_c/windows/system32/kernel32.dll", "dll");
        t.file("294100/drive_c/users/steamuser/AppData/LocalLow/Ludeon Studios/RimWorld by Ludeon Studios/Saves/colony.rws", "save");
        t.file("294100/drive_c/users/steamuser/AppData/Roaming/other.txt", "keep");
        std::os::unix::fs::symlink(".", main.join("pfx")).unwrap();
        std::os::unix::fs::symlink("/opt/proton/notepad.exe", main.join("drive_c/windows/notepad.exe")).unwrap();

        let dir = t.0.join("copy");
        ensure_clone(&main, &dir).unwrap();

        assert!(is_ready(&main, &dir));
        assert_eq!(std::fs::read_to_string(dir.join("system.reg")).unwrap(), "reg");
        assert!(dir.join("drive_c/windows/system32/kernel32.dll").is_file());
        assert!(dir.join("drive_c/users/steamuser/AppData/Roaming/other.txt").is_file());
        // Ссылки остались ссылками, а не скопированным содержимым.
        assert_eq!(std::fs::read_link(dir.join("pfx")).unwrap(), Path::new("."));
        assert_eq!(std::fs::read_link(dir.join("drive_c/windows/notepad.exe")).unwrap(), Path::new("/opt/proton/notepad.exe"));
        // Сохранения игрока в копию не попали.
        assert!(!dir.join("drive_c/users/steamuser/AppData/LocalLow/Ludeon Studios").exists());
        assert!(!t.0.join("copy.partial").exists());

        // Второй раз — без копирования: копия уже готова.
        std::fs::write(main.join("system.reg"), "changed").unwrap();
        ensure_clone(&main, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("system.reg")).unwrap(), "reg");
    }

    #[test]
    fn a_foreign_folder_is_not_overwritten() {
        let t = Temp::new("foreign");
        t.file("main/system.reg", "reg");
        t.file("busy/important.txt", "data");
        assert!(ensure_clone(&t.0.join("main"), &t.0.join("busy")).is_err());
        assert!(t.0.join("busy/important.txt").is_file());
    }

    #[test]
    fn copies_are_named_after_the_prefix_and_worker() {
        let dir = clone_dir(Path::new("/home/u/.config/hydralauncher/wine-prefixes/294100"), 1).unwrap();
        assert!(dir.ends_with("rustrim/prefixes/294100-2"), "{}", dir.display());
    }
}
