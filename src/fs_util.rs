//! Файловые операции над папками модов.

use std::path::Path;

/// Переносит все папки модов из `src_dir` (папка SteamCMD content/294100/)
/// в `dst_dir` (RimWorld/Mods). Если папка с таким именем уже существует
/// в назначении — она будет заменена (старая удаляется).
///
/// Используется fs::rename, а при ошибке (например, перенос между разными
/// файловыми системами) — fallback через рекурсивное копирование.
pub fn move_downloaded_mods(src_dir: &Path, dst_dir: &Path) {
    if !src_dir.is_dir() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(dst_dir) {
        tracing::error!("Cannot create destination dir {:?}: {}", dst_dir, e);
        return;
    }

    let entries = match std::fs::read_dir(src_dir) {
        Ok(it) => it,
        Err(e) => {
            tracing::error!("Cannot read {:?}: {}", src_dir, e);
            return;
        }
    };

    let mut moved = 0usize;
    let mut failed = 0usize;

    for entry in entries.flatten() {
        let src = entry.path();
        if !src.is_dir() {
            continue;
        }
        let Some(name) = src.file_name() else { continue };
        let dst = dst_dir.join(name);
        match replace_dir(&src, &dst) {
            Ok(()) => {
                moved += 1;
                tracing::info!("Moved mod {:?} → {:?}", src, dst);
            }
            Err(e) => {
                failed += 1;
                tracing::error!("Cannot install mod {:?} → {:?}: {}", src, dst, e);
            }
        }
    }

    tracing::info!("Moved {} mod(s) to {:?}, {} failed", moved, dst_dir, failed);
}

/// Ставит `src` на место `dst` так, что при любой ошибке `dst` остаётся
/// прежним.
///
/// Раньше старая папка удалялась до копирования новой: если копирование
/// падало (другой диск, кончилось место, права), мод пропадал совсем —
/// на это же жалуются пользователи RimSort. Теперь новая версия сначала
/// целиком собирается рядом, и лишь потом папки меняются местами.
pub fn replace_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    let parent = dst.parent().unwrap_or(Path::new("."));
    let name = dst.file_name().unwrap_or_default().to_string_lossy();
    // Точка в начале — чтобы недоделанная папка после аварии не
    // выглядела модом.
    let staging = parent.join(format!(".rust-rim-new-{name}"));
    let old = parent.join(format!(".rust-rim-old-{name}"));
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&old);

    // 1. Новая версия целиком рядом с назначением.
    let copied = if std::fs::rename(src, &staging).is_ok() {
        false
    } else {
        // Разные файловые системы — копируем.
        if let Err(e) = copy_dir_recursive(src, &staging) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
        true
    };

    // 2. Старую версию — в сторону, новую — на её место.
    let had_old = dst.exists();
    if had_old && let Err(e) = std::fs::rename(dst, &old) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&staging, dst) {
        if had_old {
            let _ = std::fs::rename(&old, dst);
        }
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }

    // 3. Уборка: её неудача мод уже не портит.
    if had_old && let Err(e) = std::fs::remove_dir_all(&old) {
        tracing::warn!("Cannot remove old version {:?}: {}", old, e);
    }
    if copied && let Err(e) = std::fs::remove_dir_all(src) {
        tracing::warn!("Copied but failed to remove source {:?}: {}", src, e);
    }
    Ok(())
}

/// Записывает файл так, что он либо прежний, либо новый целиком:
/// пишем рядом и переименовываем поверх. Обрезанный ModsConfig.xml игра
/// молча сбрасывает — вместе со всем порядком модов.
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Рекурсивно копирует директорию `src` в `dst`.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ft.is_symlink() {
            // Просто пропускаем симлинки: моды Workshop их обычно не содержат.
            continue;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Открывает папку в системном файловом менеджере.
pub fn open_in_file_manager(path: &Path) {
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rustrim_fs_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn новая_версия_мода_заменяет_старую_без_следов() {
        let dir = tmp("replace");
        let (src, dst) = (dir.join("dl/123"), dir.join("Mods/123"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("v.txt"), "new").unwrap();
        std::fs::write(dst.join("v.txt"), "old").unwrap();
        std::fs::write(dst.join("stale.txt"), "x").unwrap();

        replace_dir(&src, &dst).unwrap();

        assert_eq!(std::fs::read_to_string(dst.join("v.txt")).unwrap(), "new");
        assert!(!dst.join("stale.txt").exists());
        assert!(!src.exists());
        let leftovers: Vec<_> = std::fs::read_dir(dir.join("Mods")).unwrap().flatten()
            .map(|e| e.file_name()).collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("123")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn неудача_оставляет_старую_версию() {
        let dir = tmp("keep");
        let dst = dir.join("Mods/123");
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("v.txt"), "old").unwrap();

        // Источника нет — ни перенести, ни скопировать.
        assert!(replace_dir(&dir.join("dl/123"), &dst).is_err());
        assert_eq!(std::fs::read_to_string(dst.join("v.txt")).unwrap(), "old");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn атомарная_запись_не_оставляет_временного_файла() {
        let dir = tmp("atomic");
        let path = dir.join("ModsConfig.xml");
        std::fs::write(&path, "old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert!(!dir.join("ModsConfig.xml.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
