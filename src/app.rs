use egui::{Align2, Color32, FontId, Frame, Margin, RichText, Sense, Stroke, StrokeKind, Vec2};
use serde::{Deserialize, Serialize};
use crate::mod_data::{ModEntry, ModSource, scan_local_mods, scan_dlc_mods, parse_mods_config, write_mods_config, write_mod_list};
use crate::sorting::CommunityRules;
use crate::ui::{toolbar, mod_list::ModList, dialogs};
use crate::ui::steamcmd_panel::SteamCmdPanel;
use crate::ui::workshop_browser::WorkshopBrowser;
use crate::ui::log_panel::LogPanel;
use crate::steam::steamcmd;

// ─── Цветовая палитра ────────────────────────────────────────────────────────
pub mod theme {
    use egui::Color32;

    pub const BG_DARK:       Color32 = Color32::from_rgb(18, 20, 24);
    pub const BG_PANEL:      Color32 = Color32::from_rgb(25, 28, 34);
    pub const BG_HEADER:     Color32 = Color32::from_rgb(30, 33, 41);
    pub const BG_ROW_EVEN:   Color32 = Color32::from_rgb(28, 31, 38);
    pub const BG_ROW_ODD:    Color32 = Color32::from_rgb(32, 36, 44);
    pub const BG_ROW_HOVER:  Color32 = Color32::from_rgb(40, 46, 58);
    pub const BG_SELECTED:   Color32 = Color32::from_rgb(45, 85, 130);

    pub const BORDER:        Color32 = Color32::from_rgb(45, 50, 62);
    pub const BORDER_ACCENT: Color32 = Color32::from_rgb(70, 130, 200);

    pub const TEXT_PRIMARY:  Color32 = Color32::from_rgb(210, 215, 225);
    pub const TEXT_MUTED:    Color32 = Color32::from_rgb(120, 130, 148);
    pub const TEXT_ACCENT:   Color32 = Color32::from_rgb(100, 170, 255);

    pub const ACTIVE_GREEN:  Color32 = Color32::from_rgb(80, 200, 120);
    pub const WARNING_AMBER: Color32 = Color32::from_rgb(240, 180, 60);
    pub const ERROR_RED:     Color32 = Color32::from_rgb(220, 75, 75);

    pub const SOURCE_LOCAL:    Color32 = Color32::from_rgb(140, 160, 185);
    pub const SOURCE_WORKSHOP: Color32 = Color32::from_rgb(100, 160, 240);
    pub const SOURCE_DLC:      Color32 = Color32::from_rgb(180, 130, 240);
    pub const SOURCE_CORE:     Color32 = Color32::from_rgb(240, 190, 80);

    pub const HEADER_LEFT:  Color32 = Color32::from_rgb(60, 100, 170);
    pub const HEADER_RIGHT: Color32 = Color32::from_rgb(60, 150, 100);
}

// ─── Настройки приложения ────────────────────────────────────────────────────
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
}

impl AppSettings {
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

// ─── Payload для Drag & Drop ─────────────────────────────────────────────────
#[derive(Clone, Debug)]
pub struct DragPayload {
    pub orig_idx: usize,
}

// ─── Запросы перемещения модов ───────────────────────────────────────────────
pub enum MoveRequest {
    Activate(usize),
    Deactivate(usize),
    MoveUp(usize),
    MoveDown(usize),
    /// Drag & Drop: переместить мод orig_idx в список to_active перед позицией to_pos
    DragDrop { orig_idx: usize, to_active: bool, to_pos: usize },
    OpenFolder(usize),
}

// ─── Состояние поиска ────────────────────────────────────────────────────────
#[derive(Default)]
pub struct SearchState {
    pub inactive_query: String,
    pub active_query: String,
}

// ─── Кэши списков ────────────────────────────────────────────────────────────
// Фильтрация и анализ зависимостей стоят O(mods × deps) — считать это каждый
// кадр нельзя (на 2000 модов старый код делал тысячи to_lowercase() за кадр).
// Кэш пересчитывается только когда изменился список модов (invalidate())
// или текст поиска.

/// Предвычисленные флаги предупреждений для одного мода (индекс = индекс в mods).
#[derive(Clone, Copy, Default)]
pub struct RowWarn {
    pub missing_deps: bool,
    pub incompat: bool,
}

pub struct ListCaches {
    /// Ключ поиска: lowercase "название\npackage_id" (индекс = индекс в mods).
    keys: Vec<String>,
    pub warn: Vec<RowWarn>,
    pub inactive: Vec<usize>,
    pub active: Vec<usize>,
    last_inactive_q: String,
    last_active_q: String,
    mods_dirty: bool,
}

impl Default for ListCaches {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            warn: Vec::new(),
            inactive: Vec::new(),
            active: Vec::new(),
            last_inactive_q: String::new(),
            last_active_q: String::new(),
            mods_dirty: true,
        }
    }
}

impl ListCaches {
    pub fn invalidate(&mut self) {
        self.mods_dirty = true;
    }

    pub fn refresh(&mut self, mods: &[ModEntry], search: &SearchState) {
        let mods_changed = self.mods_dirty;
        if mods_changed {
            self.mods_dirty = false;

            self.keys.clear();
            self.keys.extend(mods.iter().map(|m| {
                let mut k = m.name.to_lowercase();
                k.push('\n');
                k.push_str(&m.package_id.to_lowercase());
                k
            }));

            let active_ids: std::collections::HashSet<&str> = mods.iter()
                .filter(|m| m.is_active)
                .map(|m| m.package_id.as_str())
                .collect();
            self.warn.clear();
            self.warn.extend(mods.iter().map(|m| RowWarn {
                missing_deps: m.dependencies.iter().any(|d| !active_ids.contains(d.as_str())),
                incompat: m.is_active
                    && m.incompatible_with.iter().any(|ic| active_ids.contains(ic.as_str())),
            }));
        }

        if mods_changed || search.inactive_query != self.last_inactive_q {
            self.last_inactive_q = search.inactive_query.clone();
            let q = search.inactive_query.to_lowercase();
            self.inactive.clear();
            self.inactive.extend(
                mods.iter().enumerate()
                    .filter(|(i, m)| !m.is_active && (q.is_empty() || self.keys[*i].contains(&q)))
                    .map(|(i, _)| i),
            );
        }

        if mods_changed || search.active_query != self.last_active_q {
            self.last_active_q = search.active_query.clone();
            let q = search.active_query.to_lowercase();
            self.active.clear();
            self.active.extend(
                mods.iter().enumerate()
                    .filter(|(i, m)| m.is_active && (q.is_empty() || self.keys[*i].contains(&q)))
                    .map(|(i, _)| i),
            );
        }
    }
}

