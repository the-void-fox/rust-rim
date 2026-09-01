//! Состояние приложения и корневой кадр UI.
//!
//! Домен (каталог модов и сборка) живёт в [`crate::mod_data`]; здесь остаётся
//! состояние интерфейса и трансляция пользовательских действий в изменения
//! домена — по одному месту на действие ([`RustRim::apply`]).

use std::collections::HashSet;

use egui::{Color32, Frame, Margin, RichText, Stroke, Vec2};

use crate::fs_util;
use crate::game::{launch, paths, Prefix};
use crate::process::Run;
use crate::bisect::Hunt;
use crate::testing::{self, Phase, TestRun};
use crate::mod_data::{
    ModDb, ModId, ModSource, Profile,
    parse_mods_config, scan_dlc_mods, scan_local_mods, write_mod_list, write_mods_config,
};
use crate::settings::AppSettings;
use crate::sorting::CommunityRules;
use crate::steam::steamcmd;
use crate::tags::{TagId, Tags};
use crate::ui::details::DetailsView;
use crate::ui::duplicates::DuplicatesUi;
use crate::ui::list_cache::{ListCaches, SearchState};
use crate::ui::tags_panel::TagsUi;
use crate::ui::launch_panel;
use crate::ui::bisect_panel::{self, BisectUi};
use crate::ui::updates_panel::{self, UpdatesUi};
use crate::ui::test_panel::{self, TestUi};
use crate::ui::validation_panel::{self, ValidationUi};
use crate::ui::log_panel::LogPanel;
use crate::ui::mod_list::ModList;
use crate::ui::preview::Preview;
use crate::ui::steamcmd_panel::SteamCmdPanel;
use crate::ui::workshop_browser::WorkshopBrowser;
use crate::ui::{dialogs, duplicates, tags_panel, theme, toolbar, widgets};

// ─── Payload для Drag & Drop ─────────────────────────────────────────────────
#[derive(Clone, Debug)]
pub struct DragPayload {
    pub id: ModId,
}

// ─── Действия пользователя ───────────────────────────────────────────────────
/// Всё, что UI может попросить сделать с модом. Списки и клавиатура только
/// возвращают действие, менять состояние им нельзя — иначе изменение сборки
/// посреди отрисовки рассинхронизировало бы кэши списков.
#[derive(Clone, Debug)]
pub enum Action {
    Activate(ModId),
    Deactivate(ModId),
    MoveUp(ModId),
    MoveDown(ModId),
    /// Drag & Drop: перенести мод на позицию `to_pos` списка `to_active`.
    /// `to_pos` — строка в *отображаемом* (отфильтрованном) списке.
    DragDrop { id: ModId, to_active: bool, to_pos: usize },
    OpenFolder(ModId),
    /// Повесить или снять тег.
    ToggleTag { id: ModId, tag: TagId },
    OpenTagEditor,
}

/// Минимальная осмысленная ширина колонки со списком модов: иконка источника,
/// обрезанное название, версия и значок предупреждения.
const MIN_LIST_WIDTH: f32 = 160.0;

/// Помещаются ли два списка рядом.
///
/// `Ui::columns` делит доступную ширину на число колонок и роняет egui через
/// `debug_assert`, если доля колонки отрицательна. Условие ниже гарантирует
/// долю не меньше [`MIN_LIST_WIDTH`], то есть заведомо положительную.
pub fn fits_two_columns(available: f32, spacing: f32) -> bool {
    available >= 2.0 * MIN_LIST_WIDTH + spacing
}

// ─── Открытые окна ───────────────────────────────────────────────────────────
#[derive(Default)]
struct Windows {
    paths: bool,
    save: bool,
    settings: bool,
    steamcmd: bool,
    workshop: bool,
    logs: bool,
}

// ─── Основное состояние приложения ───────────────────────────────────────────
pub struct RustRim {
    /// Каталог установленных модов (метаданные с диска).
    db: ModDb,
    /// Текущая сборка: что включено и в каком порядке грузится.
    profile: Profile,
    /// Выделенный мод — по идентификатору, а не по индексу: сортировка и
    /// перемещения меняют позиции, и индекс начинал указывать на другой мод.
    selected: Option<ModId>,
    search: SearchState,
    settings: AppSettings,

    windows: Windows,
    preview: Preview,

    /// Закешированные правила сообщества (загружаются при первой сортировке).
    community_rules: Option<CommunityRules>,

    /// Состояние диалогов дубликатов.
    duplicates: DuplicatesUi,

    /// Пользовательские теги модов и окно управления ими.
    tags: Tags,
    tags_ui: TagsUi,

    /// Автотест сборки через -quicktest.
    test_run: Option<TestRun>,
    test_ui: TestUi,
    /// Поиск виновника: цепочка прогонов с урезанными сборками.
    bisect: Option<Hunt>,
    bisect_ui: BisectUi,

    /// Проверка обновлений модов мастерской.
    updates_ui: UpdatesUi,

    /// Проверка сборки: диагностики и окно с ними.
    validation: ValidationUi,
    /// Версия игры из Version.txt — нужна для проверки supportedVersions.
    game_version: Option<String>,

