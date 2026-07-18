use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

use egui::{Frame, Margin, RichText, Stroke, Vec2};

use crate::app::theme;
use crate::steam::workshop_api::{self, CollectionItem, SortOrder, WorkshopItem};

// ─── Tab ─────────────────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum BrowserTab {
    Mods,
    Collections,
}

// ─── Async image cache ───────────────────────────────────────────────────────

/// Максимум текстур, загружаемых в GPU за один кадр: защита от фриза,
/// когда приходит сразу страница из 30 картинок.
const MAX_TEXTURE_UPLOADS_PER_FRAME: usize = 3;

struct ImageCache {
    textures: HashMap<String, egui::TextureHandle>,
    pending: HashSet<String>,
    // Декод выполняется в фоновом потоке; по каналу приходит готовый ColorImage.
    tx: mpsc::Sender<(String, egui::ColorImage)>,
    rx: mpsc::Receiver<(String, egui::ColorImage)>,
}

impl ImageCache {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { textures: HashMap::new(), pending: HashSet::new(), tx, rx }
    }

    fn request(&mut self, url: &str) {
        if url.is_empty() || self.textures.contains_key(url) || self.pending.contains(url) {
            return;
        }
        self.pending.insert(url.to_string());
        let url_owned = url.to_string();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            // Скачивание И декод — в этом потоке; UI-поток только грузит текстуру.
            let result: anyhow::Result<egui::ColorImage> = (|| {
                let buf = ureq::get(&url_owned)
                    .header("User-Agent", "Mozilla/5.0")
                    .call()?
                    .body_mut()
                    .read_to_vec()?;
                let img = image::load_from_memory(&buf)?;
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                Ok(egui::ColorImage::from_rgba_unmultiplied(size, &rgba.into_raw()))
            })();
            if let Ok(ci) = result {
                let _ = tx.send((url_owned, ci));
            }
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        for _ in 0..MAX_TEXTURE_UPLOADS_PER_FRAME {
            let Ok((url, ci)) = self.rx.try_recv() else { break };
            self.pending.remove(&url);
            let tex = ctx.load_texture(&url, ci, egui::TextureOptions::LINEAR);
            self.textures.insert(url, tex);
        }
    }

    fn get(&self, url: &str) -> Option<&egui::TextureHandle> {
        self.textures.get(url)
    }

    fn is_busy(&self) -> bool {
        !self.pending.is_empty()
    }
}

// ─── Fetch states ────────────────────────────────────────────────────────────

enum FetchState {
    Idle,
    Loading,
    Done(Vec<WorkshopItem>),
    Error(String),
}

enum CollBrowseState {
    Idle,
    Loading,
    Done(Vec<CollectionItem>),
    Error(String),
}

// ─── Панель ───────────────────────────────────────────────────────────────────

pub struct WorkshopBrowser {
    active_tab: BrowserTab,

    // ── Вкладка Моды ──────────────────────────────────────────────────────────
    search_input: String,
    sort: SortOrder,
    page: u32,
    has_prev: bool,
    has_next: bool,
    state: FetchState,
    fetch_rx: Option<mpsc::Receiver<Result<(Vec<WorkshopItem>, bool), String>>>,
    images: ImageCache,
    queue: Vec<(u64, String)>,
    auto_loaded: bool,

    // ── Вкладка Сборки ────────────────────────────────────────────────────────
    coll_search: String,
    coll_sort: SortOrder,
    coll_page: u32,
    coll_has_prev: bool,
    coll_has_next: bool,
    coll_state: CollBrowseState,
    coll_fetch_rx: Option<mpsc::Receiver<Result<(Vec<CollectionItem>, bool), String>>>,
    coll_images: ImageCache,
    coll_auto_loaded: bool,

    // ── Скачивание сборки ─────────────────────────────────────────────────────
    /// (ID, название) сборки, которая сейчас загружается
    coll_dl_for: Option<(u64, String)>,
    coll_dl_rx: Option<mpsc::Receiver<Result<(String, Vec<WorkshopItem>), String>>>,
    /// Уведомление после сохранения (путь к файлу + количество модов)
    coll_notif: Option<String>,
}

