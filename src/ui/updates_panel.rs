//! Окно обновлений: что устарело и что из этого перекачать.

use std::collections::HashSet;

use egui::{Align, Frame, Margin, RichText, ScrollArea, Stroke, Window};

use crate::job::Job;
use crate::mod_data::{ModDb, ModId};
use crate::updates::Update;
use crate::ui::{fit_width, theme};

#[derive(Default)]
pub struct UpdatesUi {
    pub open: bool,
    /// Запрос к мастерской: на живой установке это ~20 секунд, поэтому фоном.
    pub job: Job<Vec<Update>>,
    /// Что пользователь отметил к обновлению.
    chosen: HashSet<ModId>,
}

pub enum Reply {
    None,
    /// Спросить мастерскую заново.
    Check,
    /// Выделить мод в списке.
    Select(ModId),
    /// Поставить в очередь SteamCMD.
    Download(Vec<u64>),
}

impl UpdatesUi {
    /// Сбрасывает выбор перед новым запросом.
    pub fn reset(&mut self) {
        self.chosen.clear();
    }

    /// По умолчанию отмечено всё найденное: обычно обновляют пачкой.
    pub fn select_all(&mut self, updates: &[Update]) {
        self.chosen = updates.iter().map(|u| u.id.clone()).collect();
    }
}

pub fn show(ctx: &egui::Context, state: &mut UpdatesUi, db: &ModDb) -> Reply {
    if !state.open {
        return Reply::None;
    }
    let mut reply = Reply::None;
    let mut open = true;

    Window::new("⇧  Обновления модов")
        .open(&mut open)
        .collapsible(true)
        .resizable(true)
        .default_width(700.0)
        .default_height(520.0)
        .frame(
            Frame::window(&ctx.global_style())
                .fill(theme::BG_PANEL)
                .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)),
        )
        .show(ctx, |ui| body(ui, state, db, &mut reply));

    state.open = open;
    reply
}

fn body(ui: &mut egui::Ui, state: &mut UpdatesUi, db: &ModDb, reply: &mut Reply) {
    match &state.job {
        Job::Idle => {
            ui.label(
                RichText::new(
                    "Свежесть мода определяется по времени его обновления в мастерской: \
                     номеру версии из About.xml верить нельзя, авторы его почти не поднимают.\n\n\
                     Локальная сторона — время файлов на диске, поэтому проверка работает \
                     и для модов, поставленных до этого лаунчера.",
                )
                .color(theme::TEXT_MUTED)
                .size(11.5),
            );
            ui.add_space(8.0);
            if ui.button("Спросить мастерскую").clicked() {
                *reply = Reply::Check;
            }
        }
        Job::Running(_) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new("Спрашиваем мастерскую…")
                        .color(theme::TEXT_PRIMARY)
                        .size(11.5),
                );
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("Моды спрашиваются сотнями за запрос; на большой сборке это секунд двадцать.")
                    .color(theme::TEXT_MUTED)
                    .size(10.5),
            );
        }
        Job::Failed(err) => {
            ui.label(RichText::new(format!("⚠ {err}")).color(theme::WARNING_AMBER).size(11.5));
            ui.add_space(6.0);
            if ui.button("⟳ Ещё раз").clicked() {
                *reply = Reply::Check;
            }
        }
        Job::Done(updates) if updates.is_empty() => {
            ui.label(
                RichText::new("Все моды из мастерской свежие.")
                    .color(theme::ACTIVE_GREEN)
                    .size(11.5),
            );
            ui.add_space(6.0);
            if ui.button("⟳ Проверить ещё раз").clicked() {
                *reply = Reply::Check;
            }
        }
        Job::Done(_) => found(ui, state, db, reply),
    }
}

fn found(ui: &mut egui::Ui, state: &mut UpdatesUi, db: &ModDb, reply: &mut Reply) {
    // Список нужен и для отрисовки, и для кнопок — берём копию, чтобы не
    // держать заимствование job во время правки chosen.
    let updates: Vec<Update> = state.job.result().cloned().unwrap_or_default();

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("Устарело модов: {}", updates.len()))
                .color(theme::TEXT_PRIMARY)
                .size(12.0)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if ui.button("⟳").on_hover_text("Спросить заново").clicked() {
                *reply = Reply::Check;
            }
            if ui.button("Снять всё").clicked() {
                state.chosen.clear();
            }
            if ui.button("Отметить всё").clicked() {
                state.chosen = updates.iter().map(|u| u.id.clone()).collect();
            }
        });
    });

    ui.add_space(6.0);
    ScrollArea::vertical()
        .id_salt("updates_list")
        .auto_shrink([false, false])
        .max_height(360.0)
        .show(ui, |ui| {
            for update in &updates {
                row(ui, state, db, update, reply);
                ui.add_space(3.0);
            }
        });

    ui.add_space(6.0);
    let chosen: Vec<u64> = updates
        .iter()
        .filter(|u| state.chosen.contains(&u.id))
        .map(|u| u.workshop_id)
        .collect();
    let label = format!("⬇ Обновить отмеченные ({})", chosen.len());
    if ui
        .add_enabled(!chosen.is_empty(), egui::Button::new(RichText::new(label).size(11.5)))
        .on_hover_text("Поставить в очередь SteamCMD и открыть её")
        .clicked()
    {
        *reply = Reply::Download(chosen);
    }
}

fn row(
    ui: &mut egui::Ui,
    state: &mut UpdatesUi,
    db: &ModDb,
    update: &Update,
    reply: &mut Reply,
) {
    Frame::new()
        .fill(theme::BG_ROW_EVEN)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(fit_width(ui));
            ui.horizontal(|ui| {
                let mut on = state.chosen.contains(&update.id);
                if ui.checkbox(&mut on, "").changed() {
                    if on {
                        state.chosen.insert(update.id.clone());
                    } else {
                        state.chosen.remove(&update.id);
                    }
                }

                let name = db
                    .get(&update.id)
                    .map(|m| m.name.as_str())
                    .unwrap_or(update.title.as_str());
                if ui
                    .add(
                        egui::Label::new(
                            RichText::new(name).color(theme::TEXT_PRIMARY).size(11.5),
                        )
                        .truncate()
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_text(update.id.as_str())
                    .clicked()
                {
                    *reply = Reply::Select(update.id.clone());
                }

                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let days = update.behind().as_secs() / 86_400;
                    let color = if days > 180 {
                        theme::ERROR_RED
                    } else if days > 30 {
                        theme::WARNING_AMBER
                    } else {
                        theme::TEXT_MUTED
                    };
                    ui.label(
                        RichText::new(format!("отстал на {days} дн."))
                            .color(color)
                            .size(10.5),
                    );
                });
            });
        });
}