// ─── Асинхронная загрузка превью ─────────────────────────────────────────────
enum PreviewState {
    Idle,
    Loading {
        path: std::path::PathBuf,
        receiver: std::sync::mpsc::Receiver<Option<egui::ColorImage>>,
    },
    Ready {
        path: std::path::PathBuf,
        handle: egui::TextureHandle,
    },
    Failed(std::path::PathBuf),
}

impl PreviewState {
    fn path(&self) -> Option<&std::path::Path> {
        match self {
            Self::Idle => None,
            Self::Loading { path, .. } | Self::Ready { path, .. } => Some(path),
            Self::Failed(path) => Some(path),
        }
    }

    fn texture(&self) -> Option<&egui::TextureHandle> {
        if let Self::Ready { handle, .. } = self { Some(handle) } else { None }
    }
}

// ─── Основное состояние приложения ───────────────────────────────────────────
pub struct RustRim {
    pub mods: Vec<ModEntry>,
    /// Индекс выбранного мода в Vec<ModEntry>; единое выделение для обоих списков
    pub selected: Option<usize>,
    pub search: SearchState,
    pub settings: AppSettings,

    show_open_dialog: bool,
    show_save_dialog: bool,
    show_settings_dialog: bool,

    preview_state: PreviewState,

    /// Закешированные правила сообщества (загружаются при первой сортировке).
    community_rules: Option<CommunityRules>,

    /// Дубликаты модов: (package_id, список индексов в self.mods).
    duplicates: Vec<(String, Vec<usize>)>,
    show_duplicates_dialog: bool,
    confirm_remove_duplicates: bool,
    /// Сколько дубликатов было удалено последний раз (для уведомления).
    last_removed_count: usize,

    /// Панель загрузки модов через SteamCMD.
    steamcmd_panel: SteamCmdPanel,
    show_steamcmd_panel: bool,
    /// Браузер Steam Workshop.
    workshop_browser: WorkshopBrowser,
    show_workshop_browser: bool,

    /// Анализатор логов RimWorld.
    log_panel: LogPanel,
    show_log_panel: bool,

    /// Кэш Markdown-рендерера описаний модов.
    md_cache: egui_commonmark::CommonMarkCache,

    /// Кэши фильтрации/предупреждений списков (см. ListCaches).
    caches: ListCaches,
}

impl Default for RustRim {
    fn default() -> Self {
        Self::new()
    }
}

impl RustRim {
    pub fn new() -> Self {
        let settings = AppSettings::load();
        let has_paths = !settings.game_path.is_empty() && !settings.local_mods_path.is_empty();
        let mut app = Self {
            mods: Vec::new(),
            selected: None,
            search: SearchState::default(),
            show_open_dialog: !has_paths,
            show_save_dialog: false,
            show_settings_dialog: false,
            preview_state: PreviewState::Idle,
            community_rules: None,
            duplicates: Vec::new(),
            show_duplicates_dialog: false,
            confirm_remove_duplicates: false,
            last_removed_count: 0,
            steamcmd_panel: SteamCmdPanel::new(),
            show_steamcmd_panel: false,
            workshop_browser: WorkshopBrowser::new(),
            show_workshop_browser: false,
            log_panel: LogPanel::new(),
            show_log_panel: false,
            md_cache: egui_commonmark::CommonMarkCache::default(),
            caches: ListCaches::default(),
            settings,
        };
        if has_paths {
            app.load_local_mods();
        }
        app
    }

    fn add_missing_dependencies(&mut self) -> usize {
        let mut activated = 0;
        let mut queue: Vec<usize> = self.mods.iter()
            .enumerate()
            .filter(|(_, m)| m.is_active)
            .map(|(i, _)| i)
            .collect();
        let mut visited = std::collections::HashSet::new();

        while let Some(idx) = queue.pop() {
            if !visited.insert(idx) {
                continue;
            }
            let deps = self.mods[idx].dependencies.clone();
            for dep_id in deps {
                // Ищем мод с таким package_id (регистронезависимо)
                if let Some(dep_idx) = self.mods.iter().position(|m| m.package_id.eq_ignore_ascii_case(&dep_id)) {
                    if !self.mods[dep_idx].is_active {
                        self.mods[dep_idx].is_active = true;
                        activated += 1;
                        queue.push(dep_idx);
                    }
                } else {
                    tracing::warn!(
                        "Missing dependency '{}' for mod '{}'",
                        dep_id,
                        self.mods[idx].name
                    );
                }
            }
        }
        if activated > 0 {
            self.caches.invalidate();
        }
        activated
    }

}


impl eframe::App for RustRim {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        apply_theme(&ctx);

        // ── Тулбар ──────────────────────────────────────────────────────────
        let toolbar_resp = egui::Panel::top("toolbar_panel")
            .frame(Frame::NONE.fill(theme::BG_HEADER).inner_margin(Margin::symmetric(8, 6)))
            .show(ui, |ui| toolbar::show_toolbar(ui, &self.mods))
            .inner;

        if toolbar_resp.save_clicked     { self.show_save_dialog = true; }
        if toolbar_resp.sort_clicked     { self.sort_active_mods(); }
        if toolbar_resp.settings_clicked { self.show_settings_dialog = true; }
        if toolbar_resp.activate_all     { self.activate_all(); }
        if toolbar_resp.deactivate_all   { self.deactivate_all(); }
        if toolbar_resp.save_list_clicked { self.export_mod_list(); }
        if toolbar_resp.load_list_clicked { self.import_mod_list(); }
        if toolbar_resp.steamcmd_clicked  { self.show_steamcmd_panel = true; }
        if toolbar_resp.workshop_clicked   { self.show_workshop_browser = true; }
        if toolbar_resp.logs_clicked       { self.show_log_panel = true; }

