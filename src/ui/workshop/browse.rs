//! Общее для обеих вкладок: поиск, сортировка, страницы и запрос к мастерской.
//!
//! Вкладки «Моды» и «Сборки» устроены одинаково — та же строка поиска, та же
//! разбивка по страницам, те же четыре состояния списка. Раньше всё это было
//! написано дважды, поэтому здесь оно ровно один раз, а вкладкам остаётся
//! только отрисовка своей карточки.

use egui::{Frame, Margin, RichText, Stroke, Vec2};

use crate::job::Job;
use crate::steam::workshop_api::{CollectionItem, SortOrder, WorkshopItem};
use crate::ui::{fit_width, theme};

use super::image_cache::ImageCache;

/// Элемент выдачи. Общего кода на моды и сборки ровно столько: список должен
/// уметь заказать превью, не зная, что именно показывает.
pub trait Card {
    fn preview_url(&self) -> &str;
}

impl Card for WorkshopItem {
    fn preview_url(&self) -> &str { &self.preview_url }
}

impl Card for CollectionItem {
    fn preview_url(&self) -> &str { &self.preview_url }
}

/// Как получить страницу результатов. У обоих запросов одинаковая форма.
type Fetcher<T> = fn(&str, u32, SortOrder) -> anyhow::Result<(Vec<T>, bool)>;

/// Состояние списка для отрисовки.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status<'a> {
    /// Ещё не спрашивали.
    Idle,
    Loading,
    Error(&'a str),
    Ready,
}

/// Поисковая выдача одной вкладки.
pub struct Browse<T: 'static> {
    pub search: String,
    pub sort: SortOrder,
    pub page: u32,
    pub has_prev: bool,
    pub has_next: bool,
    pub images: ImageCache,
    job: Job<(Vec<T>, bool)>,
    items: Vec<T>,
    error: Option<String>,
    fetcher: Fetcher<T>,
    /// Первую страницу подгружаем сами, но ровно один раз.
    started: bool,
}

impl<T: Card + Clone + Send + 'static> Browse<T> {
    pub fn new(fetcher: Fetcher<T>) -> Self {
        Self {
            search: String::new(),
            sort: SortOrder::Trending,
            page: 1,
            has_prev: false,
            has_next: false,
            images: ImageCache::new(),
            job: Job::Idle,
            items: Vec::new(),
            error: None,
            fetcher,
            started: false,
        }
    }

    /// Забирает пришедший ответ и текстуры. `true` — что-то изменилось.
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        self.images.poll(ctx);
        if !self.job.poll() {
            return false;
        }
        match std::mem::replace(&mut self.job, Job::Idle) {
            Job::Done((items, has_next)) => {
                self.items = items;
                self.has_next = has_next;
                self.error = None;
            }
            Job::Failed(e) => {
                self.error = Some(e);
                self.items.clear();
            }
            other => self.job = other,
        }
        true
    }

    /// Идёт ли сейчас работа — нужно для перерисовки по таймеру.
    pub fn is_busy(&self) -> bool {
        self.job.is_running() || self.images.is_busy()
    }

    pub fn status(&self) -> Status<'_> {
        if self.job.is_running() {
            Status::Loading
        } else if let Some(e) = &self.error {
            Status::Error(e)
        } else if !self.started {
            Status::Idle
        } else {
            Status::Ready
        }
    }

    /// Копия текущих результатов: отрисовка карточек требует одновременного
    /// доступа к кэшу картинок, а он лежит здесь же.
    pub fn snapshot(&self) -> Vec<T> {
        self.items.clone()
    }

    /// Запускает запрос страницы.
    pub fn fetch(&mut self) {
        let (query, page, sort, fetcher) = (self.search.clone(), self.page, self.sort, self.fetcher);
        self.started = true;
        self.has_prev = page > 1;
        self.error = None;
        self.job = Job::spawn(move || fetcher(&query, page, sort).map_err(|e| e.to_string()));
    }

    /// Подгружает первую страницу при первом показе вкладки.
    pub fn ensure_started(&mut self) {
        if !self.started {
            self.fetch();
        }
    }
}

/// Строка поиска, сортировка и страницы. `true` — надо перезапросить.
pub fn search_bar<T: Card + Clone + Send + 'static>(
    ui: &mut egui::Ui,
    browse: &mut Browse<T>,
    hint: &str,
    id_salt: &str,
    extra_busy: bool,
) -> bool {
    let mut refetch = false;
    Frame::NONE
        .fill(theme::BG_HEADER)
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let resp = ui.add_sized(
                    [280.0, 22.0],
                    egui::TextEdit::singleline(&mut browse.search).hint_text(hint),
                );
                let search = ui
                    .button(RichText::new("⊙").size(12.0))
                    .on_hover_text("Найти")
                    .clicked()
                    || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if search {
                    browse.page = 1;
                    refetch = true;
                }

                ui.add_space(6.0);
                if sort_combo(ui, &format!("{id_salt}_sort"), &mut browse.sort) {
                    browse.page = 1;
                    refetch = true;
                }
                ui.add_space(8.0);
                if pagination(ui, &mut browse.page, browse.has_prev, browse.has_next) {
                    refetch = true;
                }

                if browse.job.is_running() || extra_busy {
                    ui.add_space(8.0);
                    ui.spinner();
                }
            });
        });
    refetch
}

