//! Диалоги про дублирующиеся моды: список найденных копий, подтверждение
//! удаления и уведомление о результате.

use egui::{Align2, RichText, ScrollArea, Window};

use crate::mod_data::ModDb;
use crate::ui::theme;

/// Состояние диалогов дубликатов.
#[derive(Default)]
pub struct DuplicatesUi {
    /// Показывать список найденных дубликатов.
    pub show_list: bool,
    /// Показывать подтверждение безвозвратного удаления.
    pub confirm: bool,
    /// Сколько папок удалено в прошлый раз (0 — уведомления нет).
    pub last_removed: usize,
}

/// Рисует все три окна. Возвращает `true`, если пользователь подтвердил
/// удаление лишних копий с диска.
pub fn show(ctx: &egui::Context, state: &mut DuplicatesUi, db: &ModDb) -> bool {
    show_list(ctx, state, db);
    show_notification(ctx, state);
    show_confirmation(ctx, state)
}

fn show_list(ctx: &egui::Context, state: &mut DuplicatesUi, db: &ModDb) {
    if !state.show_list {
        return;
    }
    let mut open = true;
    let count = db.duplicates().len();

    Window::new("Обнаружены дубликаты модов")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(
                RichText::new(format!("Найдено {count} мод(ов) с дублирующимися package ID:"))
                    .color(theme::WARNING_AMBER),
            );
            ui.add_space(6.0);
            ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                for group in db.duplicates() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("×{}", group.discarded.len() + 1))
                                .color(theme::ERROR_RED)
                                .size(11.0)
                                .strong(),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(group.id.as_str())
                                .color(theme::TEXT_ACCENT)
                                .size(11.0),
                        )
                        .on_hover_text(format!(
                            "оставлен:\n{}\n\nбудут удалены:\n{}",
                            group.kept.display(),
                            group
                                .discarded
                                .iter()
                                .map(|p| p.display().to_string())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ));
                    });
                }
            });
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Удалить дубликаты (оставить первый)").clicked() {
                    state.show_list = false;
                    state.confirm = true;
                }
                ui.add_space(8.0);
                if ui.button("Закрыть").clicked() {
                    state.show_list = false;
                }
            });
        });

    if !open {
        state.show_list = false;
    }
}

fn show_notification(ctx: &egui::Context, state: &mut DuplicatesUi) {
    if state.last_removed == 0 {
        return;
    }
    Window::new("Готово")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(
                RichText::new(format!(
                    "Удалено {} дублирующихся мод(ов).",
                    state.last_removed
                ))
                .color(theme::ACTIVE_GREEN),
            );
            ui.add_space(6.0);
            if ui.button("OK").clicked() {
                state.last_removed = 0;
            }
        });
}

fn show_confirmation(ctx: &egui::Context, state: &mut DuplicatesUi) -> bool {
    if !state.confirm {
        return false;
    }
    let mut confirmed = false;

    Window::new("Подтверждение удаления")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(RichText::new("ВНИМАНИЕ!").color(theme::ERROR_RED).strong());
            ui.label("Вы собираетесь безвозвратно удалить папки дублирующихся модов с диска.");
            ui.label("Отменить это действие будет невозможно.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Да, удалить").clicked() {
                    state.confirm = false;
                    confirmed = true;
                }
                if ui.button("Отмена").clicked() {
                    state.confirm = false;
                }
            });
        });

    confirmed
}
