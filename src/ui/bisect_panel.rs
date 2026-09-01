//! Окно поиска виновника: ход сужения сборки и найденный набор.

use egui::{Align, Frame, Margin, RichText, ScrollArea, Stroke, Window};

use crate::bisect::{Attempt, Hunt, Kind, Target};
use crate::mod_data::{ModDb, ModId};
use crate::testing::Phase;
use crate::ui::{fit_width, test_panel, theme};

#[derive(Default)]
pub struct BisectUi {
    pub open: bool,
}

pub enum Reply {
    None,
    /// Остановить поиск.
    Cancel,
    /// Выделить мод в списке.
    Select(ModId),
    /// Выключить найденные моды в сборке.
    Deactivate(Vec<ModId>),
}

pub fn show(ctx: &egui::Context, state: &mut BisectUi, hunt: Option<&Hunt>, db: &ModDb) -> Reply {
    if !state.open {
        return Reply::None;
    }
    let mut reply = Reply::None;
    let mut open = true;

    Window::new("⚖  Поиск виновника")
        .open(&mut open)
        .collapsible(true)
        .resizable(true)
        .default_width(720.0)
        .default_height(520.0)
        .frame(
            Frame::window(&ctx.global_style())
                .fill(theme::BG_PANEL)
                .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)),
        )
        .show(ctx, |ui| match hunt {
            Some(hunt) => body(ui, hunt, db, &mut reply),
            None => intro(ui),
        });

    state.open = open;
    reply
}

fn intro(ui: &mut egui::Ui) {
    ui.label(
        RichText::new(
            "Поиск запускает игру снова и снова, каждый раз с урезанной сборкой, \
             и оставляет только то, без чего поломка пропадает.\n\n\
             Деления пополам недостаточно: в RimWorld ломается обычно пара модов, \
             которые по отдельности безобидны, — такую пару половинчатый поиск \
             просто теряет. Поэтому перебираются и части сборки, и дополнения к ним.\n\n\
             Каждый прогон — это минуты. Начинать стоит после того, как обычный \
             тест уже показал провал.",
        )
        .color(theme::TEXT_MUTED)
        .size(11.5),
    );
}

fn body(ui: &mut egui::Ui, hunt: &Hunt, db: &ModDb, reply: &mut Reply) {
    header(ui, hunt, reply);
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    if hunt.is_done() {
        result(ui, hunt, db, reply);
    } else {
        current(ui, hunt);
    }

    if !hunt.attempts().is_empty() {
        ui.add_space(6.0);
        journal(ui, hunt.attempts());
    }
}

fn header(ui: &mut egui::Ui, hunt: &Hunt, reply: &mut Reply) {
    ui.horizontal(|ui| {
        let left = hunt.search().result().len();
        ui.label(
            RichText::new(format!(
                "Сужено: {} → {left} мод(ов) • прогонов: {}",
                hunt.started_with(),
                hunt.search().runs(),
            ))
            .color(theme::TEXT_PRIMARY)
            .size(12.0)
            .strong(),
        );

        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if !hunt.is_done()
                && ui
                    .button(RichText::new("⏹ Остановить").color(theme::ERROR_RED))
                    .on_hover_text("Прекратить поиск, оставив найденное приближение")
                    .clicked()
            {
                *reply = Reply::Cancel;
            }
        });
    });

    ui.add_space(2.0);
    let target = match hunt.target() {
        Target::AnyFailure => "Ищем: любой провал прогона".to_string(),
        Target::Issue(title) => format!("Ищем: {title}"),
    };
    ui.add(
        egui::Label::new(RichText::new(target).color(theme::TEXT_MUTED).size(10.5)).truncate(),
    );
}

fn current(ui: &mut egui::Ui, hunt: &Hunt) {
    match hunt.run() {
        Some(run) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(format!(
                        "{} • сборка из {} мод(ов) • {} с",
                        test_panel::phase_text(run.phase()),
                        run.active().len(),
                        run.elapsed().as_secs(),
                    ))
                    .color(theme::TEXT_PRIMARY)
                    .size(11.5),
                );
            });
            if matches!(run.phase(), Phase::Starting) {
                ui.add_space(2.0);
                ui.label(
                    RichText::new("Первую минуту лога нет — Proton поднимает окружение.")
                        .color(theme::TEXT_MUTED)
                        .size(10.5),
                );
            }
        }
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new("Готовим следующий прогон…")
                        .color(theme::TEXT_MUTED)
                        .size(11.5),
                );
            });
        }
    }
}

