//! Окно запуска игры: живой вывод Proton/wine и состояние процесса.
//!
//! Без него запуск выглядит как «ничего не произошло»: Proton при первом
//! старте разворачивает префикс десятки секунд и всё это время молчит,
//! а окно игры ещё не появилось.

use egui::{Align, Frame, RichText, ScrollArea, Stroke, Window};

use crate::process::Run;
use crate::ui::{fit_width, theme};

/// Что пользователь попросил сделать.
#[derive(PartialEq, Eq)]
pub enum Reply {
    None,
    /// Снять процесс.
    Kill,
}

pub fn show(ctx: &egui::Context, open: &mut bool, run: &Run) -> Reply {
    if !*open {
        return Reply::None;
    }
    let mut reply = Reply::None;
    let mut window_open = true;

    Window::new("▶  Запуск RimWorld")
        .open(&mut window_open)
        .collapsible(true)
        .resizable(true)
        .default_width(720.0)
        .default_height(420.0)
        .frame(
            Frame::window(&ctx.global_style())
                .fill(theme::BG_PANEL)
                .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)),
        )
        .show(ctx, |ui| {
            status_row(ui, run, &mut reply);
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
            log_view(ui, run);
        });

    *open = window_open;
    reply
}

fn status_row(ui: &mut egui::Ui, run: &Run, reply: &mut Reply) {
    ui.horizontal(|ui| {
        let secs = run.elapsed().as_secs();
        if run.is_running() {
            ui.spinner();
            ui.label(
                RichText::new(format!("Работает {secs} с"))
                    .color(theme::ACTIVE_GREEN)
                    .size(12.0)
                    .strong(),
            );
        } else {
            let (mark, color, text) = match run.status().and_then(|s| s.code()) {
                Some(0) => ("✔", theme::ACTIVE_GREEN, "Игра закрыта".to_string()),
                Some(code) => ("✕", theme::ERROR_RED, format!("Завершилась с кодом {code}")),
                None => ("■", theme::TEXT_MUTED, "Процесс снят".to_string()),
            };
            ui.label(RichText::new(mark).color(color).size(12.0));
            ui.label(RichText::new(format!("{text} • {secs} с")).color(color).size(12.0));
        }

        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if run.is_running()
                && ui
                    .button(RichText::new("⏹ Остановить").color(theme::ERROR_RED))
                    .on_hover_text("Снять процесс игры")
                    .clicked()
            {
                *reply = Reply::Kill;
            }
            ui.label(
                RichText::new(format!("строк: {}", run.total_lines()))
                    .color(theme::TEXT_MUTED)
                    .size(10.5),
            );
        });
    });

    ui.add_space(2.0);
    ui.add(
        egui::Label::new(
            RichText::new(&run.command).color(theme::TEXT_MUTED).size(10.0).monospace(),
        )
        .truncate(),
    )
    .on_hover_text(&run.command);
}

fn log_view(ui: &mut egui::Ui, run: &Run) {
    let lines = run.lines();
    if lines.is_empty() {
        ui.label(
            RichText::new("Вывода пока нет — Proton разворачивает окружение…")
                .color(theme::TEXT_MUTED)
                .italics()
                .size(11.0),
        );
        return;
    }

    Frame::NONE
        .fill(theme::BG_DARK)
        .inner_margin(egui::Margin::symmetric(6, 5))
        .show(ui, |ui| {
            ui.set_width(fit_width(ui));
            ScrollArea::vertical()
                .id_salt("launch_log")
                .auto_shrink([false, false])
                // Пока процесс жив — держимся низа: интересен свежий вывод.
                .stick_to_bottom(run.is_running())
                .show(ui, |ui| {
                    for line in &lines {
                        ui.label(
                            RichText::new(line)
                                .color(line_color(line))
                                .size(10.5)
                                .monospace(),
                        );
                    }
                });
        });
}

/// Подсветка по ключевым словам: в потоке Proton полезное тонет в шуме.
fn line_color(line: &str) -> egui::Color32 {
    let lower = line.to_lowercase();
    if lower.contains("err") || lower.contains("fail") || lower.contains("не удалось") {
        theme::ERROR_RED
    } else if lower.contains("warn") {
        theme::WARNING_AMBER
    } else {
        theme::TEXT_PRIMARY
    }
}