    /// Найденные wine-префиксы (обновляются при открытии настроек).
    detected_prefixes: Vec<Prefix>,
    /// Запущенная игра вместе с её выводом.
    game_run: Option<Run>,
    show_launch_window: bool,
    /// Короткое сообщение пользователю: (текст, это ошибка).
    notice: Option<(String, bool)>,

    steamcmd_panel: SteamCmdPanel,
    workshop_browser: WorkshopBrowser,
    log_panel: LogPanel,

    /// Правая панель с информацией о моде (со своими кэшами).
    details: DetailsView,

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
        let has_paths = settings.has_required_paths();
        let mut app = Self {
            db: ModDb::empty(),
            profile: Profile::new(),
            selected: None,
            search: SearchState::default(),
            windows: Windows { paths: !has_paths, ..Windows::default() },
            preview: Preview::new(),
            community_rules: None,
            duplicates: DuplicatesUi::default(),
            tags: Tags::load(),
            tags_ui: TagsUi::default(),
            test_run: None,
            test_ui: TestUi::default(),
            bisect: None,
            bisect_ui: BisectUi::default(),
            updates_ui: UpdatesUi::default(),
            validation: ValidationUi::new(),
            game_version: None,
            detected_prefixes: Vec::new(),
            game_run: None,
            show_launch_window: false,
            notice: None,
            steamcmd_panel: SteamCmdPanel::new(),
            workshop_browser: WorkshopBrowser::new(),
            log_panel: LogPanel::new(),
            details: DetailsView::new(),
            caches: ListCaches::default(),
            settings,
        };
        if has_paths {
            app.load_mods();
        }
        app
    }

    // ── Загрузка каталога ────────────────────────────────────────────────────

    /// Пересканирует диск и применяет ModsConfig.xml.
    /// Текущая сборка при этом теряется — используется при старте и смене путей.
    fn load_mods(&mut self) {
        self.scan_into_db();
        self.clear_selection();
        self.apply_mods_config();
        self.duplicates.show_list = !self.db.duplicates().is_empty();
    }

    /// Пересканирует диск, сохранив текущую сборку.
    ///
    /// Это то, что делает кнопка «Перезагрузить»: пользователь докинул мод
    /// в папку руками и хочет увидеть его, не потеряв несохранённые правки
    /// порядка загрузки. Моды, исчезнувшие с диска, из сборки выпадают.
    fn reload_mods(&mut self) {
        let previous: Vec<ModId> = self.profile.order().to_vec();
        self.scan_into_db();
        self.profile = Profile::from_raw_ids(previous.iter().map(ModId::as_str), &self.db);
        self.ensure_core_active();
        self.after_profile_change();
        // Выделение переживает пересканирование, если мод никуда не делся.
        if self.selected.as_ref().is_some_and(|id| !self.db.contains(id)) {
            self.clear_selection();
        }
        self.duplicates.show_list = !self.db.duplicates().is_empty();
        tracing::info!("Rescanned: {} mods, {} active", self.db.len(), self.profile.len());
    }

    // ── Запуск игры ──────────────────────────────────────────────────────────

    /// Поиск префиксов лезет в несколько папок, поэтому делается один раз
    /// при открытии настроек, а не каждый кадр.
    fn open_settings(&mut self) {
        self.detected_prefixes = paths::find_prefixes();
        self.windows.settings = true;
    }

    fn launch_game(&mut self) {
        let mut effective = self.settings.launch.clone();
        if effective.prefix.trim().is_empty() {
            // Настройки могли не открывать ни разу — ищем префикс сейчас.
            if self.detected_prefixes.is_empty() {
                self.detected_prefixes = paths::find_prefixes();
            }
            if let Some(p) = self.detected_prefixes.first() {
                effective.prefix = p.path.to_string_lossy().into_owned();
            }
        }

        let game = std::path::Path::new(&self.settings.game_path);
        let plan = match launch::plan(game, &effective, &launch::Mode::Play) {
            Ok(plan) => plan,
            Err(e) => {
                self.notice = Some((e.to_string(), true));
                return;
            }
        };

        tracing::info!("Launching game: {}", plan.display());
        let command = plan.display();
        match Run::spawn(plan.to_command(), command.clone()) {
            Ok(run) => {
                self.game_run = Some(run);
                self.show_launch_window = true;
            }
            Err(e) => {
                self.notice = Some((format!("Не удалось запустить: {e}\n\n{command}"), true));
            }
        }
    }

    /// Живой вывод запуска: без него Proton молчит десятки секунд, и
    /// непонятно, запускается игра или нет.
    fn show_launch_window(&mut self, ctx: &egui::Context) {
        let Some(run) = &mut self.game_run else { return };
        let finished_now = run.poll();
        if run.is_running() || finished_now {
            // Пока идёт вывод, окно обновляется само.
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        if launch_panel::show(ctx, &mut self.show_launch_window, run) == launch_panel::Reply::Kill {
            run.kill();
        }
    }

    fn clear_selection(&mut self) {
        self.selected = None;
        self.preview.reset();
    }

    fn scan_into_db(&mut self) {
        let mut entries = Vec::new();

        // Core + DLC из папки Data/ игры идут первыми — так ванильный контент
        // всегда побеждает случайную копию из Workshop при дедупликации.
        if !self.settings.game_path.is_empty() {
            entries.extend(scan_dlc_mods(std::path::Path::new(&self.settings.game_path)));
        }

        if !self.settings.local_mods_path.is_empty() {
            entries.extend(scan_local_mods(std::path::Path::new(
                &self.settings.local_mods_path,
            )));
        }

        // Моды скачанные через SteamCMD (если папка существует)
        let sc_base = self.settings.effective_steamcmd_path();
        if !sc_base.is_empty() {
            let content = steamcmd::steam_content_path(std::path::Path::new(&sc_base));
            if content.is_dir() {
                entries.extend(scan_local_mods(&content));
            }
        }

        self.db = ModDb::build(entries);
        self.game_version = paths::game_version(std::path::Path::new(&self.settings.game_path));
        self.caches.invalidate();
    }

    /// Читает ModsConfig.xml и собирает по нему сборку.
    fn apply_mods_config(&mut self) {
        if self.settings.config_path.is_empty() {
            return self.activate_core_only();
        }
        let xml = std::path::Path::new(&self.settings.config_path).join("ModsConfig.xml");
        if !xml.exists() {
            return self.activate_core_only();
        }
        match parse_mods_config(&xml) {
            Ok(ids) => self.apply_raw_ids(&ids),
            Err(e) => {
                tracing::warn!("Failed to read ModsConfig.xml: {}", e);
                self.activate_core_only();
            }
        }
    }

    /// Собирает сборку по списку строк (packageId или Workshop ID).
    fn apply_raw_ids(&mut self, raw: &[String]) {
        self.profile = Profile::from_raw_ids(raw, &self.db);
        self.ensure_core_active();
        self.after_profile_change();
    }

    fn activate_core_only(&mut self) {
        self.profile.clear();
        self.ensure_core_active();
        self.after_profile_change();
    }

    /// Core выключить нельзя — игра без него не запустится.
    fn ensure_core_active(&mut self) {
        let core = self
            .db
            .iter()
            .find(|m| m.is_core())
            .map(|m| m.package_id.clone());
        if let Some(id) = core {
            if !self.profile.is_active(&id) {
                self.profile.activate_at(id, 0);
            }
        }
    }

    fn after_profile_change(&mut self) {
        self.caches.invalidate();
        // Диагностики считаются от состава и порядка сборки, значит устарели.
        // Это происходит раз на действие пользователя, а не каждый кадр.
        self.revalidate();
    }

    fn revalidate(&mut self) {
        self.validation.refresh(&self.db, &self.profile, self.game_version.as_deref());
    }

    fn is_core(&self, id: &ModId) -> bool {
        self.db.get(id).is_some_and(|m| m.is_core())
    }

    // ── Действия ─────────────────────────────────────────────────────────────

    fn apply(&mut self, action: Action) {
        match action {
            Action::Activate(id) => {
                if self.db.contains(&id) {
                    self.profile.activate(id);
                }
            }
            Action::Deactivate(id) => {
                if !self.is_core(&id) {
                    self.profile.deactivate(&id);
                }
            }
            Action::MoveUp(id) => self.profile.move_up(&id),
            Action::MoveDown(id) => self.profile.move_down(&id),
            Action::DragDrop { id, to_active, to_pos } => {
                if to_active {
                    if !self.db.contains(&id) {
                        return;
                    }
                    // to_pos — строка отфильтрованного списка; переводим её
                    // в позицию внутри сборки, иначе при активном поиске мод
                    // уезжал бы не туда, куда его бросили.
                    let target = self
                        .caches
                        .active
                        .get(to_pos)
                        .and_then(|anchor| self.profile.position(anchor))
                        .unwrap_or(self.profile.len());
                    self.profile.activate_at(id, target);
                } else if !self.is_core(&id) {
                    self.profile.deactivate(&id);
                }
            }
            Action::OpenFolder(id) => {
                if let Some(m) = self.db.get(&id) {
                    fs_util::open_in_file_manager(&m.path);
                }
                return; // состояние сборки не изменилось
            }
            Action::ToggleTag { id, tag } => {
                self.tags.toggle(&id, tag);
                self.tags.save();
                // Теги входят в ключ поиска и красят строку — кэш устарел.
                self.caches.invalidate();
                return;
            }
            Action::OpenTagEditor => {
                self.tags_ui.open = true;
                return;
            }
        }
        self.after_profile_change();
    }

    fn activate_all(&mut self) {
        let ids: Vec<ModId> = self.db.ids().cloned().collect();
        for id in ids {
            self.profile.activate(id);
        }
        self.after_profile_change();
    }

    fn deactivate_all(&mut self) {
        let core: HashSet<ModId> = self
            .db
            .iter()
            .filter(|m| m.is_core())
            .map(|m| m.package_id.clone())
            .collect();
        self.profile.retain(|id| core.contains(id));
        self.after_profile_change();
    }

    /// Включает недостающие зависимости активных модов (транзитивно).
    fn add_missing_dependencies(&mut self) -> usize {
        let mut activated = 0;
        let mut queue: Vec<ModId> = self.profile.order().to_vec();
        let mut visited: HashSet<ModId> = HashSet::new();

        while let Some(id) = queue.pop() {
            if !visited.insert(id.clone()) {
                continue;
            }
            let Some(m) = self.db.get(&id) else { continue };
            let deps = m.dependencies.clone();
            let name = m.name.clone();
            for dep in deps {
                if !self.db.contains(&dep) {
                    tracing::warn!("Missing dependency '{}' for mod '{}'", dep, name);
                    continue;
                }
                if !self.profile.is_active(&dep) {
                    self.profile.activate(dep.clone());
                    activated += 1;
                    queue.push(dep);
                }
            }
        }
        if activated > 0 {
            self.after_profile_change();
        }
        activated
    }

    fn sort_active_mods(&mut self) {
        let added = self.add_missing_dependencies();
        if added > 0 {
            tracing::info!("Automatically activated {} missing dependencies", added);
        }

        // Загружаем community rules при первом использовании (если включено)
        if self.settings.use_community_rules && self.community_rules.is_none() {
            match crate::sorting::fetch_community_rules() {
                Ok(rules) => {
                    tracing::info!("Community rules loaded (ts={})", rules.timestamp);
                    self.community_rules = Some(rules);
                }
                Err(e) => tracing::warn!("Failed to fetch community rules: {}", e),
            }
        }

        let rules = self.settings.use_community_rules.then(|| self.community_rules.as_ref()).flatten();
        crate::sorting::sort_active_mods(&mut self.profile, &self.db, rules);
        self.after_profile_change();
    }

    fn count_warnings(&self) -> usize {
        self.profile
            .order()
            .iter()
            .filter(|id| self.caches.warn_for(id).missing_deps)
            .count()
    }

    // ── Сохранение и обмен списками ──────────────────────────────────────────

    fn active_ids_for_export(&self) -> Vec<String> {
        self.profile
            .order()
            .iter()
            .map(|id| id.as_str().to_string())
            .collect()
    }

    fn save_mods_config(&mut self) {
        if self.settings.config_path.is_empty() {
            tracing::warn!("config_path is not set, cannot save ModsConfig.xml");
            return;
        }
        let xml = std::path::Path::new(&self.settings.config_path).join("ModsConfig.xml");
        let ids = self.active_ids_for_export();
        match write_mods_config(&xml, &ids) {
            Ok(()) => tracing::info!("Saved {} active mods to {:?}", ids.len(), xml),
            Err(e) => tracing::error!("Failed to write ModsConfig.xml: {}", e),
        }
    }

    fn export_mod_list(&mut self) {
        let Some(path) = dialogs::pick_save_file("Сохранить список модов") else { return };
        let ids = self.active_ids_for_export();
        match write_mod_list(&path, &ids) {
            Ok(()) => tracing::info!("Exported {} mods to {:?}", ids.len(), path),
            Err(e) => tracing::error!("Failed to export mod list: {}", e),
        }
    }

    fn import_mod_list(&mut self) {
        let Some(path) = dialogs::pick_open_file("Загрузить список модов") else { return };
        match parse_mods_config(&path) {
            Ok(ids) => {
                tracing::info!("Imported mod list with {} entries from {:?}", ids.len(), path);
                self.apply_raw_ids(&ids);
            }
            Err(e) => tracing::error!("Failed to import mod list from {:?}: {}", path, e),
        }
    }

    // ── Дубликаты ────────────────────────────────────────────────────────────

    /// Удаляет с диска отброшенные копии дублирующихся модов.
    fn remove_duplicates(&mut self) {
        let mut removed = 0usize;
        let discarded: Vec<std::path::PathBuf> = self
            .db
            .duplicates()
            .iter()
            .flat_map(|g| g.discarded.iter().cloned())
            .collect();

        for path in discarded {
            if !path.exists() {
                tracing::warn!("Duplicate folder already gone: {:?}", path);
                removed += 1;
                continue;
            }
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {
                    tracing::info!("Deleted duplicate mod folder: {:?}", path);
                    removed += 1;
                }
                Err(e) => tracing::error!("Failed to delete mod folder {:?}: {}", path, e),
            }
        }

        self.duplicates.last_removed = removed;
        self.db.clear_duplicates();
        // Каталог изменился на диске — перечитываем, сохранив сборку.
        self.reload_mods();
    }
}