fn result(ui: &mut egui::Ui, hunt: &Hunt, db: &ModDb, reply: &mut Reply) {
    let found = hunt.search().result();

    if hunt.was_cancelled() {
        ui.label(
            RichText::new("Поиск остановлен. Ниже — то, до чего успели сузить.")
                .color(theme::TEXT_MUTED)
                .size(11.0),
        );
        ui.add_space(4.0);
    }

    if found.is_empty() {
        ui.label(
            RichText::new(
                "Сузить не удалось: проблема не воспроизвелась ни на одном урезанном \
                 наборе. Так бывает, когда поломка зависит от чего-то помимо состава \
                 сборки — от сохранения, настроек мода или просто от везения.",
            )
            .color(theme::WARNING_AMBER)
            .size(11.0),
        );
        return;
    }

    let verdict = if found.len() == 1 {
        "Виновник найден".to_string()
    } else {
        format!("Виновны вместе: {} мод(ов)", found.len())
    };
    ui.label(RichText::new(verdict).color(theme::ACTIVE_GREEN).size(12.0).strong());
    ui.add_space(2.0);
    if found.len() > 1 {
        ui.label(
            RichText::new(
                "По отдельности они, скорее всего, безобидны — проблема возникает \
                 только когда включены все сразу.",
            )
            .color(theme::TEXT_MUTED)
            .size(10.5),
        );
    }
    ui.add_space(6.0);

    // Найденных может быть много — например, если поиск остановили рано.
    // Без прокрутки список уезжал за нижний край окна вместе с кнопкой.
    // Половина оставшейся высоты: вторая нужна кнопке и журналу прогонов.
    let list_height = (ui.available_height() * 0.5).max(80.0);
    ScrollArea::vertical()
        .id_salt("bisect_result")
        .max_height(list_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for id in &found {
                Frame::new()
                    .fill(theme::BG_ROW_EVEN)
                    .stroke(Stroke::new(1.0, theme::ERROR_RED.gamma_multiply(0.4)))
                    .inner_margin(Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(fit_width(ui));
                        ui.horizontal(|ui| {
                            let name =
                                db.get(id).map(|m| m.name.as_str()).unwrap_or(id.as_str());
                            if ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(name)
                                            .color(theme::TEXT_PRIMARY)
                                            .size(11.5),
                                    )
                                    .truncate()
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_text(id.as_str())
                                .clicked()
                            {
                                *reply = Reply::Select(id.clone());
                            }
                        });
                    });
                ui.add_space(3.0);
            }
        });

    ui.add_space(4.0);
    if ui
        .button(RichText::new("Выключить найденные").color(theme::ERROR_RED).size(11.5))
        .on_hover_text("Убрать эти моды из активной сборки")
        .clicked()
    {
        *reply = Reply::Deactivate(found.clone());
    }

    if hunt.off_target() > 0 {
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "Осторожно: {} прогон(ов) провалились по другой причине, а не по искомой. \
                 Такие считаются «чисто», и из-за них поиск мог свернуть не туда — \
                 проверьте результат вручную.",
                hunt.off_target(),
            ))
            .color(theme::WARNING_AMBER)
            .size(10.5),
        );
    }
}

fn journal(ui: &mut egui::Ui, attempts: &[Attempt]) {
    ui.label(
        RichText::new("Прогоны")
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
                .id_salt("bisect_journal")
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .max_height(160.0)
                .show(ui, |ui| {
                    for (n, attempt) in attempts.iter().enumerate() {
                        let (mark, color) = if attempt.reproduced {
                            ("⚑", theme::ERROR_RED)
                        } else {
                            ("✓", theme::ACTIVE_GREEN)
                        };
                        let outcome = if attempt.reproduced {
                            "воспроизвелось"
                        } else {
                            "чисто"
                        };
                        ui.label(
                            RichText::new(format!(
                                "{mark} #{}  {} мод(ов), {} — {outcome} ({})",
                                n + 1,
                                attempt.size,
                                kind_text(attempt.kind),
                                test_panel::verdict_text(&attempt.verdict),
                            ))
                            .color(color)
                            .size(10.0)
                            .monospace(),
                        );
                    }
                });
        });
}

/// Откуда взялся набор.
fn kind_text(kind: Kind) -> &'static str {
    match kind {
        Kind::Hint => "подозреваемые из лога",
        Kind::Part => "часть сборки",
        Kind::Rest => "всё, кроме части",
        Kind::Recheck => "проверка без подсказки",
    }
}
