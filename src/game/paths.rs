//! Поиск исполняемого файла игры, wine-префиксов и папки данных RimWorld.

use std::path::{Path, PathBuf};

/// Steam AppID RimWorld — им называются папки compatdata.
pub const APP_ID: u32 = 294100;

/// Как игра установлена и чем её запускать.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Executable {
    /// Нативная сборка: бинарник вместе со своей папкой `*_Data`.
    Native(PathBuf),
    /// Windows-сборка — нужен Proton или Wine.
    Windows(PathBuf),
    /// macOS-сборка.
    MacApp(PathBuf),
}

impl Executable {
    pub fn path(&self) -> &Path {
        match self {
            Self::Native(p) | Self::Windows(p) | Self::MacApp(p) => p,
        }
    }

    /// Нужен ли слой совместимости (Proton/Wine).
    pub fn needs_compat_layer(&self) -> bool {
        matches!(self, Self::Windows(_)) && !cfg!(target_os = "windows")
    }
}

/// Ищет, чем запускать игру в папке `game_path`.
///
/// Нативная сборка распознаётся только вместе с папкой `*_Data`: рядом с
/// Windows-версией часто лежит самописный `RimWorldLinux` — скрипт-обёртка
/// над umu или wine. Запускать его как нативную игру нельзя: он тянет за
/// собой чужие настройки (вплоть до gamescope на весь экран), а для
/// автотестов нужен предсказуемый процесс.
pub fn find_executable(game_path: &Path) -> Option<Executable> {
    let native = game_path.join("RimWorldLinux");
    if native.is_file() && game_path.join("RimWorldLinux_Data").is_dir() {
        return Some(Executable::Native(native));
    }

    let mac = game_path.join("RimWorldMac.app");
    if mac.is_dir() {
        return Some(Executable::MacApp(mac));
    }

    let win = game_path.join("RimWorldWin64.exe");
    if win.is_file() && game_path.join("RimWorldWin64_Data").is_dir() {
        return Some(Executable::Windows(win));
    }

    None
}

/// Версия игры из `Version.txt` (например «1.6.4633 rev1260»).
pub fn game_version(game_path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(game_path.join("Version.txt")).ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

// ─── Папка данных RimWorld (Config/, Saves/, Player.log) ─────────────────────

/// Имена, под которыми RimWorld создаёт свою папку данных.
/// Второй вариант встречается в префиксах, созданных не Steam.
const DATA_DIR_NAMES: [&str; 2] = [
    "Ludeon Studios/RimWorld by Ludeon Studios",
    "RimWorld by Ludeon Studios",
];

/// Ищет папку данных RimWorld внутри wine-префикса.
///
/// Имя пользователя внутри префикса заранее неизвестно: Proton заводит
/// `steamuser`, обычный wine — имя из системы, а сторонние лаунчеры могут
/// использовать своё. Поэтому перебираем всех пользователей префикса.
pub fn data_dir_in_prefix(prefix: &Path) -> Option<PathBuf> {
    // Некоторые префиксы держат pfx/ симлинком на себя же.
    let roots = [prefix.join("drive_c"), prefix.join("pfx").join("drive_c")];
    for root in roots {
        let users = match std::fs::read_dir(root.join("users")) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for user in users.flatten() {
            let local_low = user.path().join("AppData").join("LocalLow");
            for name in DATA_DIR_NAMES {
                let candidate = local_low.join(name);
                if candidate.is_dir() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Папка данных нативной установки (без префикса).
pub fn native_data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let candidates = [
        home.join(".config/unity3d/Ludeon Studios/RimWorld by Ludeon Studios"),
        home.join("Library/Application Support/Ludeon Studios/RimWorld by Ludeon Studios"),
    ];
    candidates.into_iter().find(|p| p.is_dir())
}

pub fn config_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("Config")
}

pub fn player_log(data_dir: &Path) -> PathBuf {
    data_dir.join("Player.log")
}

// ─── Поиск префиксов ─────────────────────────────────────────────────────────

/// Найденный wine-префикс с игровыми данными.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefix {
    pub path: PathBuf,
    /// Папка данных RimWorld внутри него.
    pub data_dir: PathBuf,
    /// Откуда он взялся — для показа пользователю.
    pub source: String,
}

/// Ищет префиксы, в которых уже есть данные RimWorld.
///
/// Список отсортирован по свежести: первым идёт тот, в чей конфиг писали
/// последним — обычно это и есть работающая установка. У пользователя
/// вполне может быть несколько префиксов от разных лаунчеров, из которых
/// живой только один.
pub fn find_prefixes() -> Vec<Prefix> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut found = Vec::new();

    let steam_roots = [home.join(".steam/steam"), home.join(".local/share/Steam")];
    for root in steam_roots {
        let path = root.join(format!("steamapps/compatdata/{APP_ID}"));
        push_prefix(&mut found, path, "Steam (Proton)");
    }

    // Сторонние лаунчеры и самодельные префиксы держат их в своих папках,
    // причём имя папки произвольное — сканируем на один уровень вглубь.
    let scan_dirs = [
        (home.join(".config/hydralauncher/wine-prefixes"), "Hydra"),
        (home.join(".local/share/umu-prefixes"), "umu"),
        (home.join("Games/umu"), "umu"),
        (home.join(".local/share/lutris/prefixes"), "Lutris"),
        (home.join(".var/app/com.usebottles.bottles/data/bottles/bottles"), "Bottles"),
    ];
    for (dir, source) in scan_dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            push_prefix(&mut found, entry.path(), source);
        }
    }

    push_prefix(&mut found, home.join(".wine"), "Wine");

    found.sort_by_key(|p| std::cmp::Reverse(modified_at(&config_dir(&p.data_dir))));
    found
}