/// Оболочка списка: пустое состояние, ошибка, «ничего не найдено».
/// Карточки рисует `draw`, получая индекс элемента.
pub fn results(
    ui: &mut egui::Ui,
    id_salt: &str,
    max_height: f32,
    status: Status<'_>,
    count: usize,
    idle_hint: &str,
    mut draw: impl FnMut(&mut egui::Ui, usize),
) {
    egui::ScrollArea::vertical()
        .id_salt(id_salt)
        .max_height(max_height)
        .show(ui, |ui| {
            ui.set_width(fit_width(ui));

            match status {
                Status::Loading => return,
                Status::Idle => {
                    ui.add_space(50.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(idle_hint)
                                .color(theme::TEXT_MUTED)
                                .size(12.0)
                                .italics(),
                        );
                    });
                    return;
                }
                Status::Error(e) => {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(format!("× {e}")).color(theme::ERROR_RED).size(11.0),
                        );
                    });
                    return;
                }
                Status::Ready => {}
            }

            if count == 0 {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Ничего не найдено")
                            .color(theme::TEXT_MUTED)
                            .size(12.0)
                            .italics(),
                    );
                });
                return;
            }

            for i in 0..count {
                draw(ui, i);
                ui.add_space(2.0);
            }
        });
}

/// Рамка карточки: подсветка при наведении и открытие страницы по клику мимо
/// кнопок. `inner` возвращает `true`, если клик уже израсходован кнопкой.
pub fn card(
    ui: &mut egui::Ui,
    id: u64,
    fill: egui::Color32,
    outline_on_hover: bool,
    inner: impl FnOnce(&mut egui::Ui) -> bool,
) {
    let width = fit_width(ui);
    let mut consumed = false;
    let frame = Frame::NONE
        .fill(fill)
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width((width - 16.0).max(0.0));
            consumed = inner(ui);
        });

    // Не используем ui.interact: он перехватывает клики у кнопок внутри
    // фрейма. Поэтому вручную — наведение плюс отпускание мыши.
    if frame.response.hovered()
        && ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary))
        && !consumed
    {
        open_url(id);
    }
    if frame.response.hovered() && outline_on_hover {
        ui.painter().rect_stroke(
            frame.response.rect,
            0.0,
            Stroke::new(1.0, theme::BORDER),
            egui::epaint::StrokeKind::Outside,
        );
    }
}

/// Превью карточки: текстура, если уже загрузилась, иначе заглушка.
pub fn preview(ui: &mut egui::Ui, images: &ImageCache, url: &str) {
    let size = Vec2::new(144.0, 144.0);
    match images.get(url) {
        Some(tex) => {
            ui.add(egui::Image::new(tex).fit_to_exact_size(size));
        }
        None => {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter().rect_filled(rect, 4.0, theme::BG_DARK);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "…",
                egui::FontId::monospace(14.0),
                theme::TEXT_MUTED,
            );
        }
    }
}

pub fn tab_btn(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    let fill = if active { theme::BG_HEADER } else { theme::BG_DARK };
    let color = if active { theme::TEXT_ACCENT } else { theme::TEXT_MUTED };
    let border = if active { theme::BORDER_ACCENT } else { theme::BORDER };
    let btn = egui::Button::new(RichText::new(label).color(color).size(12.0))
        .fill(fill)
        .stroke(Stroke::new(1.0, border));
    ui.add(btn).clicked()
}

/// `true`, если выбор изменился.
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

/// `true`, если страница изменилась.
fn pagination(ui: &mut egui::Ui, page: &mut u32, has_prev: bool, has_next: bool) -> bool {
    let mut changed = false;
    let back = egui::Button::new(RichText::new("◀").color(theme::TEXT_MUTED).size(11.0));
    if ui.add_enabled(has_prev, back).clicked() {
        *page -= 1;
        changed = true;
    }
    ui.label(RichText::new(format!("  стр {page}  ")).color(theme::TEXT_MUTED).size(11.0));
    let forward = egui::Button::new(RichText::new("▶").color(theme::TEXT_MUTED).size(11.0));
    if ui.add_enabled(has_next, forward).clicked() {
        *page += 1;
        changed = true;
    }
    changed
}

pub fn open_url(id: u64) {
    let url = format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}");
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/c", "start", "", &url]).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&url).spawn();
}
