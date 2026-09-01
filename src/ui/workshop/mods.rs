//! Вкладка «Моды»: поиск по мастерской и набор очереди на скачивание.

use std::collections::HashSet;

use egui::{Align, Layout, RichText, Stroke};

use crate::steam::workshop_api::WorkshopItem;
use crate::ui::{fit_width_minus, theme};

use super::browse::{self, Browse, Card};
use super::queue::Queue;

pub fn show(
    ui: &mut egui::Ui,
    browse_state: &mut Browse<WorkshopItem>,
    queue: &mut Queue,
    installed: &HashSet<u64>,
    reserved_height: f32,
) {
    browse_state.ensure_started();

    if browse::search_bar(
        ui,
        browse_state,
        "Поиск модов RimWorld...",
        "wsbrowser",
        false,
    ) {
        browse_state.fetch();
    }
    ui.add_space(2.0);

    let items = browse_state.snapshot();
    for item in &items {
        browse_state.images.request(item.preview_url());
    }

    let height = (ui.available_height() - reserved_height).max(80.0);
    let status = browse_state.status();
    browse::results(
        ui,
        "wsbrowser_results",
        height,
        status,
        items.len(),
        "Нажмите ⊙ для просмотра популярных модов",
        |ui, i| card(ui, &items[i], browse_state, queue, installed),
    );
}

fn card(
    ui: &mut egui::Ui,
    item: &WorkshopItem,
    browse_state: &Browse<WorkshopItem>,
    queue: &mut Queue,
    installed: &HashSet<u64>,
) {
    let in_queue = queue.contains(item.id);
    let is_installed = installed.contains(&item.id);
    let fill = if in_queue { theme::BG_SELECTED } else { theme::BG_ROW_EVEN };

    // Что нажали внутри карточки — решается здесь, а применяется после неё:
    // очередь нельзя менять, пока по ней идёт отрисовка.
    let mut toggle = false;

    browse::card(ui, item.id, fill, !in_queue, |ui| {
        ui.horizontal(|ui| {
            browse::preview(ui, &browse_state.images, &item.preview_url);
            ui.add_space(8.0);

            ui.vertical(|ui| {
                ui.set_width(fit_width_minus(ui, 110.0));
                ui.label(
                    RichText::new(&item.title).color(theme::TEXT_PRIMARY).size(12.5).strong(),
                );
                ui.label(
                    RichText::new(format!("by {}  •  ID: {}", item.author, item.id))
                        .color(theme::TEXT_MUTED)
                        .size(10.5),
                );
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if is_installed {
                    ui.label(
                        RichText::new("✓ Установлено").color(theme::ACTIVE_GREEN).size(11.0),
                    );
                } else if in_queue {
                    let btn = egui::Button::new(
                        RichText::new("✓ В очереди").color(theme::ACTIVE_GREEN).size(11.0),
                    )
                    .fill(theme::BG_DARK)
                    .stroke(Stroke::new(1.0, theme::ACTIVE_GREEN));
                    toggle = ui.add(btn).on_hover_text("Убрать из очереди").clicked();
                } else {
                    let btn = egui::Button::new(
                        RichText::new("+ Добавить").color(theme::TEXT_PRIMARY).size(11.0),
                    )
                    .fill(theme::HEADER_LEFT)
                    .stroke(Stroke::new(1.0, theme::BORDER_ACCENT));
                    toggle = ui.add(btn).clicked();
                }
            });
        });
        toggle
    });

    if toggle {
        if in_queue {
            queue.remove(item.id);
        } else {
            queue.add(item.id, item.title.clone());
        }
    }
}
