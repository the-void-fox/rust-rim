//! Вкладка «Сборки»: поиск коллекций и постановка их содержимого в очередь.

use std::collections::HashSet;

use egui::{Align, Frame, Layout, Margin, RichText, Stroke};

use crate::job::Job;
use crate::steam::workshop_api::{self, CollectionItem, WorkshopItem};
use crate::ui::{fit_width_minus, theme};

use super::browse::{self, Browse, Card};
use super::queue::Queue;

/// Состояние вкладки: выдача плюс скачивание содержимого одной сборки.
pub struct Collections {
    pub browse: Browse<CollectionItem>,
    /// Какую сборку сейчас разворачиваем: (id, название из карточки).
    downloading: Option<(u64, String)>,
    job: Job<(String, Vec<WorkshopItem>)>,
    notice: Option<String>,
}

impl Default for Collections {
    fn default() -> Self {
        Self::new()
    }
}

impl Collections {
    pub fn new() -> Self {
        Self {
            browse: Browse::new(workshop_api::fetch_collections_page),
            downloading: None,
            job: Job::Idle,
            notice: None,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.browse.is_busy() || self.job.is_running()
    }

    /// Забирает содержимое скачанной сборки. `true` — что-то изменилось.
    pub fn poll(
        &mut self,
        ctx: &egui::Context,
        queue: &mut Queue,
        installed: &HashSet<u64>,
    ) -> bool {
        let browsed = self.browse.poll(ctx);
        if !self.job.poll() {
            return browsed;
        }
        // Название берём из карточки: API его не возвращает.
        let title = self.downloading.take().map(|(_, t)| t).unwrap_or_default();
        match std::mem::replace(&mut self.job, Job::Idle) {
            Job::Failed(e) => self.notice = Some(format!("× Ошибка: {e}")),
            Job::Done((_api_title, items)) => {
                let saved = save_collection_file(&title, &items);
                let added = items
                    .iter()
                    .filter(|it| !installed.contains(&it.id) && !queue.contains(it.id))
                    .map(|it| (it.id, it.title.clone()))
                    .collect::<Vec<_>>();
                let count = added.len();
                for (id, name) in added {
                    queue.add(id, name);
                }
                let file = saved
                    .as_ref()
                    .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
                    .unwrap_or_else(|| "не сохранено".into());
                self.notice = Some(format!(
                    "✓ Сборка «{title}»\n  В очередь добавлено: {count} мод(ов)\n  Файл: {file}"
                ));
            }
            other => self.job = other,
        }
        true
    }

    fn start_download(&mut self, id: u64, title: String) {
        if self.downloading.is_some() {
            return;
        }
        self.downloading = Some((id, title));
        self.job = Job::spawn(move || {
            workshop_api::fetch_collection_mods(id).map_err(|e| e.to_string())
        });
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut Collections, reserved_height: f32) {
    state.browse.ensure_started();

    let busy = state.job.is_running();
    if browse::search_bar(
        ui,
        &mut state.browse,
        "Поиск сборок RimWorld...",
        "wsbrowser_coll",
        busy,
    ) {
        state.browse.fetch();
    }
    ui.add_space(2.0);

    if let Some(text) = state.notice.clone() {
        notice(ui, &text, &mut state.notice);
        ui.add_space(2.0);
    }

    let items = state.browse.snapshot();
    for item in &items {
        state.browse.images.request(item.preview_url());
    }

    let height = (ui.available_height() - reserved_height).max(80.0);
    let status = state.browse.status();
    // Что нажали — решается при отрисовке, запускается после: start_download
    // требует &mut, а карточки держат состояние взаймы.
    let mut requested: Option<(u64, String)> = None;
    browse::results(
        ui,
        "wsbrowser_coll_results",
        height,
        status,
        items.len(),
        "Нажмите ⊙ для просмотра популярных сборок",
        |ui, i| {
            if let Some(pick) = card(ui, &items[i], state) {
                requested = Some(pick);
            }
        },
    );
    if let Some((id, title)) = requested {
        state.start_download(id, title);
    }
}

fn notice(ui: &mut egui::Ui, text: &str, slot: &mut Option<String>) {
    Frame::NONE
        .fill(theme::BG_DARK)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(text).color(theme::ACTIVE_GREEN).size(11.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button(RichText::new("×").color(theme::TEXT_MUTED)).clicked() {
                        *slot = None;
                    }
                });
            });
        });
}

/// Возвращает сборку, которую попросили развернуть.
fn card(
    ui: &mut egui::Ui,
    item: &CollectionItem,
    state: &Collections,
) -> Option<(u64, String)> {
    let is_downloading = state.downloading.as_ref().map(|(id, _)| *id) == Some(item.id);
    let busy = state.downloading.is_some();
    let mut picked = None;

    browse::card(ui, item.id, theme::BG_ROW_EVEN, true, |ui| {
        let mut consumed = false;
        ui.horizontal(|ui| {
            browse::preview(ui, &state.browse.images, &item.preview_url);
            ui.add_space(8.0);

            ui.vertical(|ui| {
                ui.set_width(fit_width_minus(ui, 160.0));
                ui.label(
                    RichText::new(&item.title).color(theme::TEXT_PRIMARY).size(12.5).strong(),
                );
                ui.label(
                    RichText::new(format!("by {}  •  ID: {}", item.author, item.id))
                        .color(theme::TEXT_MUTED)
                        .size(10.5),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Нажмите «Скачать сборку» чтобы добавить не установленные моды \
                         в очередь и сохранить список",
                    )
                    .color(theme::TEXT_MUTED)
                    .size(10.0)
                    .italics(),
                );
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if is_downloading {
                    ui.spinner();
                    ui.label(RichText::new("Загрузка...").color(theme::TEXT_MUTED).size(11.0));
                } else {
                    let btn = egui::Button::new(
                        RichText::new("⬇ Скачать сборку").color(theme::TEXT_PRIMARY).size(11.0),
                    )
                    .fill(theme::HEADER_LEFT)
                    .stroke(Stroke::new(1.0, theme::BORDER_ACCENT));
                    if ui
                        .add_enabled(!busy, btn)
                        .on_hover_text("Добавить не установленные моды в очередь и сохранить список")
                        .clicked()
                    {
                        consumed = true;
                        picked = Some((item.id, item.title.clone()));
                    }
                }
            });
        });
        consumed
    });

    picked
}

/// Сохраняет XML-список Workshop ID сборки в папку modlist.
/// Файл совместим с импортом по Workshop ID.
fn save_collection_file(title: &str, items: &[WorkshopItem]) -> Option<std::path::PathBuf> {
    let dir = directories::ProjectDirs::from("com", "rustrim", "RustRim")
        .map(|d| d.data_dir().join("modlist"))?;
    let _ = std::fs::create_dir_all(&dir);

    let safe: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .take(60)
        .collect();
    let safe = if safe.is_empty() { "Collection".to_string() } else { safe };
    let path = dir.join(format!("{safe}.xml"));

    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str("<ModsConfigData>\n\t<version>1.0</version>\n\t<activeMods>\n");
    for item in items {
        out.push_str(&format!("\t\t<li>{}</li>\n", item.id));
    }
    out.push_str("\t</activeMods>\n</ModsConfigData>\n");

    std::fs::write(&path, out).ok()?;
    Some(path)
}