        // ── Строка состояния ─────────────────────────────────────────────────
        egui::Panel::bottom("status_bar")
            .frame(Frame::NONE.fill(theme::BG_HEADER).inner_margin(Margin::symmetric(10, 4)))
            .show(ui, |ui| {
                let active_count = self.mods.iter().filter(|m| m.is_active).count();
                let total = self.mods.len();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!(
                        "Активных: {}  •  Всего: {}",
                        active_count, total
                    )).color(theme::TEXT_MUTED).size(11.5));
                    let warnings = self.count_warnings();
                    if warnings > 0 {
                        ui.separator();
                        ui.label(RichText::new(format!("⚠ {}", warnings))
                            .color(theme::WARNING_AMBER).size(11.5));
                    }
                });
            });

        // ── Индексы списков из кэша (пересчёт только при изменениях) ─────────
        self.caches.refresh(&self.mods, &self.search);
        let inactive_indices = self.caches.inactive.clone();
        let active_indices   = self.caches.active.clone();

        // ── Клавиатурная навигация (стрелки / Enter) ─────────────────────────
        let mut pending_req: Option<MoveRequest> = self.handle_keyboard_nav(&ctx, &inactive_indices, &active_indices);

        // ── Правая панель: информация о моде ─────────────────────────────────
        let selected_mod_idx = self.selected;
        egui::Panel::right("details_panel")
            .min_size(240.0)
            .default_size(300.0)
            .max_size(500.0)
            .resizable(true)
            .frame(
                Frame::NONE
                    .fill(theme::BG_PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER_ACCENT))
                    .inner_margin(Margin::symmetric(10, 5))
            )
            .show(ui, |ui| {
                Frame::NONE
                    .fill(theme::BG_HEADER)
                    .inner_margin(Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("ИНФОРМАЦИЯ О МОДЕ")
                            .color(theme::TEXT_MUTED).size(11.0).strong());
                    });
                // Запускаем фоновую загрузку превью если выделение изменилось.
                let preview_path = selected_mod_idx
                    .and_then(|i| self.mods.get(i))
                    .and_then(|m| m.preview_path.clone());

                let state_path = self.preview_state.path().map(|p| p.to_path_buf());
                if state_path.as_deref() != preview_path.as_deref() {
                    if let Some(path) = preview_path.clone() {
                        let (tx, rx) = std::sync::mpsc::channel();
                        let path_clone = path.clone();
                        std::thread::spawn(move || {
                            let img = std::fs::read(&path_clone)
                                .ok()
                                .and_then(|b| image::load_from_memory(&b).ok())
                                .map(|img| {
                                    let rgba = img.to_rgba8();
                                    let (w, h) = rgba.dimensions();
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [w as usize, h as usize], rgba.as_raw(),
                                    )
                                });
                            let _ = tx.send(img);
                        });
                        self.preview_state = PreviewState::Loading { path, receiver: rx };
                    } else {
                        self.preview_state = PreviewState::Idle;
                    }
                }

                // Проверяем, не загрузил ли фоновый поток изображение.
                let mut new_state: Option<PreviewState> = None;
                if let PreviewState::Loading { path, receiver } = &self.preview_state {
                    match receiver.try_recv() {
                        Ok(Some(ci)) => {
                            let handle = ctx.load_texture(
                                "mod_preview", ci, egui::TextureOptions::LINEAR,
                            );
                            new_state = Some(PreviewState::Ready { path: path.clone(), handle });
                        }
                        Ok(None) => {
                            new_state = Some(PreviewState::Failed(path.clone()));
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            ctx.request_repaint_after(std::time::Duration::from_millis(16));
                        }
                        Err(_) => {
                            new_state = Some(PreviewState::Failed(path.clone()));
                        }
                    }
                }
                if let Some(state) = new_state {
                    self.preview_state = state;
                }

                let selected_mod = selected_mod_idx.and_then(|i| self.mods.get(i));
                let preview_tex  = self.preview_state.texture();
                show_mod_details(ui, selected_mod, preview_tex, &mut self.md_cache);
            });

        // ── Центральная область: два списка ──────────────────────────────────
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(theme::BG_DARK))
            .show(ui, |ui| {
                ui.columns(2, |cols| {
                // Левая колонка — неактивные
                Frame::NONE
                    .fill(theme::BG_PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .show(&mut cols[0], |ui| {
                        show_panel_header(ui, "НЕАКТИВНЫЕ МОДЫ", theme::HEADER_LEFT, false,
                            self.mods.iter().filter(|m| !m.is_active).count());
                        ui.add_space(2.0);
                        show_search_bar(ui, &mut self.search.inactive_query, "inactive_search");
                        ui.add_space(2.0);
                        if let Some(req) = ModList::new(&self.mods, &inactive_indices, &self.caches.warn, &mut self.selected, false).show(ui) {
                            pending_req = Some(req);
                        }
                    });

                // Правая колонка — активные
                Frame::NONE
                    .fill(theme::BG_PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .show(&mut cols[1], |ui| {
                        show_panel_header(ui, "АКТИВНЫЕ МОДЫ", theme::HEADER_RIGHT, true,
                            self.mods.iter().filter(|m| m.is_active).count());
                        ui.add_space(2.0);
                        show_search_bar(ui, &mut self.search.active_query, "active_search");
                        ui.add_space(2.0);
                        if let Some(req) = ModList::new(&self.mods, &active_indices, &self.caches.warn, &mut self.selected, true).show(ui) {
                            pending_req = Some(req);
                        }
                    });
            });
        });

        if let Some(req) = pending_req {
            self.handle_move_request(req);
        }

        // ── Drag ghost (отображается поверх всего) ───────────────────────────
        if let Some(payload) = egui::DragAndDrop::payload::<DragPayload>(&ctx) {
            if let Some(cursor) = ctx.pointer_latest_pos() {
                let mod_name = self.mods.get(payload.orig_idx)
                    .map(|m| m.name.as_str())
                    .unwrap_or("...");
                egui::Area::new(egui::Id::new("drag_ghost"))
                    .fixed_pos(cursor + Vec2::new(14.0, -10.0))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(&ctx, |ui| {
                        Frame::NONE
                            .fill(theme::BG_SELECTED)
                            .inner_margin(Margin::symmetric(10, 5))
                            .stroke(Stroke::new(1.0, theme::BORDER_ACCENT))
                            .show(ui, |ui| {
                                ui.label(RichText::new(mod_name).color(Color32::WHITE).size(12.0));
                            });
                    });
            }
        }
        // Если мышь отпущена вне любого списка — сбрасываем payload
        if egui::DragAndDrop::has_any_payload(&ctx) && ctx.input(|i| i.pointer.primary_released()) {
            egui::DragAndDrop::clear_payload(&ctx);
        }

        // ── Диалоги ──────────────────────────────────────────────────────────
        if self.show_open_dialog {
            if dialogs::open_folder_dialog(&ctx, &mut self.show_open_dialog, &mut self.settings) {
                self.settings.save();
                self.load_local_mods();
            }
        }
        if self.show_save_dialog {
            let config_path = self.settings.config_path.clone();
            if dialogs::save_dialog(&ctx, &mut self.show_save_dialog, &self.mods, &config_path) {
                self.save_mods_config();
            }
        }
        if self.show_settings_dialog {
            if dialogs::settings_dialog(&ctx, &mut self.show_settings_dialog, &mut self.settings) {
                self.settings.save();
                self.load_local_mods();
            }
        }

        // ── Панель SteamCMD ──────────────────────────────────────────────────
        if self.show_steamcmd_panel {
            let base = self.settings.effective_steamcmd_path();
            if self.steamcmd_panel.show(
                &ctx,
                &mut self.show_steamcmd_panel,
                &base,
                self.settings.steamcmd_auto_move,
                self.settings.steamcmd_multi_download,
                self.settings.steamcmd_max_processes,
                self.settings.steamcmd_multi_threshold,
            ) {
                // Переносим скачанные моды в RimWorld/Mods,
                // чтобы они лежали как обычные локальные моды.
                let sc_base = std::path::Path::new(&base);
                let src_dir = steamcmd::steam_content_path(sc_base);
                let dst_dir = std::path::Path::new(&self.settings.local_mods_path);
                if !self.settings.local_mods_path.is_empty() && src_dir.is_dir() {
                    move_downloaded_mods(&src_dir, dst_dir);
                }
                self.load_local_mods();
            }
        }

        // ── Браузер Steam Workshop ────────────────────────────────────────────
        if self.show_workshop_browser {
            let installed_ids: std::collections::HashSet<u64> = self.mods.iter()
                .filter_map(|m| if let ModSource::Workshop(id) = m.source { Some(id) } else { None })
                .collect();
            if let Some(ids) = self.workshop_browser.show(&ctx, &mut self.show_workshop_browser, &installed_ids) {
                self.steamcmd_panel.add_ids(&ids);
                self.show_steamcmd_panel = true;
            }
        }

        // ── Анализ логов ─────────────────────────────────────────────────────
        if self.show_log_panel {
            let path_before = self.settings.log_file_path.clone();
            let picked = self.log_panel.show(
                &ctx, &mut self.show_log_panel, &self.mods, &mut self.settings.log_file_path);
            if self.settings.log_file_path != path_before {
                self.settings.save();
            }
            // Клик по подозреваемому — выделяем мод в списке
            if let Some(pid) = picked {
                if let Some(i) = self.mods.iter().position(|m| m.package_id == pid) {
                    self.selected = Some(i);
                }
            }
        }

        // ── Диалог дубликатов ────────────────────────────────────────────────
        if self.show_duplicates_dialog {
            let mut open = true;
            egui::Window::new("Обнаружены дубликаты модов")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    ui.label(RichText::new(format!(
                        "Найдено {} мод(ов) с дублирующимися package ID:",
                        self.duplicates.len()
                    )).color(theme::WARNING_AMBER));
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                        for (id, indices) in &self.duplicates {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("×{}", indices.len()))
                                    .color(theme::ERROR_RED).size(11.0).strong());
                                ui.add_space(4.0);
                                ui.label(RichText::new(id)
                                    .color(theme::TEXT_ACCENT).size(11.0));
                            });
                        }
                    });
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Удалить дубликаты (оставить первый)").clicked() {
                            self.show_duplicates_dialog = false;
                            self.confirm_remove_duplicates = true;
                        }
                        ui.add_space(8.0);
                        if ui.button("Закрыть").clicked() {
                            self.show_duplicates_dialog = false;
                        }
                    });
                });
            if !open {
                self.show_duplicates_dialog = false;
            }
        }

        // ── Уведомление об удалении дубликатов ───────────────────────────────
        if self.last_removed_count > 0 {
            egui::Window::new("Готово")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    ui.label(RichText::new(format!(
                        "Удалено {} дублирующихся мод(ов).",
                        self.last_removed_count
                    )).color(theme::ACTIVE_GREEN));
                    ui.add_space(6.0);
                    if ui.button("OK").clicked() {
                        self.last_removed_count = 0;
                    }
                });
        }

        if self.confirm_remove_duplicates {
            egui::Window::new("Подтверждение удаления")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    ui.label(RichText::new("ВНИМАНИЕ!").color(theme::ERROR_RED).strong());
                    ui.label("Вы собираетесь безвозвратно удалить папки дублирующихся модов с диска.");
                    ui.label("Отменить это действие будет невозможно.");
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Да, удалить").clicked() {
                            self.remove_duplicates();
                            self.confirm_remove_duplicates = false;
                        }
                        if ui.button("Отмена").clicked() {
                            self.confirm_remove_duplicates = false;
                        }
                    });
                });
        }
    }
}