// ─── Корневой кадр ───────────────────────────────────────────────────────────

impl eframe::App for RustRim {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        theme::apply(&ctx);

        self.show_toolbar(ui);
        self.show_status_bar(ui);

        // Индексы списков из кэша (пересчёт только при изменениях)
        self.caches.refresh(&self.db, &self.profile, &self.tags, &self.search);

        let mut action = self.handle_keyboard_nav(&ctx);

        self.show_details_panel(ui, &ctx);
        if let Some(req) = self.show_lists(ui) {
            action = Some(req);
        }

        if let Some(action) = action {
            self.apply(action);
        }

        self.show_drag_ghost(&ctx);
        self.show_dialogs(&ctx);
        self.show_notice(&ctx);
        self.show_launch_window(&ctx);
        self.show_validation(&ctx);
        self.show_test(&ctx);
        self.show_bisect(&ctx);
        self.show_updates(&ctx);
    }
}

impl RustRim {
    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        let resp = egui::Panel::top("toolbar_panel")
            .frame(Frame::NONE.fill(theme::BG_HEADER).inner_margin(Margin::symmetric(8, 6)))
            .show(ui, toolbar::show_toolbar)
            .inner;

        if resp.save_clicked      { self.windows.save = true; }
        if resp.sort_clicked      { self.sort_active_mods(); }
        if resp.settings_clicked  { self.open_settings(); }
        if resp.activate_all      { self.activate_all(); }
        if resp.deactivate_all    { self.deactivate_all(); }
        if resp.save_list_clicked { self.export_mod_list(); }
        if resp.load_list_clicked { self.import_mod_list(); }
        if resp.steamcmd_clicked  { self.windows.steamcmd = true; }
        if resp.workshop_clicked  { self.windows.workshop = true; }
        if resp.logs_clicked      { self.windows.logs = true; }
        if resp.reload_clicked    { self.reload_mods(); }
        if resp.updates_clicked   { self.updates_ui.open = true; }
        if resp.tags_clicked      { self.tags_ui.open = true; }
        if resp.play_clicked      { self.launch_game(); }
        if resp.check_clicked     { self.revalidate(); self.validation.open = true; }
        if resp.test_clicked      { self.start_test(); }
    }

    fn show_status_bar(&mut self, ui: &mut egui::Ui) {
        let active = self.profile.len();
        let total = self.db.len();
        let warnings = self.count_warnings();

        egui::Panel::bottom("status_bar")
            .frame(Frame::NONE.fill(theme::BG_HEADER).inner_margin(Margin::symmetric(10, 4)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Активных: {active}  •  Всего: {total}"))
                            .color(theme::TEXT_MUTED)
                            .size(11.5),
                    );
                    if warnings > 0 {
                        ui.separator();
                        ui.label(
                            RichText::new(format!("⚠ {warnings}"))
                                .color(theme::WARNING_AMBER)
                                .size(11.5),
                        );
                    }
                    let (errors, warns) = (self.validation.errors(), self.validation.warnings());
                    if self.validation.was_checked() && (errors > 0 || warns > 0) {
                        ui.separator();
                        let color = if errors > 0 { theme::ERROR_RED } else { theme::WARNING_AMBER };
                        let label = RichText::new(format!("Проверка: × {errors}  ⚠ {warns}"))
                            .color(color)
                            .size(11.5);
                        if ui
                            .add(egui::Button::new(label).frame(false))
                            .on_hover_text("Открыть проверку сборки")
                            .clicked()
                        {
                            self.validation.open = true;
                        }
                    }
                });
            });
    }

    fn show_details_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let preview_path = self
            .selected
            .as_ref()
            .and_then(|id| self.db.get(id))
            .and_then(|m| m.preview_path.clone());

        egui::Panel::right("details_panel")
            .min_size(240.0)
            .default_size(300.0)
            .max_size(500.0)
            .resizable(true)
            .frame(
                Frame::NONE
                    .fill(theme::BG_PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER_ACCENT))
                    .inner_margin(Margin::symmetric(10, 5)),
            )
            .show(ui, |ui| {
                Frame::NONE
                    .fill(theme::BG_HEADER)
                    .inner_margin(Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.set_width(widgets::fit_width(ui));
                        ui.label(
                            RichText::new("ИНФОРМАЦИЯ О МОДЕ")
                                .color(theme::TEXT_MUTED)
                                .size(11.0)
                                .strong(),
                        );
                    });

                let tex = self.preview.texture_for(ctx, preview_path.as_deref());
                let selected = self.selected.as_ref().and_then(|id| self.db.get(id));
                let opts = crate::description::Options {
                    inline_images: self.settings.load_remote_images,
                };
                self.details.show(ui, selected, tex, opts);
            });
    }

    fn show_lists(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let inactive_count = self.db.len().saturating_sub(self.profile.len());
        let active_count = self.profile.len();

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(theme::BG_DARK))
            .show(ui, |ui| {
                let spacing = ui.spacing().item_spacing.x;

                // `Ui::columns` делит доступную ширину на число колонок и
                // роняет egui, если результат отрицательный. Поэтому две
                // колонки рисуем только когда они реально помещаются, иначе
                // остаётся один список — порядок загрузки важнее каталога.
                if !fits_two_columns(widgets::fit_width(ui), spacing) {
                    if let Some(req) = self.list_column(ui, true, active_count) {
                        action = Some(req);
                    }
                    return;
                }

                ui.columns(2, |cols| {
                    if let Some(req) = self.list_column(&mut cols[0], false, inactive_count) {
                        action = Some(req);
                    }
                    if let Some(req) = self.list_column(&mut cols[1], true, active_count) {
                        action = Some(req);
                    }
                });
            });

        action
    }

    /// Одна колонка списка модов: заголовок, поиск и сам список.
    fn list_column(&mut self, ui: &mut egui::Ui, is_active: bool, count: usize) -> Option<Action> {
        let (title, accent) = if is_active {
            ("АКТИВНЫЕ МОДЫ", theme::HEADER_RIGHT)
        } else {
            ("НЕАКТИВНЫЕ МОДЫ", theme::HEADER_LEFT)
        };
        let (ids, query, search_id) = if is_active {
            (&self.caches.active, &mut self.search.active_query, "active_search")
        } else {
            (&self.caches.inactive, &mut self.search.inactive_query, "inactive_search")
        };

        let mut action = None;
        Frame::NONE
            .fill(theme::BG_PANEL)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .show(ui, |ui| {
                widgets::panel_header(ui, title, accent, is_active, count);
                ui.add_space(2.0);
                widgets::search_bar(ui, query, search_id);
                ui.add_space(2.0);
                action = ModList::new(
                    &self.db, ids, &self.caches.warn, &self.tags,
                    &mut self.selected, is_active,
                )
                .show(ui);
            });
        action
    }

    fn show_drag_ghost(&mut self, ctx: &egui::Context) {
        if let Some(payload) = egui::DragAndDrop::payload::<DragPayload>(ctx) {
            if let Some(cursor) = ctx.pointer_latest_pos() {
                let name = self
                    .db
                    .get(&payload.id)
                    .map(|m| m.name.as_str())
                    .unwrap_or("...");
                egui::Area::new(egui::Id::new("drag_ghost"))
                    .fixed_pos(cursor + Vec2::new(14.0, -10.0))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(ctx, |ui| {
                        Frame::NONE
                            .fill(theme::BG_SELECTED)
                            .inner_margin(Margin::symmetric(10, 5))
                            .stroke(Stroke::new(1.0, theme::BORDER_ACCENT))
                            .show(ui, |ui| {
                                ui.label(RichText::new(name).color(Color32::WHITE).size(12.0));
                            });
                    });
            }
        }
        // Если мышь отпущена вне любого списка — сбрасываем payload
        if egui::DragAndDrop::has_any_payload(ctx) && ctx.input(|i| i.pointer.primary_released()) {
            egui::DragAndDrop::clear_payload(ctx);
        }
    }

    /// Обрабатывает стрелки, Enter и F5.
    /// - ↑ / ↓ — двигает выделение внутри того списка, где сейчас выделенный мод
    /// - Enter — переносит выделенный мод в другой список
    /// - Ctrl+↑ / Ctrl+↓ — меняет позицию активного мода в порядке загрузки
    /// - F5 — пересканировать папки модов
    ///
    /// Ввод игнорируется, если пользователь печатает в TextEdit (поиск),
    /// чтобы не перехватывать стрелки в тексте.
    fn handle_keyboard_nav(&mut self, ctx: &egui::Context) -> Option<Action> {
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.reload_mods();
            return None;
        }
        if ctx.memory(|m| m.focused().is_some()) {
            return None;
        }

        let (up, down, enter, ctrl) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Enter),
                i.modifiers.ctrl || i.modifiers.command,
            )
        });
        if !(up || down || enter) {
            return None;
        }

        // Определяем, в каком списке сейчас выделенный мод.
        let Some(selected) = self.selected.clone() else {
            // Ничего не выделено: первое нажатие стрелки выделяет первый элемент.
            if up || down {
                self.selected = self
                    .caches
                    .inactive
                    .first()
                    .or_else(|| self.caches.active.first())
                    .cloned();
            }
            return None;
        };

        let in_active_list = self.profile.is_active(&selected);
        let list: &[ModId] = if in_active_list {
            &self.caches.active
        } else {
            &self.caches.inactive
        };

        // Ctrl+↑ / Ctrl+↓ — переупорядочивание в списке активных.
        if ctrl && in_active_list {
            if up {
                return Some(Action::MoveUp(selected));
            }
            if down {
                return Some(Action::MoveDown(selected));
            }
        }

        if up || down {
            if list.is_empty() {
                return None;
            }
            let new_pos = match list.iter().position(|i| *i == selected) {
                Some(p) if up => p.saturating_sub(1),
                Some(p) => (p + 1).min(list.len() - 1),
                None => 0,
            };
            self.selected = Some(list[new_pos].clone());
            return None;
        }

        // Enter — переносим мод в противоположный список.
        if in_active_list {
            if !self.is_core(&selected) {
                return Some(Action::Deactivate(selected));
            }
        } else {
            return Some(Action::Activate(selected));
        }

        None
    }

    // ── Диалоги ──────────────────────────────────────────────────────────────

    fn show_dialogs(&mut self, ctx: &egui::Context) {
        if self.windows.paths
            && dialogs::open_folder_dialog(ctx, &mut self.windows.paths, &mut self.settings)
        {
            self.settings.save();
            self.load_mods();
        }

        if self.windows.save {
            let config_path = self.settings.config_path.clone();
            if dialogs::save_dialog(
                ctx,
                &mut self.windows.save,
                self.profile.len(),
                self.db.len(),
                &config_path,
            ) {
                self.save_mods_config();
            }
        }

        if self.windows.settings
            && dialogs::settings_dialog(
                ctx,
                &mut self.windows.settings,
                &mut self.settings,
                &self.detected_prefixes,
            )
        {
            self.settings.save();
            self.load_mods();
        }

        if tags_panel::show(ctx, &mut self.tags_ui, &mut self.tags) {
            self.tags.save();
            self.caches.invalidate();
        }

        self.show_steamcmd_panel(ctx);
        self.show_workshop_browser(ctx);
        self.show_log_panel(ctx);
        if duplicates::show(ctx, &mut self.duplicates, &self.db) {
            self.remove_duplicates();
        }
    }

    fn show_steamcmd_panel(&mut self, ctx: &egui::Context) {
        if !self.windows.steamcmd {
            return;
        }
        let base = self.settings.effective_steamcmd_path();
        let finished = self.steamcmd_panel.show(
            ctx,
            &mut self.windows.steamcmd,
            &base,
            self.settings.steamcmd_auto_move,
            self.settings.steamcmd_multi_download,
            self.settings.steamcmd_max_processes,
            self.settings.steamcmd_multi_threshold,
        );
        if !finished {
            return;
        }
        // Переносим скачанные моды в RimWorld/Mods, чтобы они лежали
        // как обычные локальные моды.
        let src = steamcmd::steam_content_path(std::path::Path::new(&base));
        if !self.settings.local_mods_path.is_empty() && src.is_dir() {
            fs_util::move_downloaded_mods(&src, std::path::Path::new(&self.settings.local_mods_path));
        }
        self.reload_mods();
    }

    fn show_workshop_browser(&mut self, ctx: &egui::Context) {
        if !self.windows.workshop {
            return;
        }
        let installed: HashSet<u64> = self
            .db
            .iter()
            .filter_map(|m| match m.source {
                ModSource::Workshop(id) => Some(id),
                _ => None,
            })
            .collect();
        if let Some(ids) =
            self.workshop_browser.show(ctx, &mut self.windows.workshop, &installed)
        {
            self.steamcmd_panel.add_ids(&ids);
            self.windows.steamcmd = true;
        }
    }

    /// Запускает прогон текущей сборки.
    fn start_test(&mut self) {
        let active = self.profile.order().to_vec();
        match self.build_test(&active) {
            Ok(run) => {
                self.test_run = Some(run);
                self.test_ui.reset();
            }
            Err(e) => self.notice = Some((e, true)),
        }
    }

    /// Готовит прогон произвольного набора модов.
    ///
    /// Общее для обычного теста и для поиска виновника: настройки, план
    /// запуска и уборка процессов у них одни и те же, разный только состав.
    fn build_test(&mut self, active: &[ModId]) -> Result<TestRun, String> {
        let Some(prefix) = self.effective_prefix() else {
            return Err("Не найден wine-префикс: укажите его в Настройки → Запуск.".to_string());
        };
        if self.settings.config_path.is_empty() {
            return Err(
                "Не задан путь к конфигу игры — прогону некуда положить сборку.".to_string()
            );
        }

        let game = std::path::Path::new(&self.settings.game_path);
        let config_dir = std::path::PathBuf::from(&self.settings.config_path);
        // Лог кладём рядом с Player.log: под umu игра работает в контейнере
        // со своим /tmp, и файл на произвольном пути хосту не виден.
        let log_file = config_dir
            .parent()
            .unwrap_or(&config_dir)
            .join("rustrim-quicktest.log");

        let mut launch_settings = self.settings.launch.clone();
        launch_settings.prefix = prefix.to_string_lossy().into_owned();
        let mode = launch::Mode::QuickTest { log_file: log_file.clone() };
        let plan = launch::plan(game, &launch_settings, &mode).map_err(|e| e.to_string())?;

        let mut config = testing::Config::new(config_dir, log_file);
        if let Some(exe) = paths::find_executable(game) {
            config.cleanup = Some((prefix, exe.path().to_path_buf()));
        }

        TestRun::start(plan.to_command(), plan.display(), config, active)
            .map_err(|e| format!("Не удалось запустить прогон: {e}"))
    }

    fn effective_prefix(&mut self) -> Option<std::path::PathBuf> {
        let configured = self.settings.launch.prefix.trim();
        if !configured.is_empty() {
            return Some(std::path::PathBuf::from(configured));
        }
        if self.detected_prefixes.is_empty() {
            self.detected_prefixes = paths::find_prefixes();
        }
        self.detected_prefixes.first().map(|p| p.path.clone())
    }

    fn show_test(&mut self, ctx: &egui::Context) {
        if let Some(run) = &mut self.test_run {
            let finished = matches!(run.poll(), Phase::Done(_));
            if finished {
                // Разбор лога делается один раз, а не каждый кадр.
                if !self.test_ui.issues_ready() {
                    let issues = run.issues(&self.db, &self.profile);
                    self.test_ui.set_issues(issues);
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
        }

        match test_panel::show(ctx, &mut self.test_ui, self.test_run.as_ref()) {
            test_panel::Reply::None => {}
            test_panel::Reply::Cancel => {
                if let Some(run) = &mut self.test_run {
                    run.cancel();
                }
            }
            test_panel::Reply::Restart => self.start_test(),
            test_panel::Reply::Bisect => self.start_bisect(),
            test_panel::Reply::Select(id) => {
                if self.db.contains(&id) {
                    self.selected = Some(id);
                }
            }
        }
    }

    /// Запускает поиск виновника по результатам последнего прогона.
    fn start_bisect(&mut self) {
        let issues = self.test_ui.issues().to_vec();
        self.bisect = Some(Hunt::from_failed_run(&self.db, &self.profile, &issues));
        self.bisect_ui.open = true;
    }

    fn show_bisect(&mut self, ctx: &egui::Context) {
        // Поиск на время работы вынимается из self: чтобы запустить очередной
        // прогон, ему нужен build_test, а тот забирает self целиком.
        if let Some(mut hunt) = self.bisect.take() {
            hunt.poll(&self.db, &self.profile);
            if let Some(active) = hunt.wants_run() {
                match self.build_test(&active) {
                    Ok(run) => hunt.attach(run),
                    Err(e) => {
                        self.notice = Some((e, true));
                        hunt.cancel();
                    }
                }
            }
            if !hunt.is_done() {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
            self.bisect = Some(hunt);
        }

        match bisect_panel::show(ctx, &mut self.bisect_ui, self.bisect.as_ref(), &self.db) {
            bisect_panel::Reply::None => {}
            bisect_panel::Reply::Cancel => {
                if let Some(hunt) = &mut self.bisect {
                    hunt.cancel();
                }
            }
            bisect_panel::Reply::Select(id) => {
                if self.db.contains(&id) {
                    self.selected = Some(id);
                }
            }
            bisect_panel::Reply::Deactivate(ids) => {
                for id in ids {
                    self.apply(Action::Deactivate(id));
                }
            }
        }
    }

    fn show_updates(&mut self, ctx: &egui::Context) {
        if self.updates_ui.job.poll() {
            // Отмечаем всё найденное: обновляют обычно пачкой.
            if let Some(found) = self.updates_ui.job.result() {
                let found = found.clone();
                self.updates_ui.select_all(&found);
            }
            ctx.request_repaint();
        } else if self.updates_ui.job.is_running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }

        match updates_panel::show(ctx, &mut self.updates_ui, &self.db) {
            updates_panel::Reply::None => {}
            updates_panel::Reply::Check => self.start_update_check(),
            updates_panel::Reply::Select(id) => {
                if self.db.contains(&id) {
                    self.selected = Some(id);
                }
            }
            updates_panel::Reply::Download(ids) => {
                // Обновление — это та же закачка: SteamCMD перезапишет папку.
                self.steamcmd_panel.add_ids(&ids);
                self.windows.steamcmd = true;
            }
        }
    }

    /// Спрашивает мастерскую о свежести установленных модов.
    fn start_update_check(&mut self) {
        let installed = crate::updates::installed_workshop_mods(&self.db);
        self.updates_ui.reset();
        self.updates_ui.job = crate::job::Job::spawn(move || {
            crate::updates::check(&installed).map_err(|e| e.to_string())
        });
    }

    fn show_validation(&mut self, ctx: &egui::Context) {
        match validation_panel::show(ctx, &mut self.validation, &self.db) {
            validation_panel::Reply::None => {}
            validation_panel::Reply::Select(id) => self.selected = Some(id),
            validation_panel::Reply::Apply(fix) => self.apply_fix(fix),
        }
    }

    fn apply_fix(&mut self, fix: crate::validation::Fix) {
        use crate::validation::Fix;
        match fix {
            Fix::Activate(id) => self.apply(Action::Activate(id)),
            Fix::Deactivate(id) => self.apply(Action::Deactivate(id)),
            Fix::Sort => self.sort_active_mods(),
            // Не качаем сразу: ставим в очередь SteamCMD и открываем её, чтобы
            // недостающие зависимости можно было набрать пачкой и скачать разом.
            Fix::Download(workshop_id) => {
                self.steamcmd_panel.add_ids(&[workshop_id]);
                self.windows.steamcmd = true;
            }
        }
    }

    /// Короткое сообщение о результате действия (например, запуска игры).
    fn show_notice(&mut self, ctx: &egui::Context) {
        let Some((text, is_error)) = self.notice.clone() else { return };
        let title = if is_error { "Ошибка" } else { "Готово" };
        let color = if is_error { theme::ERROR_RED } else { theme::ACTIVE_GREEN };
        let mut open = true;

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .max_width(560.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(&text).color(color).size(11.5));
                ui.add_space(8.0);
                if ui.button("OK").clicked() {
                    self.notice = None;
                }
            });
        if !open {
            self.notice = None;
        }
    }

    fn show_log_panel(&mut self, ctx: &egui::Context) {
        if !self.windows.logs {
            return;
        }
        let path_before = self.settings.log_file_path.clone();
        let picked = self.log_panel.show(
            ctx,
            &mut self.windows.logs,
            &self.db,
            &self.profile,
            &mut self.settings.log_file_path,
        );
        if self.settings.log_file_path != path_before {
            self.settings.save();
        }
        // Клик по подозреваемому — выделяем мод в списке
        if let Some(id) = picked {
            if self.db.contains(&id) {
                self.selected = Some(id);
            }
        }
    }
}
