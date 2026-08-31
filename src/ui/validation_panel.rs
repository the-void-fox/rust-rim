//! Окно «Проверка сборки»: список найденных проблем с кнопками исправления.

use egui::{Align, Frame, Margin, RichText, ScrollArea, Stroke, Window};

use crate::mod_data::{ModDb, ModId, Profile};
use crate::ui::{fit_width, theme};
use crate::validation::{self, Diagnostic, Fix, Severity};

#[derive(Default)]
pub struct ValidationUi {
    pub open: bool,
    diagnostics: Vec<Diagnostic>,
    show_warnings: bool,
    /// Проверка уже проводилась — до этого счётчики показывать нечестно.
    checked: bool,
}

/// Что пользователь попросил сделать.
pub enum Reply {
    None,
    /// Выделить мод в списке.
    Select(ModId),
    Apply(Fix),
}

impl ValidationUi {
    pub fn new() -> Self {
        Self { show_warnings: true, ..Default::default() }
    }

    pub fn refresh(&mut self, db: &ModDb, profile: &Profile, game_version: Option<&str>) {
        self.diagnostics = validation::validate(db, profile, game_version);
        self.checked = true;
    }

    pub fn errors(&self) -> usize {
        self.count(Severity::Error)
    }

    pub fn warnings(&self) -> usize {
        self.count(Severity::Warning)
    }

    fn count(&self, severity: Severity) -> usize {
        self.diagnostics.iter().filter(|d| d.severity == severity).count()
    }

    pub fn was_checked(&self) -> bool {
        self.checked
    }
}

pub fn show(ctx: &egui::Context, state: &mut ValidationUi, db: &ModDb) -> Reply {
    if !state.open {
        return Reply::None;
    }
    let mut reply = Reply::None;
    let mut open = true;

    Window::new("✓  Проверка сборки")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(720.0)
        .default_height(480.0)
        .frame(
            Frame::window(&ctx.global_style())
                .fill(theme::BG_PANEL)
                .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)),
        )
        .show(ctx, |ui| {
            header(ui, state);
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
            body(ui, state, db, &mut reply);
        });

    state.open = open;
    reply
}

fn header(ui: &mut egui::Ui, state: &mut ValidationUi) {
    let (errors, warnings) = (state.errors(), state.warnings());
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("✕ {errors}"))
                .color(if errors > 0 { theme::ERROR_RED } else { theme::TEXT_MUTED })
                .size(12.0)
                .strong(),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("⚠ {warnings}"))
                .color(if warnings > 0 { theme::WARNING_AMBER } else { theme::TEXT_MUTED })
                .size(12.0),
        );
        ui.add_space(12.0);
        ui.checkbox(
            &mut state.show_warnings,
            RichText::new("Показывать предупреждения").size(11.0),
        );
    });
    ui.add_space(2.0);
    ui.label(
        RichText::new(
            "Проверяется текущий порядок загрузки — тот, который уйдёт в ModsConfig.xml.",
        )
        .color(theme::TEXT_MUTED)
        .size(10.5)
        .italics(),
    );
}

fn body(ui: &mut egui::Ui, state: &mut ValidationUi, db: &ModDb, reply: &mut Reply) {
    let visible: Vec<&Diagnostic> = state
        .diagnostics
        .iter()
        .filter(|d| state.show_warnings || d.severity == Severity::Error)
        .collect();

    if visible.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Проблем не найдено 🎉")
                    .color(theme::ACTIVE_GREEN)
                    .size(13.0),
            );
        });
        return;
    }

    ScrollArea::vertical()
        .id_salt("validation_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for diag in visible {
                draw_diagnostic(ui, diag, db, reply);
                ui.add_space(4.0);
            }
        });
}

fn draw_diagnostic(ui: &mut egui::Ui, diag: &Diagnostic, db: &ModDb, reply: &mut Reply) {
    let (mark, color) = match diag.severity {
        Severity::Error => ("✕", theme::ERROR_RED),
        Severity::Warning => ("⚠", theme::WARNING_AMBER),
    };

    Frame::new()
        .fill(theme::BG_ROW_EVEN)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(fit_width(ui));

            ui.horizontal(|ui| {
                ui.label(RichText::new(mark).color(color).size(12.0));
                ui.add(
                    egui::Label::new(
                        RichText::new(&diag.title).color(theme::TEXT_PRIMARY).size(11.5).strong(),
                    )
                    .truncate(),
                )
                .on_hover_text(&diag.title);

                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if let Some(fix) = &diag.fix {
                        if ui
                            .button(RichText::new(fix_label(fix, db)).color(theme::ACTIVE_GREEN))
                            .clicked()
                        {
                            *reply = Reply::Apply(fix.clone());
                        }
                    }
                    if let Some(id) = diag.mods.first() {
                        if db.contains(id) && ui.button("Показать").clicked() {
                            *reply = Reply::Select(id.clone());
                        }
                    }
                });
            });

            if !diag.detail.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(&diag.detail).color(theme::TEXT_MUTED).size(10.5),
                    )
                    .wrap(),
                );
            }
        });
}

fn fix_label(fix: &Fix, db: &ModDb) -> String {
    let name = |id: &ModId| {
        db.get(id).map(|m| m.name.clone()).unwrap_or_else(|| id.to_string())
    };
    match fix {
        Fix::Activate(id) => format!("Включить «{}»", name(id)),
        Fix::Deactivate(id) => format!("Выключить «{}»", name(id)),
        Fix::Sort => "Сортировать".to_string(),
    }
}