impl RustRim {
    fn load_local_mods(&mut self) {
        if self.settings.local_mods_path.is_empty() {
            return;
        }
        let mut all_mods = Vec::new();

        // Core + DLC из папки Data/ игры идут первыми
        if !self.settings.game_path.is_empty() {
            let game_path = std::path::Path::new(&self.settings.game_path);
            all_mods.extend(scan_dlc_mods(game_path));
        }

        // Локальные и Workshop моды
        let mods_path = std::path::Path::new(&self.settings.local_mods_path);
        all_mods.extend(scan_local_mods(mods_path));

        // Моды скачанные через SteamCMD (если папка существует)
        let sc_base = self.settings.effective_steamcmd_path();
        if !sc_base.is_empty() {
            let content = steamcmd::steam_content_path(std::path::Path::new(&sc_base));
            if content.is_dir() {
                all_mods.extend(scan_local_mods(&content));
            }
        }

        self.mods = all_mods;
        self.selected = None;
        self.preview_state = PreviewState::Idle;
        self.caches.invalidate();
        self.apply_mods_config();
        self.check_duplicates();
    }

    fn check_duplicates(&mut self) {
        let mut seen: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
        for (i, m) in self.mods.iter().enumerate() {
            seen.entry(m.package_id.to_lowercase()).or_default().push(i);
        }
        self.duplicates = seen.into_iter()
            .filter(|(_, idxs)| idxs.len() > 1)
            .collect();
        self.duplicates.sort_by(|a, b| a.0.cmp(&b.0));
        self.show_duplicates_dialog = !self.duplicates.is_empty();
    }