impl WorkshopBrowser {
    pub fn new() -> Self {
        Self {
            active_tab: BrowserTab::Mods,

            search_input: String::new(),
            sort: SortOrder::Trending,
            page: 1,
            has_prev: false,
            has_next: false,
            state: FetchState::Idle,
            fetch_rx: None,
            images: ImageCache::new(),
            queue: Vec::new(),
            auto_loaded: false,

            coll_search: String::new(),
            coll_sort: SortOrder::Trending,
            coll_page: 1,
            coll_has_prev: false,
            coll_has_next: false,
            coll_state: CollBrowseState::Idle,
            coll_fetch_rx: None,
            coll_images: ImageCache::new(),
            coll_auto_loaded: false,

            coll_dl_for: None,
            coll_dl_rx: None,
            coll_notif: None,
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        installed_ids: &HashSet<u64>,
    ) -> Option<Vec<u64>> {
        self.images.poll(ctx);
        self.coll_images.poll(ctx);
        self.poll_fetch();
        self.poll_coll_fetch();
        self.poll_coll_download(installed_ids);

        if !self.auto_loaded {
            self.auto_loaded = true;
            self.trigger_fetch();
        }

        let any_loading = matches!(&self.state, FetchState::Loading)
            || matches!(&self.coll_state, CollBrowseState::Loading)
            || self.images.is_busy()
            || self.coll_images.is_busy()
            || self.coll_dl_rx.is_some();
        if any_loading {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }

        let mut result = None;
        egui::Window::new("🔍  Steam Workshop — Браузер модов")
            .open(open)
            .collapsible(false)
            .resizable(true)
            .min_width(720.0)
            .min_height(520.0)
            .frame(
                Frame::window(&ctx.global_style())
                    .fill(theme::BG_PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)),
            )
            .show(ctx, |ui| {
                result = self.content(ui, installed_ids);
            });

        result
    }

    // ── Polling ───────────────────────────────────────────────────────────────

    fn poll_fetch(&mut self) {
        let Some(rx) = &self.fetch_rx else { return };
        if let Ok(res) = rx.try_recv() {
            self.fetch_rx = None;
            match res {
                Ok((items, has_next)) => {
                    self.has_next = has_next;
                    self.state = FetchState::Done(items);
                }
                Err(e) => self.state = FetchState::Error(e),
            }
        }
    }

    fn poll_coll_fetch(&mut self) {
        let Some(rx) = &self.coll_fetch_rx else { return };
        if let Ok(res) = rx.try_recv() {
            self.coll_fetch_rx = None;
            match res {
                Ok((items, has_next)) => {
                    self.coll_has_next = has_next;
                    self.coll_state = CollBrowseState::Done(items);
                }
                Err(e) => self.coll_state = CollBrowseState::Error(e),
            }
        }
    }

    fn poll_coll_download(&mut self, installed_ids: &HashSet<u64>) {
        let Some(rx) = &self.coll_dl_rx else { return };
        let Ok(res) = rx.try_recv() else { return };
        self.coll_dl_rx = None;
        let stored_title = self.coll_dl_for.take().map(|(_, t)| t).unwrap_or_default();

        match res {
            Err(e) => {
                self.coll_notif = Some(format!("× Ошибка: {e}"));
            }
            Ok((_api_title, items)) => {
                // Используем название из карточки (API его не возвращает)
                let title = stored_title;
                let saved_path = save_collection_file(&title, &items);

                // В очередь добавляем только не установленные
                let new_mods: Vec<(u64, String)> = items
                    .iter()
                    .filter(|it| !installed_ids.contains(&it.id))
                    .filter(|it| !self.queue.iter().any(|(qid, _)| *qid == it.id))
                    .map(|it| (it.id, it.title.clone()))
                    .collect();
                let added = new_mods.len();
                self.queue.extend(new_mods);

                let file_info = saved_path
                    .as_ref()
                    .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
                    .unwrap_or_else(|| "не сохранено".into());

                self.coll_notif = Some(format!(
                    "✓ Сборка «{title}»\n  В очередь добавлено: {added} мод(ов)\n  Файл: {file_info}"
                ));
            }
        }
    }

    // ── Trigger fetches ───────────────────────────────────────────────────────