fn push_prefix(out: &mut Vec<Prefix>, path: PathBuf, source: &str) {
    if out.iter().any(|p| p.path == path) {
        return;
    }
    if let Some(data_dir) = data_dir_in_prefix(&path) {
        out.push(Prefix { path, data_dir, source: source.to_string() });
    }
}

fn modified_at(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("rustrim_paths_{}_{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn dir(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(&p).unwrap();
            p
        }

        fn file(&self, rel: &str, body: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
            p
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn detects_native_build() {
        let t = TempTree::new("native");
        t.file("game/RimWorldLinux", "#!/bin/sh");
        t.dir("game/RimWorldLinux_Data");

        assert_eq!(
            find_executable(&t.0.join("game")),
            Some(Executable::Native(t.0.join("game/RimWorldLinux"))),
        );
    }

    #[test]
    fn wrapper_script_without_data_is_not_native() {
        // Ровно случай DRM-free копии: рядом с Windows-сборкой лежит
        // самописный RimWorldLinux, запускающий игру через umu.
        let t = TempTree::new("wrapper");
        t.file("game/RimWorldLinux", "#!/usr/bin/env bash\numu-run ...");
        t.file("game/RimWorldWin64.exe", "MZ");
        t.dir("game/RimWorldWin64_Data");

        let found = find_executable(&t.0.join("game"));
        assert_eq!(found, Some(Executable::Windows(t.0.join("game/RimWorldWin64.exe"))));
        assert!(found.unwrap().needs_compat_layer() || cfg!(target_os = "windows"));
    }

    #[test]
    fn exe_without_data_folder_is_not_detected() {
        let t = TempTree::new("no_data");
        t.file("game/RimWorldWin64.exe", "MZ");
        assert_eq!(find_executable(&t.0.join("game")), None);
    }

    #[test]
    fn empty_folder_yields_nothing() {
        let t = TempTree::new("empty");
        assert_eq!(find_executable(&t.dir("game")), None);
    }

    #[test]
    fn reads_game_version() {
        let t = TempTree::new("version");
        t.file("game/Version.txt", "1.6.4633 rev1260\n");
        assert_eq!(game_version(&t.0.join("game")).as_deref(), Some("1.6.4633 rev1260"));
        assert_eq!(game_version(&t.0.join("nope")), None);
    }

    #[test]
    fn finds_data_dir_for_any_prefix_user() {
        // Proton заводит steamuser, обычный wine — имя из системы.
        for user in ["steamuser", "voidfox"] {
            let t = TempTree::new(&format!("prefix_{user}"));
            let data = t.dir(&format!(
                "pfx/drive_c/users/{user}/AppData/LocalLow/Ludeon Studios/RimWorld by Ludeon Studios"
            ));
            assert_eq!(data_dir_in_prefix(&t.0.join("pfx")), Some(data));
        }
    }

    #[test]
    fn finds_data_dir_without_publisher_folder() {
        // Встречается в префиксах, созданных не Steam.
        let t = TempTree::new("nopublisher");
        let data = t.dir("pfx/drive_c/users/steamuser/AppData/LocalLow/RimWorld by Ludeon Studios");
        assert_eq!(data_dir_in_prefix(&t.0.join("pfx")), Some(data));
    }

    #[test]
    fn prefix_without_game_data_is_rejected() {
        let t = TempTree::new("bare");
        t.dir("pfx/drive_c/users/steamuser/AppData/LocalLow");
        assert_eq!(data_dir_in_prefix(&t.0.join("pfx")), None);
    }

    #[test]
    fn config_and_log_hang_off_data_dir() {
        let data = Path::new("/prefix/drive_c/users/x/AppData/LocalLow/RimWorld by Ludeon Studios");
        assert_eq!(config_dir(data).file_name().unwrap(), "Config");
        assert_eq!(player_log(data).file_name().unwrap(), "Player.log");
    }
}