    fn remove_duplicates(&mut self) {
        let mut to_remove_indices = std::collections::HashSet::new();
        for (_, indices) in &self.duplicates {
            if indices.len() <= 1 { continue; }
            // Оставляем первый индекс, удаляем остальные
            for &idx in &indices[1..] {
                to_remove_indices.insert(idx);
            }
        }

        let mut sorted: Vec<usize> = to_remove_indices.into_iter().collect();
        sorted.sort_unstable_by(|a, b| b.cmp(a));

        let mut removed_count = 0;
        let mut actually_remove: Vec<usize> = Vec::new();
        for &idx in &sorted {
            if idx >= self.mods.len() { continue; }
            let m = &self.mods[idx];
            // Пропускаем Core и DLC (на всякий случай)
            if matches!(m.source, ModSource::Core | ModSource::DLC(_)) {
                tracing::warn!("Skipping deletion of core/dlc mod at {:?}", m.path);
                continue;
            }
            let disk_ok = if m.path.exists() {
                match std::fs::remove_dir_all(&m.path) {
                    Ok(_) => {
                        tracing::info!("Deleted duplicate mod folder: {:?}", m.path);
                        true
                    }
                    Err(e) => {
                        tracing::error!("Failed to delete mod folder {:?}: {}", m.path, e);
                        false
                    }
                }
            } else {
                tracing::warn!("Mod folder does not exist: {:?}", m.path);
                true // папки нет — из списка тоже убираем
            };
            if disk_ok {
                removed_count += 1;
                actually_remove.push(idx);
            }
        }

        // Удаляем из self.mods только те записи, где диск был успешно очищен
        actually_remove.sort_unstable_by(|a, b| b.cmp(a));
        for idx in actually_remove {
            if idx < self.mods.len() {
                self.mods.remove(idx);
            }
        }

        self.last_removed_count = removed_count;
        self.selected = None;
        self.duplicates.clear();
        self.caches.invalidate();
    }

    /// Читает ModsConfig.xml и помечает соответствующие моды активными в правильном порядке.
    fn apply_mods_config(&mut self) {
        if self.settings.config_path.is_empty() {
            // Нет конфига — активируем только Core
            self.activate_core_only();
            return;
        }
        let xml = std::path::Path::new(&self.settings.config_path).join("ModsConfig.xml");
        if !xml.exists() {
            self.activate_core_only();
            return;
        }
        let active_ids = match parse_mods_config(&xml) {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!("Failed to read ModsConfig.xml: {}", e);
                self.activate_core_only();
                return;
            }
        };

