//! Мелкие переиспользуемые элементы главного экрана.

use egui::{Align, Color32, Frame, Layout, Margin, RichText, Sense, Vec2};

use crate::ui::theme;

/// Доступная ширина, никогда не отрицательная.
///
/// `Ui::available_width()` уходит в минус, когда контейнер уже переполнен —
/// это норма при узком окне или длинной строке. Но `set_width` и
/// `allocate_exact_size` на отрицательном значении роняют egui через
/// `debug_assert` («Negative width makes no sense»), поэтому любой расчёт
/// ширины обязан проходить здесь.
///
/// Отдельного «минимального размера окна» для этого недостаточно: тайловые
/// композиторы (niri, sway) игнорируют `min_inner_size`, и окно всё равно
/// можно сжать до любой ширины.
pub fn fit_width(ui: &egui::Ui) -> f32 {
    ui.available_width().max(0.0)
}

/// [`fit_width`] с вычетом места, зарезервированного под соседний элемент.
pub fn fit_width_minus(ui: &egui::Ui, reserve: f32) -> f32 {
    (ui.available_width() - reserve).max(0.0)
}

/// Заголовок колонки со счётчиком модов.
pub fn panel_header(ui: &mut egui::Ui, title: &str, accent: Color32, is_active: bool, count: usize) {
    Frame::NONE
        .fill(theme::BG_HEADER)
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 16.0), Sense::hover());
                ui.painter().rect_filled(rect, 1.0, accent);
                ui.add_space(6.0);
                ui.label(RichText::new(title).color(accent).size(11.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let badge = if is_active { theme::ACTIVE_GREEN } else { theme::TEXT_MUTED };
                    ui.label(RichText::new(format!("{count}")).color(badge).size(11.0));
                    ui.label(RichText::new("●").color(badge).size(8.0));
                });
            });
        });
}

/// Строка поиска над списком модов.
pub fn search_bar(ui: &mut egui::Ui, query: &mut String, id: &str) {
    Frame::NONE
        .fill(theme::BG_DARK)
        .inner_margin(Margin::symmetric(6, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("⊙").size(12.0).color(theme::TEXT_MUTED));
                let edit = egui::TextEdit::singleline(query)
                    .hint_text("Поиск...")
                    .id(egui::Id::new(id))
                    .frame(Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .text_color(theme::TEXT_PRIMARY);
                ui.add(edit);
                if !query.is_empty()
                    && ui.small_button(RichText::new("×").color(theme::TEXT_MUTED)).clicked()
                {
                    query.clear();
                }
            });
        });
}