    fn trigger_fetch(&mut self) {
        let query = self.search_input.clone();
        let sort = self.sort;
        let page = self.page;
        let (tx, rx) = mpsc::channel();
        self.fetch_rx = Some(rx);
        self.state = FetchState::Loading;
        self.has_prev = page > 1;
        std::thread::spawn(move || {
            let res = workshop_api::fetch_workshop_page(&query, page, sort)
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    fn trigger_coll_fetch(&mut self) {
        let query = self.coll_search.clone();
        let sort = self.coll_sort;
        let page = self.coll_page;
        let (tx, rx) = mpsc::channel();
        self.coll_fetch_rx = Some(rx);
        self.coll_state = CollBrowseState::Loading;
        self.coll_has_prev = page > 1;
        std::thread::spawn(move || {
            let res = workshop_api::fetch_collections_page(&query, page, sort)
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    fn trigger_coll_download(&mut self, collection_id: u64, title: String) {
        if self.coll_dl_for.is_some() {
            return; // уже загружается
        }
        self.coll_dl_for = Some((collection_id, title));
        let (tx, rx) = mpsc::channel();
        self.coll_dl_rx = Some(rx);
        std::thread::spawn(move || {
            let res = workshop_api::fetch_collection_mods(collection_id)
                .map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    // ── UI ────────────────────────────────────────────────────────────────────

    fn content(&mut self, ui: &mut egui::Ui, installed_ids: &HashSet<u64>) -> Option<Vec<u64>> {
        // ── Вкладки ──────────────────────────────────────────────────────────
        Frame::NONE
            .fill(theme::BG_DARK)
            .inner_margin(Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    tab_btn(ui, "📦  Моды",    self.active_tab == BrowserTab::Mods,        || self.active_tab = BrowserTab::Mods);
                    tab_btn(ui, "📚  Сборки",  self.active_tab == BrowserTab::Collections,  || self.active_tab = BrowserTab::Collections);
                });
            });

        ui.add_space(2.0);

        match self.active_tab {
            BrowserTab::Mods        => self.content_mods(ui, installed_ids),
            BrowserTab::Collections => self.content_collections(ui),
        }
    }

    // ── Вкладка Моды ─────────────────────────────────────────────────────────

    fn content_mods(&mut self, ui: &mut egui::Ui, installed_ids: &HashSet<u64>) -> Option<Vec<u64>> {
        let mut to_download: Option<Vec<u64>> = None;

        let mut do_fetch = false;
        Frame::NONE
            .fill(theme::BG_HEADER)
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let resp = ui.add_sized(
                        [280.0, 22.0],
                        egui::TextEdit::singleline(&mut self.search_input)
                            .hint_text("Поиск модов RimWorld..."),
                    );
                    let search = ui.button(RichText::new("🔍").size(12.0))
                        .on_hover_text("Найти").clicked()
                        || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if search { self.page = 1; do_fetch = true; }

                    ui.add_space(6.0);
                    if sort_combo(ui, "wsbrowser_sort", &mut self.sort) {
                        self.page = 1;
                        do_fetch = true;
                    }
                    ui.add_space(8.0);
                    if pagination(ui, &mut self.page, self.has_prev, self.has_next) {
                        do_fetch = true;
                    }

                    if matches!(&self.state, FetchState::Loading) {
                        ui.add_space(8.0);
                        ui.spinner();
                    }
                });
            });
        if do_fetch { self.trigger_fetch(); }

        ui.add_space(2.0);

        let items_snap: Option<Vec<WorkshopItem>> = match &self.state {
            FetchState::Done(v) => Some(v.clone()),
            _ => None,
        };
        let err_msg: Option<String> = match &self.state {
            FetchState::Error(e) => Some(e.clone()),
            _ => None,
        };
        let is_idle    = matches!(&self.state, FetchState::Idle);
        let is_loading = matches!(&self.state, FetchState::Loading);

        let queue_height = if self.queue.is_empty() { 0.0 } else { 72.0 };
        let results_h = (ui.available_height() - queue_height - 4.0).max(80.0);

        egui::ScrollArea::vertical()
            .id_salt("wsbrowser_results")
            .max_height(results_h)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                if is_idle || is_loading {
                    if is_idle {
                        ui.add_space(50.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new("Нажмите 🔍 для просмотра популярных модов")
                                    .color(theme::TEXT_MUTED).size(12.0).italics(),
                            );
                        });
                    }
                    return;
                }

                if let Some(e) = err_msg {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(format!("× {e}")).color(theme::ERROR_RED).size(11.0));
                    });
                    return;
                }

                let Some(items) = items_snap else { return };

                if items.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("Ничего не найдено").color(theme::TEXT_MUTED).size(12.0).italics(),
                        );
                    });
                    return;
                }

                for item in &items {
                    self.images.request(&item.preview_url);
                }

                let w = ui.available_width();
                for item in &items {
                    let in_queue   = self.queue.iter().any(|(id, _)| *id == item.id);
                    let is_installed = installed_ids.contains(&item.id);
                    let row_bg = if in_queue { theme::BG_SELECTED } else { theme::BG_ROW_EVEN };

                    let mut card_consumed = false;
                    let frame_resp = Frame::NONE
                        .fill(row_bg)
                        .inner_margin(Margin::symmetric(8, 5))
                        .show(ui, |ui| {
                            ui.set_width(w - 16.0);
                            ui.horizontal(|ui| {
                                // Превью
                                let img_size = Vec2::new(144.0, 144.0);
                                if let Some(tex) = self.images.get(&item.preview_url) {
                                    ui.add(egui::Image::new(tex).fit_to_exact_size(img_size));
                                } else {
                                    img_placeholder(ui, img_size);
                                }
                                ui.add_space(8.0);

                                // Инфо
                                ui.vertical(|ui| {
                                    ui.set_width(ui.available_width() - 110.0);
                                    ui.label(RichText::new(&item.title).color(theme::TEXT_PRIMARY).size(12.5).strong());
                                    ui.label(
                                        RichText::new(format!("by {}  •  ID: {}", item.author, item.id))
                                            .color(theme::TEXT_MUTED).size(10.5),
                                    );
                                });

                                // Кнопка
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if is_installed {
                                        ui.label(RichText::new("✓ Установлено").color(theme::ACTIVE_GREEN).size(11.0));
                                    } else if in_queue {
                                        let btn = egui::Button::new(
                                            RichText::new("✓ В очереди").color(theme::ACTIVE_GREEN).size(11.0),
                                        )
                                        .fill(theme::BG_DARK)
                                        .stroke(Stroke::new(1.0, theme::ACTIVE_GREEN));
                                        if ui.add(btn).on_hover_text("Убрать из очереди").clicked() {
                                            card_consumed = true;
                                            let rid = item.id;
                                            self.queue.retain(|(id, _)| *id != rid);
                                        }
                                    } else {
                                        let btn = egui::Button::new(
                                            RichText::new("+ Добавить").color(theme::TEXT_PRIMARY).size(11.0),
                                        )
                                        .fill(theme::HEADER_LEFT)
                                        .stroke(Stroke::new(1.0, theme::BORDER_ACCENT));
                                        if ui.add(btn).clicked() {
                                            card_consumed = true;
                                            self.queue.push((item.id, item.title.clone()));
                                        }
                                    }
                                });
                            });
                        });

                    // Клик по карточке → открыть в Steam
                    // Не используем ui.interact (он перехватывает клики у кнопок внутри фрейма).
                    // Вместо этого проверяем hover + отпускание мыши вручную.
                    if frame_resp.response.hovered()
                        && ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary))
                        && !card_consumed
                    {
                        open_url(item.id);
                    }

                    if frame_resp.response.hovered() && !in_queue {
                        ui.painter().rect_stroke(
                            frame_resp.response.rect, 0.0,
                            Stroke::new(1.0, theme::BORDER),
                            egui::epaint::StrokeKind::Outside,
                        );
                    }
                    ui.add_space(2.0);
                }
            });

        // Очередь
        if !self.queue.is_empty() {
            ui.separator();
            Frame::NONE
                .fill(theme::BG_HEADER)
                .inner_margin(Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Очередь: {}  ", self.queue.len()))
                                .color(theme::TEXT_MUTED).size(11.0),
                        );
                        egui::ScrollArea::horizontal()
                            .id_salt("wsbrowser_queue_tags")
                            .max_height(26.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let snap = self.queue.clone();
                                    for (id, title) in &snap {
                                        let short: String = title.chars().take(22).collect();
                                        let short = if title.chars().count() > 22 {
                                            format!("{}…", short)
                                        } else {
                                            short
                                        };
                                        let tag = egui::Button::new(
                                            RichText::new(format!("× {short}"))
                                                .color(theme::TEXT_MUTED).size(10.5),
                                        )
                                        .fill(theme::BG_DARK)
                                        .stroke(Stroke::new(1.0, theme::BORDER));
                                        if ui.add(tag).on_hover_text(format!("Убрать {id}")).clicked() {
                                            let rid = *id;
                                            self.queue.retain(|(qid, _)| *qid != rid);
                                        }
                                        ui.add_space(3.0);
                                    }
                                });
                            });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let dl_btn = egui::Button::new(
                                RichText::new("⬇  Скачать через SteamCMD").color(theme::TEXT_PRIMARY).size(11.5),
                            )
                            .fill(theme::HEADER_LEFT)
                            .stroke(Stroke::new(1.0, theme::BORDER_ACCENT));
                            if ui.add(dl_btn).clicked() {
                                to_download = Some(self.queue.iter().map(|(id, _)| *id).collect());
                                self.queue.clear();
                            }
                            ui.add_space(6.0);
                            if ui.button(RichText::new("× Очистить").color(theme::TEXT_MUTED).size(11.0)).clicked() {
                                self.queue.clear();
                            }
                        });
                    });
                });
        }

        to_download
    }

    // ── Вкладка Сборки ───────────────────────────────────────────────────────

    fn content_collections(&mut self, ui: &mut egui::Ui) -> Option<Vec<u64>> {
        // Авто-загрузка первой страницы
        if !self.coll_auto_loaded {
            self.coll_auto_loaded = true;
            self.trigger_coll_fetch();
        }

        let mut do_coll_fetch = false;
        Frame::NONE
            .fill(theme::BG_HEADER)
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let resp = ui.add_sized(
                        [280.0, 22.0],
                        egui::TextEdit::singleline(&mut self.coll_search)
                            .hint_text("Поиск сборок RimWorld..."),
                    );
                    let search = ui.button(RichText::new("🔍").size(12.0))
                        .on_hover_text("Найти").clicked()
                        || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if search { self.coll_page = 1; do_coll_fetch = true; }

                    ui.add_space(6.0);
                    if sort_combo(ui, "wsbrowser_coll_sort", &mut self.coll_sort) {
                        self.coll_page = 1;
                        do_coll_fetch = true;
                    }
                    ui.add_space(8.0);
                    if pagination(ui, &mut self.coll_page, self.coll_has_prev, self.coll_has_next) {
                        do_coll_fetch = true;
                    }

                    if matches!(&self.coll_state, CollBrowseState::Loading) || self.coll_dl_rx.is_some() {
                        ui.add_space(8.0);
                        ui.spinner();
                    }
                });
            });
        if do_coll_fetch { self.trigger_coll_fetch(); }

        ui.add_space(2.0);

        // Уведомление
        if let Some(notif) = &self.coll_notif.clone() {
            Frame::NONE
                .fill(theme::BG_DARK)
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(notif).color(theme::ACTIVE_GREEN).size(11.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(RichText::new("×").color(theme::TEXT_MUTED)).clicked() {
                                self.coll_notif = None;
                            }
                        });
                    });
                });
            ui.add_space(2.0);
        }

        // Снапшоты состояния
        let items_snap: Option<Vec<CollectionItem>> = match &self.coll_state {
            CollBrowseState::Done(v) => Some(v.clone()),
            _ => None,
        };
        let err_msg: Option<String> = match &self.coll_state {
            CollBrowseState::Error(e) => Some(e.clone()),
            _ => None,
        };
        let is_idle    = matches!(&self.coll_state, CollBrowseState::Idle);
        let is_loading = matches!(&self.coll_state, CollBrowseState::Loading);

        let queue_height = if self.queue.is_empty() { 0.0 } else { 72.0 };
        let results_h = (ui.available_height() - queue_height - 4.0).max(80.0);

        egui::ScrollArea::vertical()
            .id_salt("wsbrowser_coll_results")
            .max_height(results_h)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                if is_idle || is_loading {
                    if is_idle {
                        ui.add_space(50.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new("Нажмите 🔍 для просмотра популярных сборок")
                                    .color(theme::TEXT_MUTED).size(12.0).italics(),
                            );
                        });
                    }
                    return;
                }

                if let Some(e) = err_msg {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(format!("× {e}")).color(theme::ERROR_RED).size(11.0));
                    });
                    return;
                }

                let Some(items) = items_snap else { return };

                if items.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("Ничего не найдено").color(theme::TEXT_MUTED).size(12.0).italics(),
                        );
                    });
                    return;
                }

                for item in &items {
                    self.coll_images.request(&item.preview_url);
                }

                let w = ui.available_width();
                let dl_busy = self.coll_dl_for.is_some();

                for item in &items {
                    let is_downloading = self.coll_dl_for.as_ref().map(|(id, _)| *id) == Some(item.id);
                    let mut card_consumed = false;
                    let frame_resp = Frame::NONE
                        .fill(theme::BG_ROW_EVEN)
                        .inner_margin(Margin::symmetric(8, 5))
                        .show(ui, |ui| {
                            ui.set_width(w - 16.0);
                            ui.horizontal(|ui| {
                                // Превью
                                let img_size = Vec2::new(144.0, 144.0);
                                if let Some(tex) = self.coll_images.get(&item.preview_url) {
                                    ui.add(egui::Image::new(tex).fit_to_exact_size(img_size));
                                } else {
                                    img_placeholder(ui, img_size);
                                }
                                ui.add_space(8.0);

                                // Инфо
                                ui.vertical(|ui| {
                                    ui.set_width(ui.available_width() - 160.0);
                                    ui.label(RichText::new(&item.title).color(theme::TEXT_PRIMARY).size(12.5).strong());
                                    ui.label(
                                        RichText::new(format!("by {}  •  ID: {}", item.author, item.id))
                                            .color(theme::TEXT_MUTED).size(10.5),
                                    );
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new("Нажмите «Скачать сборку» чтобы добавить не установленные моды в очередь и сохранить список")
                                            .color(theme::TEXT_MUTED).size(10.0).italics(),
                                    );
                                });

                                // Кнопка
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if is_downloading {
                                        ui.spinner();
                                        ui.label(RichText::new("Загрузка...").color(theme::TEXT_MUTED).size(11.0));
                                    } else {
                                        let btn = egui::Button::new(
                                            RichText::new("⬇ Скачать сборку")
                                                .color(theme::TEXT_PRIMARY).size(11.0),
                                        )
                                        .fill(theme::HEADER_LEFT)
                                        .stroke(Stroke::new(1.0, theme::BORDER_ACCENT));
                                        if ui.add_enabled(!dl_busy, btn)
                                            .on_hover_text("Добавить не установленные моды в очередь и сохранить список")
                                            .clicked()
                                        {
                                            card_consumed = true;
                                            self.trigger_coll_download(item.id, item.title.clone());
                                        }
                                    }
                                });
                            });
                        });

                    // Клик по карточке → открыть страницу сборки в Steam
                    if frame_resp.response.hovered()
                        && ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary))
                        && !card_consumed
                    {
                        open_url(item.id);
                    }

                    if frame_resp.response.hovered() {
                        ui.painter().rect_stroke(
                            frame_resp.response.rect, 0.0,
                            Stroke::new(1.0, theme::BORDER),
                            egui::epaint::StrokeKind::Outside,
                        );
                    }
                    ui.add_space(2.0);
                }
            });

        // Очередь (общая для обеих вкладок)
        let mut to_download: Option<Vec<u64>> = None;
        if !self.queue.is_empty() {
            ui.separator();
            Frame::NONE
                .fill(theme::BG_HEADER)
                .inner_margin(Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Очередь: {}  ", self.queue.len()))
                                .color(theme::TEXT_MUTED).size(11.0),
                        );
                        egui::ScrollArea::horizontal()
                            .id_salt("wsbrowser_coll_queue_tags")
                            .max_height(26.0)
                            .max_width(ui.available_width() * 0.7)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let snap = self.queue.clone();
                                    for (id, title) in &snap {
                                        let short: String = title.chars().take(22).collect();
                                        let short = if title.chars().count() > 22 {
                                            format!("{}…", short)
                                        } else {
                                            short
                                        };
                                        let tag = egui::Button::new(
                                            RichText::new(format!("× {short}"))
                                                .color(theme::TEXT_MUTED).size(10.5),
                                        )
                                        .fill(theme::BG_DARK)
                                        .stroke(Stroke::new(1.0, theme::BORDER));
                                        if ui.add(tag).on_hover_text(format!("Убрать {id}")).clicked() {
                                            let rid = *id;
                                            self.queue.retain(|(qid, _)| *qid != rid);
                                        }
                                        ui.add_space(3.0);
                                    }
                                });
                            });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let dl_btn = egui::Button::new(
                                RichText::new("⬇  Скачать через SteamCMD").color(theme::TEXT_PRIMARY).size(11.5),
                            )
                            .fill(theme::HEADER_LEFT)
                            .stroke(Stroke::new(1.0, theme::BORDER_ACCENT));
                            if ui.add(dl_btn).clicked() {
                                to_download = Some(self.queue.iter().map(|(id, _)| *id).collect());
                                self.queue.clear();
                            }
                            ui.add_space(6.0);
                            if ui.button(RichText::new("× Очистить").color(theme::TEXT_MUTED).size(11.0)).clicked() {
                                self.queue.clear();
                            }
                        });
                    });
                });
        }

        to_download
    }
}