        self.apply_active_ids(&active_ids);
    }

    /// Активирует моды по переданному списку package_id (нижний регистр).
    /// Сортирует self.mods: активные в порядке списка, неактивные после.
    fn apply_active_ids(&mut self, active_ids: &[String]) {
        let order_map: std::collections::HashMap<String, usize> = active_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.to_lowercase(), i))
            .collect();

        for m in &mut self.mods {
            // Core всегда активен
            if m.source == ModSource::Core {
                m.is_active = true;
            } else {
                let by_pkg = order_map.contains_key(&m.package_id.to_lowercase());
                // Также проверяем совпадение по Workshop ID (для файлов-списков сборок)
                let by_wid = match &m.source {
                    ModSource::Workshop(wid) => order_map.contains_key(&wid.to_string()),
                    _ => false,
                };
                m.is_active = by_pkg || by_wid;
            }
        }

        // Сортируем self.mods: активные в порядке конфига, неактивные после.
        // Core получает позицию из order_map (обычно 0 или 2), или идёт первым если не в списке.
        self.mods.sort_by(|a, b| {
            let ka = a.package_id.to_lowercase();
            let kb = b.package_id.to_lowercase();
            let wid_key = |src: &ModSource| -> Option<String> {
                if let ModSource::Workshop(wid) = src { Some(wid.to_string()) } else { None }
            };
            let pos_a = if a.source == ModSource::Core && !order_map.contains_key(&ka) {
                Some(0) // Core без позиции в конфиге идёт первым среди активных
            } else {
                order_map.get(&ka).copied()
                    .or_else(|| wid_key(&a.source).as_deref().and_then(|k| order_map.get(k).copied()))
            };
            let pos_b = if b.source == ModSource::Core && !order_map.contains_key(&kb) {
                Some(0)
            } else {
                order_map.get(&kb).copied()
                    .or_else(|| wid_key(&b.source).as_deref().and_then(|k| order_map.get(k).copied()))
            };
            match (pos_a, pos_b) {
                (Some(oa), Some(ob)) => oa.cmp(&ob),
                (Some(_), None)      => std::cmp::Ordering::Less,
                (None, Some(_))      => std::cmp::Ordering::Greater,
                (None, None)         => std::cmp::Ordering::Equal,
            }
        });

        self.selected = None;
        self.caches.invalidate();
    }

    /// Активирует только Core, все остальные деактивирует.
    fn activate_core_only(&mut self) {
        for m in &mut self.mods {
            m.is_active = m.source == ModSource::Core;
        }
        self.mods.sort_by(|a, b| {
            match (a.source == ModSource::Core, b.source == ModSource::Core) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _             => std::cmp::Ordering::Equal,
            }
        });
        self.selected = None;
        self.caches.invalidate();
    }

    /// Записывает текущий порядок активных модов в ModsConfig.xml.
    fn save_mods_config(&mut self) {
        if self.settings.config_path.is_empty() {
            tracing::warn!("config_path is not set, cannot save ModsConfig.xml");
            return;
        }
        let xml = std::path::Path::new(&self.settings.config_path).join("ModsConfig.xml");
        // Дедупликация: если два мода имеют одинаковый package_id — берём первый.
        let mut seen = std::collections::HashSet::new();
        let active_ids: Vec<String> = self.mods.iter()
            .filter(|m| m.is_active && seen.insert(m.package_id.clone()))
            .map(|m| m.package_id.clone())
            .collect();
        if let Err(e) = write_mods_config(&xml, &active_ids) {
            tracing::error!("Failed to write ModsConfig.xml: {}", e);
        } else {
            tracing::info!("Saved {} active mods to {:?}", active_ids.len(), xml);
        }
    }

    /// Экспортирует текущий список активных модов в XML-файл (совместимо с RimSort).
    fn export_mod_list(&mut self) {
        let Some(path) = crate::ui::dialogs::pick_save_file("Сохранить список модов") else { return };
        let mut seen = std::collections::HashSet::new();
        let active_ids: Vec<String> = self.mods.iter()
            .filter(|m| m.is_active && seen.insert(m.package_id.clone()))
            .map(|m| m.package_id.clone())
            .collect();
        if let Err(e) = write_mod_list(&path, &active_ids) {
            tracing::error!("Failed to export mod list: {}", e);
        } else {
            tracing::info!("Exported {} mods to {:?}", active_ids.len(), path);
        }
    }

    /// Импортирует список активных модов из XML-файла (совместимо с RimSort/ModsConfig.xml/.rml).
    fn import_mod_list(&mut self) {
        let Some(path) = crate::ui::dialogs::pick_open_file("Загрузить список модов") else { return };
        let active_ids = match parse_mods_config(&path) {
            Ok(ids) => ids,
            Err(e) => {
                tracing::error!("Failed to import mod list from {:?}: {}", path, e);
                return;
            }
        };
        tracing::info!("Imported mod list with {} entries from {:?}", active_ids.len(), path);
        self.apply_active_ids(&active_ids);
    }

    pub fn sort_active_mods(&mut self) {
        // Добавляем недостающие зависимости
        let added = self.add_missing_dependencies();
        if added > 0 {
            tracing::info!("Automatically activated {} missing dependencies", added);
            // Можно показать всплывающее уведомление (опционально)
        }

        // Загружаем community rules при первом использовании (если включено)
        if self.settings.use_community_rules && self.community_rules.is_none() {
            match crate::sorting::fetch_community_rules() {
                Ok(rules) => {
                    tracing::info!("Community rules loaded (ts={})", rules.timestamp);
                    self.community_rules = Some(rules);
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch community rules: {}", e);
                }
            }
        }

        let rules = if self.settings.use_community_rules {
            self.community_rules.as_ref()
        } else {
            None
        };

        crate::sorting::sort_active_mods(&mut self.mods, rules);
        self.caches.invalidate();
    }

    fn activate_all(&mut self) {
        for m in &mut self.mods { m.is_active = true; }
        self.caches.invalidate();
    }

    fn deactivate_all(&mut self) {
        for m in &mut self.mods {
            if m.source != ModSource::Core { m.is_active = false; }
        }
        self.caches.invalidate();
    }

    fn count_warnings(&self) -> usize {
        // Из кэша: warn[i] соответствует mods[i] (refresh() уже вызван в ui()).
        self.mods.iter().zip(self.caches.warn.iter())
            .filter(|(m, w)| m.is_active && w.missing_deps)
            .count()
    }

    /// Перемещает мод (orig_idx) на позицию to_pos в порядке активных модов.
    fn move_active_mod_to_position(&mut self, orig_idx: usize, to_pos: usize) {
        // Индексы в self.mods, где лежат активные моды (по порядку)
        let positions: Vec<usize> = self.mods.iter().enumerate()
            .filter(|(_, m)| m.is_active)
            .map(|(i, _)| i)
            .collect();

        let from_pos = match positions.iter().position(|&i| i == orig_idx) {
            Some(p) => p,
            None => return,
        };

        let to_pos = to_pos.min(positions.len().saturating_sub(1));
        if from_pos == to_pos { return; }

        let mut active_mods: Vec<ModEntry> = positions.iter().map(|&i| self.mods[i].clone()).collect();
        let entry = active_mods.remove(from_pos);
        active_mods.insert(to_pos, entry);

        // Записываем обратно на те же слоты (positions не меняется)
        for (pos, entry) in positions.iter().zip(active_mods.into_iter()) {
            self.mods[*pos] = entry;
        }
    }

    /// Обрабатывает стрелки и Enter.
    /// - ↑ / ↓ — двигает выделение внутри того списка, в котором сейчас выделенный мод
    /// - Enter — перемещает выделенный мод в другой список (активировать / деактивировать)
    /// - Ctrl+↑ / Ctrl+↓ — меняет позицию активного мода в порядке загрузки
    /// Ввод игнорируется, если пользователь сейчас печатает в TextEdit
    /// (например, в поле поиска), чтобы не перехватывать стрелки в тексте.
    fn handle_keyboard_nav(
        &mut self,
        ctx: &egui::Context,
        inactive_indices: &[usize],
        active_indices: &[usize],
    ) -> Option<MoveRequest> {
        // Если фокус в текстовом поле — не перехватываем стрелки/Enter.
        if ctx.memory(|m| m.focused().is_some()) {
            return None;
        }

        // Определяем, в каком списке сейчас выделенный мод.
        let (in_active_list, list): (bool, &[usize]) = match self.selected {
            Some(sel) => {
                if let Some(m) = self.mods.get(sel) {
                    if m.is_active { (true, active_indices) } else { (false, inactive_indices) }
                } else {
                    // selected указывает в никуда — сбрасываем и выходим
                    self.selected = None;
                    return None;
                }
            }
            None => {
                // Ничего не выделено: на первое же нажатие стрелки — выделяем первый элемент.
                let pressed_down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));
                let pressed_up   = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
                if pressed_down || pressed_up {
                    if let Some(&first) = inactive_indices.first().or_else(|| active_indices.first()) {
                        self.selected = Some(first);
                    }
                }
                return None;
            }
        };

        // Позиция выделенного мода в своём списке (после фильтрации по поиску).
        let sel_orig = self.selected.unwrap();
        let pos_in_list = list.iter().position(|&i| i == sel_orig);

        let (up, down, enter, ctrl) = ctx.input(|i| (
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::Enter),
            i.modifiers.ctrl || i.modifiers.command,
        ));

        // Ctrl+↑ / Ctrl+↓ — переупорядочивание в списке активных.
        if ctrl && in_active_list {
            if up   { return Some(MoveRequest::MoveUp(sel_orig)); }
            if down { return Some(MoveRequest::MoveDown(sel_orig)); }
        }

        // Обычные стрелки — сдвиг выделения внутри списка.
        if up || down {
            if list.is_empty() { return None; }
            let new_pos = match pos_in_list {
                Some(p) => {
                    if up   { p.saturating_sub(1) }
                    else    { (p + 1).min(list.len() - 1) }
                }
                None => 0,
            };
            self.selected = Some(list[new_pos]);
            return None;
        }

        // Enter — переносим мод в противоположный список.
        if enter {
            if in_active_list {
                // Деактивировать нельзя только Core.
                if self.mods.get(sel_orig).map(|m| m.source != ModSource::Core).unwrap_or(false) {
                    return Some(MoveRequest::Deactivate(sel_orig));
                }
            } else {
                return Some(MoveRequest::Activate(sel_orig));
            }
        }

        None
    }

    fn handle_move_request(&mut self, req: MoveRequest) {
        self.caches.invalidate();
        match req {
            MoveRequest::Activate(orig_idx) => {
                if orig_idx < self.mods.len() { self.mods[orig_idx].is_active = true; }
            }
            MoveRequest::Deactivate(orig_idx) => {
                if orig_idx < self.mods.len() && self.mods[orig_idx].source != ModSource::Core {
                    self.mods[orig_idx].is_active = false;
                }
            }
            MoveRequest::MoveUp(orig_idx) => {
                let positions: Vec<usize> = self.mods.iter().enumerate()
                    .filter(|(_, m)| m.is_active).map(|(i, _)| i).collect();
                if let Some(pos) = positions.iter().position(|&i| i == orig_idx) {
                    if pos > 0 { self.mods.swap(orig_idx, positions[pos - 1]); }
                }
            }
            MoveRequest::MoveDown(orig_idx) => {
                let positions: Vec<usize> = self.mods.iter().enumerate()
                    .filter(|(_, m)| m.is_active).map(|(i, _)| i).collect();
                if let Some(pos) = positions.iter().position(|&i| i == orig_idx) {
                    if pos + 1 < positions.len() { self.mods.swap(orig_idx, positions[pos + 1]); }
                }
            }
            MoveRequest::OpenFolder(idx) => {
                if let Some(m) = self.mods.get(idx) {
                    let path = m.path.clone();
                    #[cfg(target_os = "linux")]
                    let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                    #[cfg(target_os = "windows")]
                    let _ = std::process::Command::new("explorer").arg(&path).spawn();
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open").arg(&path).spawn();
                }
            }
            MoveRequest::DragDrop { orig_idx, to_active, to_pos } => {
                if orig_idx >= self.mods.len() { return; }
                let from_active = self.mods[orig_idx].is_active;
                if from_active != to_active {
                    if to_active {
                        self.mods[orig_idx].is_active = true;
                        self.move_active_mod_to_position(orig_idx, to_pos);
                    } else if self.mods[orig_idx].source != ModSource::Core {
                        self.mods[orig_idx].is_active = false;
                    }
                } else if to_active {
                    self.move_active_mod_to_position(orig_idx, to_pos);
                }
                // Перестановка внутри неактивного списка не нужна (порядок не важен)
            }
        }
    }
}

