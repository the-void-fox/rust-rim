//! Окно автотеста сборки: ход прогона, вердикт и найденные ошибки.

use egui::{Align, Color32, Frame, Margin, RichText, ScrollArea, Stroke, Window};

use crate::log_analysis::LogIssue;
use crate::mod_data::ModId;
use crate::testing::{Phase, TestRun, Verdict};
use crate::ui::{fit_width, theme};

#[derive(Default)]
pub struct TestUi {
    pub open: bool,
    /// Разобранные записи лога — считаются один раз по завершении прогона.
    issues: Vec<LogIssue>,
    issues_ready: bool,
}

pub enum Reply {
    None,
    /// Остановить прогон.
    Cancel,
    /// Запустить заново.
    Restart,
    /// Выделить мод в списке.
    Select(ModId),
}

impl TestUi {
    /// Сбрасывает состояние перед новым прогоном.
    pub fn reset(&mut self) {
        self.issues.clear();
        self.issues_ready = false;
        self.open = true;
    }

    /// Складывает разбор лога, когда прогон закончился.
    pub fn set_issues(&mut self, issues: Vec<LogIssue>) {
        self.issues = issues;
        self.issues_ready = true;
    }

    pub fn issues_ready(&self) -> bool {
        self.issues_ready
    }
}

pub fn show(ctx: &egui::Context, state: &mut TestUi, run: Option<&TestRun>) -> Reply {
    if !state.open {
        return Reply::None;
    }
    let mut reply = Reply::None;
    let mut open = true;

    Window::new("🧪  Тест сборки")
        .open(&mut open)
        .collapsible(true)
        .resizable(true)
        .default_width(760.0)
        .default_height(500.0)
        .frame(
            Frame::window(&ctx.global_style())
                .fill(theme::BG_PANEL)
                .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)),
        )
        .show(ctx, |ui| match run {
            Some(run) => {
                status(ui, run, &mut reply);
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                match run.phase() {
                    Phase::Done(verdict) => done(ui, verdict, state, &mut reply),
                    _ => running(ui, run),
                }
            }
            None => {
                ui.label(
                    RichText::new(
                        "Прогон запускает игру с -quicktest: она грузит текущую сборку, \
                         генерирует карту и пишет лог, по которому выносится вердикт.\n\n\
                         ModsConfig.xml на время прогона подменяется и возвращается обратно.",
                    )
                    .color(theme::TEXT_MUTED)
                    .size(11.5),
                );
            }
        });

    state.open = open;
    reply
}

fn status(ui: &mut egui::Ui, run: &TestRun, reply: &mut Reply) {
    ui.horizontal(|ui| {
        let secs = run.elapsed().as_secs();
        match run.phase() {
            Phase::Done(verdict) => {
                let (mark, color) = verdict_look(verdict);
                ui.label(RichText::new(mark).color(color).size(13.0));
                ui.label(
                    RichText::new(format!("{} • {secs} с", verdict_text(verdict)))
                        .color(color)
                        .size(12.0)
                        .strong(),
                );
            }
            phase => {
                ui.spinner();
                ui.label(
                    RichText::new(format!("{} • {secs} с", phase_text(phase)))
                        .color(theme::TEXT_PRIMARY)
                        .size(12.0),
                );
            }
        }

        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            match run.phase() {
                Phase::Done(_) => {
                    if ui.button("↻ Повторить").clicked() {
                        *reply = Reply::Restart;
                    }
                }
                _ => {
                    if ui
                        .button(RichText::new("⏹ Остановить").color(theme::ERROR_RED))
                        .clicked()
                    {
                        *reply = Reply::Cancel;
                    }
                }
            }
        });
    });
}

fn phase_text(phase: &Phase) -> String {
    match phase {
        // Первую минуту лога нет вообще — это норма, а не зависание.
        Phase::Starting => "Запуск: Proton поднимает окружение".to_string(),
        Phase::Loading { lines } => format!("Загрузка модов ({lines} строк лога)"),
        Phase::Settling { lines, quiet } => format!(
            "Карта готова, ждём тишины {} с ({lines} строк)",
            quiet.as_secs(),
        ),
        Phase::Done(_) => "Готово".to_string(),
    }
}

fn verdict_look(verdict: &Verdict) -> (&'static str, Color32) {
    match verdict {
        Verdict::Passed => ("✔", theme::ACTIVE_GREEN),
        Verdict::LoadedWithErrors => ("⚠", theme::WARNING_AMBER),
        Verdict::Crashed { .. } | Verdict::DiedWhileLoading => ("✕", theme::ERROR_RED),
        Verdict::TimedOut => ("⏱", theme::WARNING_AMBER),
        Verdict::Cancelled => ("■", theme::TEXT_MUTED),
    }
}

