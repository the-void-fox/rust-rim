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

        // Если в назначении уже есть папка с таким именем — удаляем,
        // чтобы получить «свежую» версию мода.
        if dst.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dst) {
                tracing::error!("Cannot replace existing {:?}: {}", dst, e);
                failed += 1;
                continue;
            }
        }

        match std::fs::rename(&src, &dst) {
            Ok(_) => {
                moved += 1;
                tracing::info!("Moved mod {:?} → {:?}", src, dst);
            }
            Err(_) => {
                // Возможно, src и dst на разных файловых системах —
                // делаем копирование + удаление.
                if let Err(e) = copy_dir_recursive(&src, &dst) {
                    tracing::error!("Failed to copy mod {:?} → {:?}: {}", src, dst, e);
                    failed += 1;
                    continue;
                }
                if let Err(e) = std::fs::remove_dir_all(&src) {
                    tracing::warn!("Copied but failed to remove source {:?}: {}", src, e);
                }
                moved += 1;
                tracing::info!("Copied mod {:?} → {:?}", src, dst);
            }
        }
    }

    tracing::info!("Moved {} mod(s) to {:?}, {} failed", moved, dst_dir, failed);
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