// ─── Вспомогательные функции UI ──────────────────────────────────────────────



fn show_panel_header(ui: &mut egui::Ui, title: &str, accent: Color32, is_active: bool, count: usize) {
    Frame::NONE
        .fill(theme::BG_HEADER)
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 16.0), Sense::hover());
                ui.painter().rect_filled(rect, 1.0, accent);
                ui.add_space(6.0);
                ui.label(RichText::new(title).color(accent).size(11.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let badge_color = if is_active { theme::ACTIVE_GREEN } else { theme::TEXT_MUTED };
                    ui.label(RichText::new(format!("{}", count)).color(badge_color).size(11.0));
                    ui.label(RichText::new("●").color(badge_color).size(8.0));
                });
            });
        });
}

fn show_search_bar(ui: &mut egui::Ui, query: &mut String, id: &str) {
    Frame::NONE
        .fill(theme::BG_DARK)
        .inner_margin(Margin::symmetric(6, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🔍").size(12.0).color(theme::TEXT_MUTED));
                let edit = egui::TextEdit::singleline(query)
                    .hint_text("Поиск...")
                    .id(egui::Id::new(id))
                    .frame(Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .text_color(theme::TEXT_PRIMARY);
                ui.add(edit);
                if !query.is_empty() {
                    if ui.small_button(RichText::new("✕").color(theme::TEXT_MUTED)).clicked() {
                        query.clear();
                    }
                }
            });
        });
}

fn show_mod_details(
    ui: &mut egui::Ui,
    mod_entry: Option<&ModEntry>,
    preview_tex: Option<&egui::TextureHandle>,
    md_cache: &mut egui_commonmark::CommonMarkCache,
) {
    match mod_entry {
        None => {
            ui.add_space(ui.available_height() / 3.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Выберите мод\nдля просмотра")
                    .color(theme::TEXT_MUTED).size(12.0).italics());
            });
        }
        Some(m) => {
            egui::ScrollArea::vertical()
                .id_salt("details_scroll")
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());

                    // ── Баннер мода ──────────────────────────────────────
                    let img_w = ui.available_width();
                    let img_h = 160.0_f32.min(img_w * 0.5625); // 16:9

                    if let Some(tex) = preview_tex {
                        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        let (img_rect, _) = ui.allocate_exact_size(Vec2::new(img_w, img_h), Sense::hover());
                        // Фон под изображением
                        ui.painter().rect_filled(img_rect, 4.0, theme::BG_DARK);
                        // Вписываем изображение с сохранением пропорций
                        let tex_size = tex.size_vec2();
                        let scale = (img_w / tex_size.x).min(img_h / tex_size.y);
                        let draw_size = tex_size * scale;
                        let draw_rect = egui::Rect::from_center_size(img_rect.center(), draw_size);
                        ui.painter().image(tex.id(), draw_rect, uv, Color32::WHITE);
                        ui.painter().rect_stroke(img_rect, 4.0, Stroke::new(1.0, theme::BORDER), StrokeKind::Outside);
                    } else {
                        let (img_rect, _) = ui.allocate_exact_size(Vec2::new(img_w, img_h), Sense::hover());
                        ui.painter().rect_filled(img_rect, 4.0, theme::BG_DARK);
                        ui.painter().rect_stroke(img_rect, 4.0, Stroke::new(1.0, theme::BORDER), StrokeKind::Outside);
                        let icon = if m.preview_path.is_some() { "⏳" } else { "◫" };
                        ui.painter().text(
                            img_rect.center(),
                            Align2::CENTER_CENTER,
                            icon,
                            FontId::proportional(28.0),
                            theme::TEXT_MUTED,
                        );
                    }

                    ui.add_space(10.0);

                    // ── Название и версия ────────────────────────────────
                    let src_col = source_color(&m.source);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(source_label(&m.source))
                            .color(src_col).size(10.0).strong());
                        ui.add_space(4.0);
                        ui.label(RichText::new(&m.name)
                            .color(theme::TEXT_PRIMARY).size(13.0).strong());
                    });
                    if !m.version.is_empty() {
                        ui.label(RichText::new(format!("v{}", m.version))
                            .color(theme::TEXT_MUTED).size(11.0));
                    }

                    ui.add_space(6.0);

                    // ── Автор и ID ───────────────────────────────────────
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Автор:").color(theme::TEXT_MUTED).size(11.0));
                        ui.label(RichText::new(&m.author).color(theme::TEXT_ACCENT).size(11.0));
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("ID:").color(theme::TEXT_MUTED).size(11.0));
                        ui.label(RichText::new(&m.package_id).color(theme::TEXT_MUTED).size(10.5));
                    });
                    let versions = m.supported_versions.join(", ");
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Версии RW:").color(theme::TEXT_MUTED).size(11.0));
                        ui.label(RichText::new(versions).color(theme::TEXT_PRIMARY).size(11.0));
                    });

                    // ── Описание ─────────────────────────────────────────
                    if !m.description.is_empty() {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(4.0);
                        ui.label(RichText::new("ОПИСАНИЕ")
                            .color(theme::TEXT_MUTED).size(10.0).strong());
                        ui.add_space(4.0);
                        let desc = clean_unity_tags(&m.description);
                        if looks_like_markdown(&desc) {
                            egui_commonmark::CommonMarkViewer::new()
                                .show(ui, md_cache, &desc);
                        } else {
                            ui.add(egui::Label::new(
                                RichText::new(desc).color(theme::TEXT_PRIMARY).size(11.5)
                            ).wrap());
                        }
                    }

                    // ── Зависимости и несовместимости ────────────────────
                    let has_deps = !m.dependencies.is_empty();
                    let has_incompat = !m.incompatible_with.is_empty();
                    if has_deps || has_incompat {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(4.0);
                    }
                    if has_deps {
                        ui.label(RichText::new("ЗАВИСИМОСТИ")
                            .color(theme::TEXT_MUTED).size(10.0).strong());
                        for dep in &m.dependencies {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("→").color(theme::WARNING_AMBER).size(11.0));
                                ui.label(RichText::new(dep).color(theme::TEXT_PRIMARY).size(11.0));
                            });
                        }
                        ui.add_space(4.0);
                    }
                    if has_incompat {
                        ui.label(RichText::new("НЕСОВМЕСТИМО")
                            .color(theme::TEXT_MUTED).size(10.0).strong());
                        for ic in &m.incompatible_with {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("×").color(theme::ERROR_RED).size(11.0));
                                ui.label(RichText::new(ic).color(theme::TEXT_PRIMARY).size(11.0));
                            });
                        }
                    }
                });
        }
    }
}

