//! Очередь на скачивание, общая для обеих вкладок.
//!
//! Раньше её отрисовка была скопирована в обе вкладки дословно — полсотни
//! строк в двух экземплярах, и правки приходилось вносить дважды.

use egui::{Align, Frame, Layout, Margin, RichText, Stroke};

use crate::ui::{fit_width, theme};

/// Сколько символов названия влезает в ярлык очереди.
const TAG_CHARS: usize = 22;

#[derive(Default)]
pub struct Queue {
    items: Vec<(u64, String)>,
}

impl Queue {
    pub fn contains(&self, id: u64) -> bool {
        self.items.iter().any(|(qid, _)| *qid == id)
    }

    pub fn add(&mut self, id: u64, title: String) {
        if !self.contains(id) {
            self.items.push((id, title));
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|(qid, _)| *qid != id);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Высота, которую очередь займёт внизу окна.
    pub fn height(&self) -> f32 {
        if self.items.is_empty() { 0.0 } else { 72.0 }
    }
}

/// Рисует очередь. Возвращает идентификаторы, если нажали «Скачать».
pub fn footer(ui: &mut egui::Ui, queue: &mut Queue, id_salt: &str) -> Option<Vec<u64>> {
    if queue.is_empty() {
        return None;
    }
    let mut to_download = None;

    ui.separator();
    Frame::NONE
        .fill(theme::BG_HEADER)
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Очередь: {}  ", queue.len()))
                        .color(theme::TEXT_MUTED)
                        .size(11.0),
                );

                let tags_width = fit_width(ui) * 0.7;
                egui::ScrollArea::horizontal()
                    .id_salt(id_salt)
                    .max_height(26.0)
                    .max_width(tags_width)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let snap = queue.items.clone();
                            for (id, title) in &snap {
                                if tag(ui, *id, title) {
                                    queue.remove(*id);
                                }
                                ui.add_space(3.0);
                            }
                        });
                    });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let dl = egui::Button::new(
                        RichText::new("⬇  Скачать через SteamCMD")
                            .color(theme::TEXT_PRIMARY)
                            .size(11.5),
                    )
                    .fill(theme::HEADER_LEFT)
                    .stroke(Stroke::new(1.0, theme::BORDER_ACCENT));
                    if ui.add(dl).clicked() {
                        to_download = Some(queue.items.iter().map(|(id, _)| *id).collect());
                        queue.clear();
                    }
                    ui.add_space(6.0);
                    if ui
                        .button(RichText::new("× Очистить").color(theme::TEXT_MUTED).size(11.0))
                        .clicked()
                    {
                        queue.clear();
                    }
                });
            });
        });

    to_download
}

/// Ярлык одного мода в очереди. `true` — нажали «убрать».
fn tag(ui: &mut egui::Ui, id: u64, title: &str) -> bool {
    let short: String = title.chars().take(TAG_CHARS).collect();
    let short = if title.chars().count() > TAG_CHARS {
        format!("{short}…")
    } else {
        short
    };
    let button = egui::Button::new(
        RichText::new(format!("× {short}")).color(theme::TEXT_MUTED).size(10.5),
    )
    .fill(theme::BG_DARK)
    .stroke(Stroke::new(1.0, theme::BORDER));
    ui.add(button).on_hover_text(format!("Убрать {id}")).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_the_same_mod_twice_keeps_one() {
        // Мод может прийти и поштучно, и в составе сборки.
        let mut q = Queue::default();
        q.add(1, "Harmony".into());
        q.add(1, "Harmony".into());
        assert_eq!(q.len(), 1);
        assert!(q.contains(1));
    }

    #[test]
    fn removing_leaves_the_rest() {
        let mut q = Queue::default();
        q.add(1, "a".into());
        q.add(2, "b".into());
        q.remove(1);
        assert!(!q.contains(1));
        assert!(q.contains(2));
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn removing_something_absent_is_harmless() {
        let mut q = Queue::default();
        q.add(1, "a".into());
        q.remove(42);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn an_empty_queue_takes_no_space() {
        // По этой высоте список результатов считает, сколько ему осталось;
        // лишние пиксели выгнали бы очередь за нижний край окна.
        let mut q = Queue::default();
        assert_eq!(q.height(), 0.0);
        q.add(1, "a".into());
        assert!(q.height() > 0.0);
    }
}
