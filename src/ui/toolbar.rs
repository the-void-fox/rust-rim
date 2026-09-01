use egui::{Align, Button, Color32, FontId, Layout, RichText, Ui, Vec2};

use crate::ui::theme;

/// Результат взаимодействия с панелью инструментов.
#[derive(Default)]
pub struct ToolbarResponse {
    pub play_clicked: bool,
    pub save_clicked: bool,
    pub reload_clicked: bool,
    pub updates_clicked: bool,
    pub sort_clicked: bool,
    pub check_clicked: bool,
    pub test_clicked: bool,
    pub settings_clicked: bool,
    pub activate_all: bool,
    pub deactivate_all: bool,
    pub save_list_clicked: bool,
    pub load_list_clicked: bool,
    pub steamcmd_clicked: bool,
    pub workshop_clicked: bool,
    pub logs_clicked: bool,
    pub tags_clicked: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Btn {
    Play,
    Save,
    Reload,
    Updates,
    Sort,
    Check,
    Test,
    Workshop,
    SteamCmd,
    Logs,
    Tags,
    SaveList,
    LoadList,
    ActivateAll,
    DeactivateAll,
}

struct Spec {
    btn: Btn,
    icon: &'static str,
    label: &'static str,
    tip: &'static str,
    color: fn() -> Color32,
    accent: fn() -> Color32,
}

/// Кнопки в порядке убывания важности: при нехватке ширины хвост списка
/// первым уезжает в меню «⋯».
const SPECS: &[Spec] = &[
    Spec { btn: Btn::Play, icon: "▶", label: "Запустить",
        tip: "Запустить RimWorld с текущей сборкой модов",
        color: || theme::ACTIVE_GREEN, accent: || theme::ACTIVE_GREEN },
    Spec { btn: Btn::Save, icon: "⇩", label: "Сохранить",
        tip: "Сохранить ModsConfig.xml",
        color: || theme::TEXT_PRIMARY, accent: || theme::BORDER },
    Spec { btn: Btn::Reload, icon: "⟳", label: "Обновить",
        tip: "Перечитать папки модов с диска (F5). Порядок загрузки сохраняется",
        color: || theme::TEXT_PRIMARY, accent: || theme::BORDER },
    Spec { btn: Btn::Updates, icon: "⇧", label: "Обновления",
        tip: "Проверить, для каких модов в мастерской есть версия новее",
        color: || theme::SOURCE_WORKSHOP, accent: || theme::SOURCE_WORKSHOP },
    Spec { btn: Btn::Sort, icon: "⇅", label: "Сортировать",
        tip: "Автоматически отсортировать активные моды",
        color: || theme::TEXT_PRIMARY, accent: || theme::BORDER },
    Spec { btn: Btn::Check, icon: "✓", label: "Проверить",
        tip: "Статическая проверка сборки: зависимости, конфликты, порядок загрузки",
        color: || theme::TEXT_PRIMARY, accent: || theme::BORDER },
    Spec { btn: Btn::Test, icon: "⚛", label: "Тест",
        tip: "Прогнать сборку через -quicktest и разобрать лог игры",
        color: || theme::ACTIVE_GREEN, accent: || theme::ACTIVE_GREEN },
    Spec { btn: Btn::Workshop, icon: "⚒", label: "Workshop",
        tip: "Просмотр и поиск модов в Steam Workshop",
        color: || theme::SOURCE_WORKSHOP, accent: || theme::SOURCE_WORKSHOP },
    Spec { btn: Btn::SteamCmd, icon: "⬇", label: "SteamCMD",
        tip: "Скачать моды из Steam Workshop через SteamCMD",
        color: || theme::SOURCE_WORKSHOP, accent: || theme::SOURCE_WORKSHOP },
    Spec { btn: Btn::Logs, icon: "☰", label: "Логи",
        tip: "Анализ Player.log: ошибки и предполагаемые моды-виновники",
        color: || theme::WARNING_AMBER, accent: || theme::WARNING_AMBER },
    Spec { btn: Btn::Tags, icon: "⚑", label: "Теги",
        tip: "Управление тегами модов. Фильтр в поиске: tag:имя",
        color: || theme::TEXT_ACCENT, accent: || theme::BORDER_ACCENT },
    Spec { btn: Btn::SaveList, icon: "⎘", label: "Сохранить список",
        tip: "Экспортировать список активных модов в файл (совместимо с RimSort)",
        color: || theme::TEXT_ACCENT, accent: || theme::BORDER_ACCENT },
    Spec { btn: Btn::LoadList, icon: "⎆", label: "Загрузить список",
        tip: "Импортировать список модов из файла (ModsConfig.xml, .rml, .rws)",
        color: || theme::TEXT_ACCENT, accent: || theme::BORDER_ACCENT },
    Spec { btn: Btn::ActivateAll, icon: "▶▶", label: "Все активны",
        tip: "Активировать все моды",
        color: || theme::ACTIVE_GREEN, accent: || theme::ACTIVE_GREEN },
    Spec { btn: Btn::DeactivateAll, icon: "◀◀", label: "Все неактивны",
        tip: "Деактивировать все (кроме Core)",
        color: || theme::ERROR_RED, accent: || theme::ERROR_RED },
];

const LEGEND: &[(&str, &str, fn() -> Color32)] = &[
    ("◆", "Core", || theme::SOURCE_CORE),
    ("★", "DLC", || theme::SOURCE_DLC),
    ("◇", "Workshop", || theme::SOURCE_WORKSHOP),
    ("◉", "Local", || theme::SOURCE_LOCAL),
];

const BTN_TEXT: f32 = 12.0;
const LEGEND_TEXT: f32 = 10.5;
/// Ниже этой ширины окна легенда источников не показывается.
const LEGEND_MIN_WIDTH: f32 = 900.0;

/// Отрисовывает панель инструментов и возвращает информацию о нажатых кнопках.
///
/// Раскладка адаптивная: панель не может «наехать» сама на себя, потому что
/// ширина правой группы (легенда и шестерёнка) резервируется до отрисовки
/// левой. Раньше обе группы жили в одном `horizontal`, egui не резервировал
/// место под правую, и при узком окне кнопки накладывались друг на друга.
///
/// Порядок сжатия: подписи → иконки → меню «⋯».
pub fn show_toolbar(ui: &mut Ui) -> ToolbarResponse {
    let mut resp = ToolbarResponse::default();

    ui.horizontal(|ui| {
        let total = ui.available_width();
        let row_h = ui.spacing().interact_size.y.max(22.0);

        let show_legend = total >= LEGEND_MIN_WIDTH;
        let right_w = right_group_width(ui, show_legend);
        let left_w = (total - right_w).max(0.0);

        ui.allocate_ui_with_layout(
            Vec2::new(left_w, row_h),
            Layout::left_to_right(Align::Center),
            |ui| draw_left_group(ui, &mut resp, left_w),
        );

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let gear = Button::new(RichText::new("⚙").color(theme::TEXT_MUTED).size(14.0))
                .fill(Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE);
            if ui.add(gear).on_hover_text("Настройки").clicked() {
                resp.settings_clicked = true;
            }

            if show_legend {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);
                for (mark, label, color) in LEGEND {
                    ui.label(
                        RichText::new(format!("{mark} {label}"))
                            .color(color())
                            .size(LEGEND_TEXT),
                    );
                    ui.add_space(4.0);
                }
            }
        });
    });

    resp
}