pub fn source_color(source: &ModSource) -> Color32 {
    match source {
        ModSource::Core        => theme::SOURCE_CORE,
        ModSource::DLC(_)      => theme::SOURCE_DLC,
        ModSource::Workshop(_) => theme::SOURCE_WORKSHOP,
        ModSource::Local       => theme::SOURCE_LOCAL,
    }
}

pub fn source_label(source: &ModSource) -> &'static str {
    match source {
        ModSource::Core        => "CORE",
        ModSource::DLC(_)      => "DLC",
        ModSource::Workshop(_) => "WORKSHOP",
        ModSource::Local       => "LOCAL",
    }
}

/// Переносит все папки модов из `src_dir` (папка SteamCMD content/294100/)
/// в `dst_dir` (RimWorld/Mods). Если папка с таким именем уже существует
/// в назначении — она будет заменена (старая удаляется).
///
/// Используется fs::rename, а при ошибке (например, перенос между разными
/// файловыми системами) — fallback через рекурсивное копирование.
pub fn move_downloaded_mods(src_dir: &std::path::Path, dst_dir: &std::path::Path) {
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
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
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

pub fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    style.visuals.window_fill         = theme::BG_PANEL;
    style.visuals.panel_fill          = theme::BG_DARK;
    style.visuals.override_text_color = Some(theme::TEXT_PRIMARY);
    style.visuals.window_stroke       = Stroke::new(1.0, theme::BORDER);
    style.visuals.selection.bg_fill   = theme::BG_SELECTED;
    style.visuals.selection.stroke    = Stroke::new(1.0, theme::BORDER_ACCENT);
    style.visuals.extreme_bg_color    = theme::BG_DARK;
    style.visuals.faint_bg_color      = theme::BG_ROW_ODD;

    // ── Состояния виджетов ────────────────────────────────────────────────────
    // expansion = 0 на всех состояниях — кнопки не меняют размер при наведении.
    style.visuals.widgets.noninteractive.expansion = 0.0;
    style.visuals.widgets.inactive.expansion       = 0.0;
    style.visuals.widgets.hovered.expansion        = 0.0;
    style.visuals.widgets.active.expansion         = 0.0;
    style.visuals.widgets.open.expansion           = 0.0;

    style.visuals.widgets.noninteractive.bg_fill   = theme::BG_PANEL;
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, theme::TEXT_MUTED);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;

    style.visuals.widgets.inactive.bg_fill         = theme::BG_ROW_EVEN;
    style.visuals.widgets.inactive.bg_stroke       = Stroke::new(1.0, theme::BORDER);
    style.visuals.widgets.inactive.fg_stroke       = Stroke::new(1.0, theme::TEXT_PRIMARY);

    style.visuals.widgets.hovered.bg_fill          = theme::BG_ROW_HOVER;
    style.visuals.widgets.hovered.bg_stroke        = Stroke::new(1.0, theme::BORDER_ACCENT);
    style.visuals.widgets.hovered.fg_stroke        = Stroke::new(1.0, theme::TEXT_PRIMARY);

    style.visuals.widgets.active.bg_fill           = theme::BG_SELECTED;
    style.visuals.widgets.active.bg_stroke         = Stroke::new(1.0, theme::BORDER_ACCENT);
    style.visuals.widgets.active.fg_stroke         = Stroke::new(1.0, Color32::WHITE);

    style.visuals.widgets.open.bg_fill             = theme::BG_ROW_HOVER;
    style.visuals.widgets.open.bg_stroke           = Stroke::new(1.0, theme::BORDER_ACCENT);

    // ── Отступы ───────────────────────────────────────────────────────────────
    style.spacing.item_spacing   = Vec2::new(6.0, 3.0);
    style.spacing.window_margin  = Margin::same(10);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);

    ctx.set_global_style(style);
}
// ─── Описания модов: Markdown ────────────────────────────────────────────────

/// Переводит Unity rich-text теги RimWorld (`<b>`, `<i>`, `<color=…>`, `<size=…>`)
/// в Markdown-эквиваленты, чтобы описание не показывало сырые теги.
fn clean_unity_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        let Some(close) = tail.find('>') else {
            out.push_str(tail);
            break;
        };
        let tag = &tail[1..close]; // содержимое между < и >
        let lower = tag.to_ascii_lowercase();
        match lower.as_str() {
            "b" | "/b"   => out.push_str("**"),
            "i" | "/i"   => out.push_str("*"),
            "/color" | "/size" => {}
            _ if lower.starts_with("color=") || lower.starts_with("size=") => {}
            // Не тег форматирования (например "x < y") — оставляем как есть
            _ => out.push_str(&tail[..close + 1]),
        }
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Эвристика: похоже ли описание на Markdown. Обычный текст рендерим как раньше,
/// чтобы одиночные переводы строк не склеивались в параграфы.
fn looks_like_markdown(text: &str) -> bool {
    if text.contains("**") || text.contains("](") || text.contains('`') {
        return true;
    }
    text.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("# ") || t.starts_with("## ") || t.starts_with("### ")
            || t.starts_with("- ") || t.starts_with("* ") || t.starts_with("> ")
    })
}