// ─── Вспомогательные функции ─────────────────────────────────────────────────

fn tab_btn(ui: &mut egui::Ui, label: &str, active: bool, on_click: impl FnOnce()) {
    let fill   = if active { theme::BG_HEADER } else { theme::BG_DARK };
    let color  = if active { theme::TEXT_ACCENT } else { theme::TEXT_MUTED };
    let border = if active { theme::BORDER_ACCENT } else { theme::BORDER };
    let btn = egui::Button::new(RichText::new(label).color(color).size(12.0))
        .fill(fill)
        .stroke(Stroke::new(1.0, border));
    if ui.add(btn).clicked() { on_click(); }
}

/// Возвращает true если выбор изменился.
fn sort_combo(ui: &mut egui::Ui, id: &str, sort: &mut SortOrder) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(RichText::new(sort.label()).color(theme::TEXT_MUTED).size(11.0))
        .show_ui(ui, |ui| {
            for s in SortOrder::ALL {
                if ui.selectable_label(*sort == s, RichText::new(s.label()).size(11.0)).clicked() {
                    *sort = s;
                    changed = true;
                }
            }
        });
    changed
}

/// Возвращает true если страница изменилась.
fn pagination(ui: &mut egui::Ui, page: &mut u32, has_prev: bool, has_next: bool) -> bool {
    let mut changed = false;
    if ui.add_enabled(has_prev, egui::Button::new(RichText::new("◀").color(theme::TEXT_MUTED).size(11.0))).clicked() {
        *page -= 1;
        changed = true;
    }
    ui.label(RichText::new(format!("  стр {}  ", page)).color(theme::TEXT_MUTED).size(11.0));
    if ui.add_enabled(has_next, egui::Button::new(RichText::new("▶").color(theme::TEXT_MUTED).size(11.0))).clicked() {
        *page += 1;
        changed = true;
    }
    changed
}