fn verdict_text(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Passed => "Сборка загрузилась без ошибок".to_string(),
        Verdict::LoadedWithErrors => "Загрузилась, но в логе есть ошибки".to_string(),
        Verdict::Crashed { code } => match code {
            Some(c) => format!("Игра завершилась, не начав загрузку (код {c})"),
            None => "Игра завершилась, не начав загрузку".to_string(),
        },
        Verdict::DiedWhileLoading => {
            "Игра пропала посреди загрузки модов — до карты не дошло".to_string()
        }
        Verdict::TimedOut => "Не уложились в отведённое время".to_string(),
        Verdict::Cancelled => "Остановлено".to_string(),
    }
}

fn running(ui: &mut egui::Ui, run: &TestRun) {
    let lines = run.process_lines();
    ui.label(
        RichText::new("Вывод запуска")
            .color(theme::TEXT_MUTED)
            .size(10.5)
            .strong(),
    );
    ui.add_space(2.0);
    Frame::NONE
        .fill(theme::BG_DARK)
        .inner_margin(Margin::symmetric(6, 5))
        .show(ui, |ui| {
            ui.set_width(fit_width(ui));
            ScrollArea::vertical()
                .id_salt("test_process_log")
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in lines.iter().rev().take(200).rev() {
                        ui.label(
                            RichText::new(line).color(theme::TEXT_MUTED).size(10.0).monospace(),
                        );
                    }
                });
        });
}

fn done(ui: &mut egui::Ui, verdict: &Verdict, state: &TestUi, reply: &mut Reply) {
    if matches!(verdict, Verdict::DiedWhileLoading) {
        ui.label(
            RichText::new(
                "Так бывает под umu: Prepatcher перезапускает игру, а контейнер \
                 к этому моменту уже свернулся. Попробуйте повторить прогон или \
                 выбрать другой способ запуска в настройках.",
            )
            .color(theme::TEXT_MUTED)
            .size(11.0),
        );
        ui.add_space(6.0);
    }

    if !state.issues_ready {
        ui.label(RichText::new("Разбираю лог…").color(theme::TEXT_MUTED).size(11.0));
        return;
    }
    if state.issues.is_empty() {
        ui.label(
            RichText::new("Записей об ошибках в логе нет.")
                .color(theme::ACTIVE_GREEN)
                .size(11.5),
        );
        return;
    }

    ui.label(
        RichText::new(format!("Записей в логе: {}", state.issues.len()))
            .color(theme::TEXT_MUTED)
            .size(10.5),
    );
    ui.add_space(4.0);

    ScrollArea::vertical()
        .id_salt("test_issues")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for issue in &state.issues {
                draw_issue(ui, issue, reply);
                ui.add_space(4.0);
            }
        });
}

fn draw_issue(ui: &mut egui::Ui, issue: &LogIssue, reply: &mut Reply) {
    Frame::new()
        .fill(theme::BG_ROW_EVEN)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(fit_width(ui));
            ui.horizontal(|ui| {
                if issue.count > 1 {
                    ui.label(
                        RichText::new(format!("×{}", issue.count))
                            .color(theme::TEXT_ACCENT)
                            .size(11.0)
                            .strong(),
                    );
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(&issue.title).color(theme::TEXT_PRIMARY).size(11.0),
                    )
                    .truncate(),
                )
                .on_hover_text(&issue.full_text);
            });

            if issue.suspects.is_empty() {
                return;
            }
            ui.add_space(3.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Подозреваемые:").color(theme::TEXT_MUTED).size(10.5),
                );
                for suspect in &issue.suspects {
                    let color = if suspect.score >= 5 {
                        theme::ERROR_RED
                    } else if suspect.score >= 3 {
                        theme::WARNING_AMBER
                    } else {
                        theme::TEXT_MUTED
                    };
                    let button = egui::Button::new(
                        RichText::new(&suspect.name).color(color).size(10.5),
                    )
                    .fill(theme::BG_DARK)
                    .stroke(Stroke::new(1.0, color.gamma_multiply(0.4)));
                    let tip = format!(
                        "{}\nсчёт: {}\n{}",
                        suspect.package_id,
                        suspect.score,
                        suspect.evidence.join("\n"),
                    );
                    if ui.add(button).on_hover_text(tip).clicked() {
                        *reply = Reply::Select(suspect.package_id.clone());
                    }
                }
            });
        });
}
