//! Окно управления тегами: создание, переименование, цвет, удаление.

use std::collections::HashMap;

use egui::{Align2, Frame, RichText, Stroke, Window};

use crate::tags::{Rgb, TagId, Tags};
use crate::ui::theme;

#[derive(Default)]
pub struct TagsUi {
    pub open: bool,
    /// Имя для нового тега.
    new_name: String,
    /// Буферы редактирования имён: индекс тега → текст в поле.
    /// Правка применяется по Enter или потере фокуса, чтобы промежуточные
    /// состояния (в том числе пустая строка) не попадали в хранилище.
    buffers: HashMap<usize, String>,
    confirm_delete: Option<TagId>,
    /// Последняя отклонённая правка — показывается подсказкой.
    error: Option<String>,
}

/// Отложенная правка: применяется после обхода списка, чтобы не менять
/// `Tags` во время итерации по нему.
enum Op {
    Rename(TagId, String),
    Recolor(TagId, Rgb),
    Delete(TagId),
    Create(String),
}

/// Возвращает `true`, если теги изменились и их нужно сохранить.
pub fn show(ctx: &egui::Context, state: &mut TagsUi, tags: &mut Tags) -> bool {
    if !state.open {
        return false;
    }

    let mut ops: Vec<Op> = Vec::new();
    let mut open = true;

    Window::new(RichText::new("⚑  Теги").color(theme::TEXT_PRIMARY).size(13.0).strong())
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .min_width(440.0)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(
            Frame::window(&ctx.global_style())
                .fill(theme::BG_PANEL)
                .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)),
        )
        .show(ctx, |ui| body(ui, state, tags, &mut ops));

    state.open = open;
    if !open {
        state.confirm_delete = None;
        state.error = None;
    }

    apply(state, tags, ops)
}

fn body(ui: &mut egui::Ui, state: &mut TagsUi, tags: &Tags, ops: &mut Vec<Op>) {
    ui.label(
        RichText::new("Теги помечают моды цветом в списке. Фильтр в поиске: tag:имя")
            .color(theme::TEXT_MUTED)
            .size(10.5)
            .italics(),
    );
    ui.add_space(6.0);

    if tags.is_empty() {
        ui.label(
            RichText::new("Тегов пока нет — создайте первый ниже.")
                .color(theme::TEXT_MUTED)
                .size(11.5),
        );
    }

    egui::ScrollArea::vertical()
        .id_salt("tags_list")
        .max_height(320.0)
        .show(ui, |ui| {
            for id in tags.ids() {
                let Some(tag) = tags.get(id) else { continue };
                ui.horizontal(|ui| {
                    let mut color = tag.color;
                    if ui.color_edit_button_srgb(&mut color).changed() {
                        ops.push(Op::Recolor(id, color));
                    }

                    let buffer = state
                        .buffers
                        .entry(tag_index(id))
                        .or_insert_with(|| tag.name.clone());
                    // Внешнее переименование (например, отказ) возвращает поле
                    // к актуальному имени, пока в нём не печатают.
                    let edit = ui.add(
                        egui::TextEdit::singleline(buffer)
                            .id(egui::Id::new(("tag_name", tag_index(id))))
                            .desired_width(200.0)
                            .text_color(theme::TEXT_PRIMARY),
                    );
                    let commit = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        || edit.lost_focus();
                    if commit && *buffer != tag.name {
                        ops.push(Op::Rename(id, buffer.clone()));
                    }

                    let used = tags.usage(id);
                    ui.label(
                        RichText::new(format!("{used} мод(ов)"))
                            .color(theme::TEXT_MUTED)
                            .size(10.5),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button(RichText::new("🗑").color(theme::ERROR_RED))
                            .on_hover_text("Удалить тег")
                            .clicked()
                        {
                            state.confirm_delete = Some(id);
                        }
                    });
                });
            }
        });

    if let Some(err) = &state.error {
        ui.add_space(4.0);
        ui.label(RichText::new(err).color(theme::WARNING_AMBER).size(10.5));
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        let edit = ui.add(
            egui::TextEdit::singleline(&mut state.new_name)
                .id(egui::Id::new("new_tag_name"))
                .desired_width(200.0)
                .hint_text("Название нового тега")
                .text_color(theme::TEXT_PRIMARY),
        );
        let by_enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let can_add = !state.new_name.trim().is_empty();
        if (ui.add_enabled(can_add, egui::Button::new("＋ Добавить")).clicked() || by_enter)
            && can_add
        {
            ops.push(Op::Create(state.new_name.clone()));
        }
    });

    // ── Подтверждение удаления ──────────────────────────────────────────────
    if let Some(id) = state.confirm_delete {
        let (name, used) = match tags.get(id) {
            Some(tag) => (tag.name.clone(), tags.usage(id)),
            None => {
                state.confirm_delete = None;
                return;
            }
        };
        ui.add_space(8.0);
        Frame::NONE
            .fill(theme::BG_DARK)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(crate::ui::fit_width(ui));
                ui.label(
                    RichText::new(format!("Удалить тег «{name}»? Он снимется с {used} мод(ов)."))
                        .color(theme::TEXT_PRIMARY)
                        .size(11.5),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Удалить").color(theme::ERROR_RED)).clicked() {
                        ops.push(Op::Delete(id));
                        state.confirm_delete = None;
                    }
                    if ui.button("Отмена").clicked() {
                        state.confirm_delete = None;
                    }
                });
            });
    }
}

fn apply(state: &mut TagsUi, tags: &mut Tags, ops: Vec<Op>) -> bool {
    let mut changed = false;
    for op in ops {
        match op {
            Op::Recolor(id, color) => {
                tags.set_color(id, color);
                changed = true;
            }
            Op::Rename(id, name) => {
                if tags.rename(id, &name) {
                    state.error = None;
                    changed = true;
                } else {
                    state.error = Some(format!("Имя «{}» занято или пустое", name.trim()));
                    // Возвращаем поле к настоящему имени.
                    if let Some(tag) = tags.get(id) {
                        state.buffers.insert(tag_index(id), tag.name.clone());
                    }
                }
            }
            Op::Delete(id) => {
                tags.delete(id);
                // Индексы сдвинулись — буферы имён больше не соответствуют тегам.
                state.buffers.clear();
                changed = true;
            }
            Op::Create(name) => {
                if tags.create(&name).is_some() {
                    state.new_name.clear();
                    state.error = None;
                    changed = true;
                }
            }
        }
    }
    changed
}

/// `TagId` непрозрачен снаружи, но для ключа буфера нужен порядковый номер.
fn tag_index(id: TagId) -> usize {
    id.index()
}