fn draw_left_group(ui: &mut Ui, resp: &mut ToolbarResponse, available: f32) {
    ui.label(RichText::new("RUSTRIM").color(theme::TEXT_ACCENT).size(13.0).strong());
    ui.add_space(8.0);

    let mut budget = available - text_width(ui, "RUSTRIM", 13.0) - 16.0;

    // Помещаются ли все кнопки с подписями?
    let full: f32 = SPECS.iter().map(|s| button_width(ui, &full_text(s))).sum();
    if full <= budget {
        for spec in SPECS {
            draw_button(ui, resp, spec, &full_text(spec));
        }
        return;
    }

    // Иначе — иконки; сколько влезет, остальное в меню «⋯».
    let icon_widths: Vec<f32> = SPECS.iter().map(|s| button_width(ui, s.icon)).collect();
    let all_icons: f32 = icon_widths.iter().sum();

    let visible = if all_icons <= budget {
        SPECS.len()
    } else {
        budget -= button_width(ui, "⋯");
        let mut n = 0;
        for w in &icon_widths {
            if budget - w < 0.0 {
                break;
            }
            budget -= w;
            n += 1;
        }
        n
    };

    for spec in &SPECS[..visible] {
        draw_button(ui, resp, spec, spec.icon);
    }

    if visible < SPECS.len() {
        ui.menu_button("⋯", |ui| {
            ui.set_min_width(200.0);
            for spec in &SPECS[visible..] {
                if ui.add(styled(spec, &full_text(spec))).on_hover_text(spec.tip).clicked() {
                    mark(resp, spec.btn);
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Ещё действия");
    }
}

fn draw_button(ui: &mut Ui, resp: &mut ToolbarResponse, spec: &Spec, text: &str) {
    if ui.add(styled(spec, text)).on_hover_text(spec.tip).clicked() {
        mark(resp, spec.btn);
    }
}

fn styled(spec: &Spec, text: &str) -> Button<'static> {
    Button::new(RichText::new(text.to_owned()).color((spec.color)()).size(BTN_TEXT))
        .fill(theme::BG_ROW_EVEN)
        .stroke(egui::Stroke::new(1.0, (spec.accent)().gamma_multiply(0.4)))
}

fn mark(resp: &mut ToolbarResponse, btn: Btn) {
    match btn {
        Btn::Play          => resp.play_clicked = true,
        Btn::Save          => resp.save_clicked = true,
        Btn::Reload        => resp.reload_clicked = true,
        Btn::Updates       => resp.updates_clicked = true,
        Btn::Sort          => resp.sort_clicked = true,
        Btn::Check         => resp.check_clicked = true,
        Btn::Test          => resp.test_clicked = true,
        Btn::Workshop      => resp.workshop_clicked = true,
        Btn::SteamCmd      => resp.steamcmd_clicked = true,
        Btn::Logs          => resp.logs_clicked = true,
        Btn::Tags          => resp.tags_clicked = true,
        Btn::SaveList      => resp.save_list_clicked = true,
        Btn::LoadList      => resp.load_list_clicked = true,
        Btn::ActivateAll   => resp.activate_all = true,
        Btn::DeactivateAll => resp.deactivate_all = true,
    }
}

fn full_text(spec: &Spec) -> String {
    format!("{} {}", spec.icon, spec.label)
}

fn text_width(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.ctx().fonts_mut(|f| {
        f.layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::WHITE)
            .size()
            .x
    })
}

/// Ширина кнопки вместе с внутренними отступами и промежутком до следующей.
fn button_width(ui: &Ui, text: &str) -> f32 {
    text_width(ui, text, BTN_TEXT)
        + ui.spacing().button_padding.x * 2.0
        + ui.spacing().item_spacing.x
}

/// Ширина правой группы: легенда источников + шестерёнка.
fn right_group_width(ui: &Ui, with_legend: bool) -> f32 {
    let gear = text_width(ui, "⚙", 14.0) + ui.spacing().button_padding.x * 2.0;
    if !with_legend {
        return gear + ui.spacing().item_spacing.x;
    }
    let legend: f32 = LEGEND
        .iter()
        .map(|(mark, label, _)| {
            text_width(ui, &format!("{mark} {label}"), LEGEND_TEXT)
                + 4.0
                + ui.spacing().item_spacing.x
        })
        .sum();
    gear + legend + 20.0
}
