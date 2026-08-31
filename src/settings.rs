//! Пользовательские настройки: пути, поведение, внешний вид.

use serde::{Deserialize, Serialize};

#[derive(PartialEq, Default, Clone, Serialize, Deserialize)]
pub enum SettingsTab {
    #[default]
    Paths,
    Interface,
    Behavior,
}

#[derive(Serialize, Deserialize)]
pub struct AppSettings {
    pub game_path: String,
    pub config_path: String,
    pub local_mods_path: String,
    pub dark_theme: bool,
    pub show_package_ids: bool,
    pub sort_on_load: bool,
    pub use_community_rules: bool,
    /// Базовая папка для SteamCMD (steamcmd/ и steam/ создаются внутри).
    /// Пустая строка → используется папка данных приложения.
    pub steamcmd_path: String,
    /// Автоматически перемещать скачанные моды в папку локальных модов после загрузки.
    pub steamcmd_auto_move: bool,
    /// Включить параллельную загрузку модов несколькими процессами SteamCMD.
    pub steamcmd_multi_download: bool,
    /// Максимальное количество параллельных процессов SteamCMD (2–4 рекомендуется).
    pub steamcmd_max_processes: usize,
    /// Минимальное число модов для активации мульти-загрузки.
    pub steamcmd_multi_threshold: usize,
    /// Путь к Player.log для анализатора логов (пустой — автопоиск).
    #[serde(default)]
    pub log_file_path: String,
    #[serde(skip)]
    pub active_tab: SettingsTab,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            game_path: String::new(),
            config_path: String::new(),
            local_mods_path: String::new(),
            dark_theme: true,
            show_package_ids: false,
            sort_on_load: false,
            use_community_rules: true,
            steamcmd_path: String::new(),
            steamcmd_auto_move: true,
            steamcmd_multi_download: true,
            steamcmd_max_processes: 2,
            steamcmd_multi_threshold: 10,
            log_file_path: String::new(),
            active_tab: SettingsTab::default(),
        }
    }
}

impl AppSettings {
    /// Возвращает эффективный путь для SteamCMD:
    /// пользовательский путь если задан, иначе — папка данных приложения.
    pub fn effective_steamcmd_path(&self) -> String {
        if !self.steamcmd_path.is_empty() {
            return self.steamcmd_path.clone();
        }
        directories::ProjectDirs::from("com", "rustrim", "RustRim")
            .map(|d| d.data_dir().join("steamcmd_data").to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Заданы ли пути, без которых приложению нечего показывать.
    pub fn has_required_paths(&self) -> bool {
        !self.game_path.is_empty() && !self.local_mods_path.is_empty()
    }

    fn config_file_path() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("com", "rustrim", "RustRim")
            .map(|dirs| dirs.config_dir().join("settings.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_file_path() else { return Self::default() };
        let Ok(data) = std::fs::read_to_string(&path) else { return Self::default() };
        serde_json::from_str(&data).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::config_file_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, data);
        }
    }
}