fn img_placeholder(ui: &mut egui::Ui, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, 4.0, theme::BG_DARK);
    ui.painter().text(
        rect.center(), egui::Align2::CENTER_CENTER, "…",
        egui::FontId::monospace(14.0), theme::TEXT_MUTED,
    );
}

fn open_url(id: u64) {
    let url = format!("https://steamcommunity.com/sharedfiles/filedetails/?id={}", id);
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/c", "start", "", &url]).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&url).spawn();
}

/// Сохраняет XML-список Workshop ID сборки в папку modlist.
/// Файл совместим с импортом по Workshop ID.
fn save_collection_file(title: &str, items: &[WorkshopItem]) -> Option<std::path::PathBuf> {
    let dir = directories::ProjectDirs::from("com", "rustrim", "RustRim")
        .map(|d| d.data_dir().join("modlist"))?;
    let _ = std::fs::create_dir_all(&dir);

    let safe_title: String = title.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .take(60)
        .collect();
    let safe_title = if safe_title.is_empty() { "Collection".to_string() } else { safe_title };

    let path = dir.join(format!("{}.xml", safe_title));

    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str("<ModsConfigData>\n\t<version>1.0</version>\n\t<activeMods>\n");
    for item in items {
        out.push_str(&format!("\t\t<li>{}</li>\n", item.id));
    }
    out.push_str("\t</activeMods>\n</ModsConfigData>\n");

    std::fs::write(&path, out).ok()?;
    Some(path)
}
