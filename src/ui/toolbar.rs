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
    pub hunt_clicked: bool,
    pub mark_working: bool,
    pub disable_new: bool,
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

/// Что панели нужно знать о состоянии приложения.
#[derive(Default, Clone, Copy)]
pub struct ToolbarState {
    /// Сборка отличается от записанной в ModsConfig.xml.
    pub unsaved: bool,
    /// Последний тест провалился — есть что сужать поиском виновника.
    pub can_hunt: bool,
    /// Сколько активных модов включено после рабочей сборки.
    pub new_count: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Btn {
    Reload,
    Updates,
    Sort,
    Check,
    Test,
    Hunt,
    MarkWorking,
    DisableNew,
    Workshop,
    SteamCmd,
    Logs,
    Tags,
    SaveList,
    LoadList,
    ActivateAll,
    DeactivateAll,
}

/// Пункт выпадающей группы. `None` — разделитель.
type Item = Option<(Btn, &'static str, &'static str)>;

struct Group {
    icon: &'static str,
    label: &'static str,
    items: &'static [Item],
}

// Раньше здесь была плоская строка из пятнадцати равноправных кнопок: частое
// («Запустить») стояло вперемешку с редким («Все неактивны»), а при узком
// окне хвост уезжал в «⋯». Теперь на виду два главных действия, остальное
// разложено по группам по смыслу.
const GROUPS: &[Group] = &[
    Group { icon: "▤", label: "Моды", items: &[
        Some((Btn::Reload, "⟳  Перечитать с диска (F5)",
            "Перечитать папки модов. Порядок загрузки сохраняется")),
        Some((Btn::Updates, "⇧  Проверить обновления",
            "Для каких модов в мастерской есть версия новее")),
        Some((Btn::Workshop, "⚒  Мастерская Steam",
            "Найти и скачать моды и сборки из Steam Workshop")),
    ]},
    Group { icon: "⇅", label: "Сборка", items: &[
        Some((Btn::Sort, "⇅  Сортировать",
            "Расставить активные моды в правильном порядке загрузки")),
        Some((Btn::Check, "✓  Проверить сборку",
            "Без запуска игры: недостающие зависимости, конфликты, порядок, версия игры")),
        None,
        Some((Btn::Test, "⚛  Тест игры",
            "Запустить игру с этой сборкой и разобрать лог. Занимает минуты")),
        Some((Btn::Hunt, "⚖  Найти виновника",
            "Запускать игру с урезанными сборками, пока не найдётся мод, который всё ломает")),
        None,
        Some((Btn::MarkWorking, "★  Запомнить как рабочую",
            "Отметить текущую сборку как рабочую. Всё, что включишь потом, будет помечено «новый». \
             После успешного теста игры отметка ставится сама")),
        Some((Btn::DisableNew, "×  Выключить новые",
            "Выключить моды, включённые после рабочей сборки — быстрый откат к тому, что работало")),
    ]},
    Group { icon: "⎘", label: "Списки", items: &[
        Some((Btn::SaveList, "⎘  Сохранить список…",
            "Записать активные моды в файл (совместимо с RimSort)")),
        Some((Btn::LoadList, "⎆  Загрузить список…",
            "Заменить сборку списком из файла: ModsConfig.xml, .rml или сохранение .rws")),
        None,
        Some((Btn::ActivateAll, "▶▶  Включить все",
            "Сделать активными все установленные моды")),
        Some((Btn::DeactivateAll, "◀◀  Выключить все",
            "Оставить активным только Core")),
    ]},
    Group { icon: "☰", label: "Инструменты", items: &[
        Some((Btn::Logs, "☰  Анализ логов",
            "Разобрать Player.log: ошибки и моды, которые их вызвали")),
        Some((Btn::Tags, "⚑  Теги",
            "Свои метки и цвета для модов")),
        Some((Btn::SteamCmd, "⬇  SteamCMD",
            "Скачать моды из мастерской по ID без клиента Steam")),
    ]},
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
const LEGEND_MIN_WIDTH: f32 = 1000.0;

/// Отрисовывает панель инструментов и возвращает информацию о нажатых кнопках.
///
/// Ширина правой группы (легенда и шестерёнка) резервируется до отрисовки
/// левой — иначе при узком окне кнопки наезжают друг на друга.
pub fn show_toolbar(ui: &mut Ui, state: ToolbarState) -> ToolbarResponse {
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
            |ui| draw_left_group(ui, &mut resp, state, left_w),
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

fn draw_left_group(ui: &mut Ui, resp: &mut ToolbarResponse, state: ToolbarState, available: f32) {
    // Подписи групп убираются первыми, потом подписи главных кнопок:
    // иконки с выпадающими меню остаются доступны при любой ширине.
    let save_mark = if state.unsaved { " •" } else { "" };
    let play_full = "▶ Запустить".to_string();
    let save_full = format!("⇩ Сохранить{save_mark}");
    let groups_full: f32 = GROUPS.iter().map(|g| button_width(ui, &group_text(g, true))).sum();
    let groups_icons: f32 = GROUPS.iter().map(|g| button_width(ui, &group_text(g, false))).sum();
    let main_full = button_width(ui, &play_full) + button_width(ui, &save_full);
    let brand = text_width(ui, "RUSTRIM", 13.0) + 16.0;

    let show_brand = brand + main_full + groups_full + 16.0 <= available;
    let group_labels = main_full + groups_full + 16.0 <= available;
    let main_labels = main_full + groups_icons + 16.0 <= available;

    if show_brand {
        ui.label(RichText::new("RUSTRIM").color(theme::TEXT_ACCENT).size(13.0).strong());
        ui.add_space(8.0);
    }

    let (play, save) = if main_labels {
        (play_full, save_full)
    } else {
        ("▶".to_string(), format!("⇩{save_mark}"))
    };
    let play_btn = Button::new(RichText::new(play).color(theme::BG_DARK).size(BTN_TEXT).strong())
        .fill(theme::ACTIVE_GREEN);
    if ui.add(play_btn).on_hover_text("Запустить RimWorld с текущей сборкой").clicked() {
        resp.play_clicked = true;
    }
    let save_color = if state.unsaved { theme::WARNING_AMBER } else { theme::TEXT_PRIMARY };
    let save_tip = if state.unsaved {
        "Есть несохранённые изменения: игра их не увидит, пока сборка не записана в ModsConfig.xml"
    } else {
        "Записать сборку в ModsConfig.xml — игра загрузит именно её"
    };
    let save_btn = Button::new(RichText::new(save).color(save_color).size(BTN_TEXT))
        .fill(theme::BG_ROW_EVEN)
        .stroke(egui::Stroke::new(1.0, save_color.gamma_multiply(0.5)));
    if ui.add(save_btn).on_hover_text(save_tip).clicked() {
        resp.save_clicked = true;
    }

    ui.add_space(4.0);
    ui.separator();

    for group in GROUPS {
        let text = RichText::new(group_text(group, group_labels))
            .color(theme::TEXT_PRIMARY)
            .size(BTN_TEXT);
        ui.menu_button(text, |ui| {
            ui.set_min_width(220.0);
            for item in group.items {
                let Some((btn, label, tip)) = *item else {
                    ui.separator();
                    continue;
                };
                let (enabled, why_not) = match btn {
                    Btn::Hunt => (state.can_hunt,
                        "Сначала «Тест игры»: искать виновника есть смысл, когда тест провалился"),
                    Btn::DisableNew => (state.new_count > 0,
                        "Новых модов нет: всё активное входит в рабочую сборку"),
                    _ => (true, ""),
                };
                let tip = if enabled { tip } else { why_not };
                let label = match btn {
                    Btn::DisableNew if state.new_count > 0 => format!("{label} ({})", state.new_count),
                    _ => label.to_string(),
                };
                let r = ui.add_enabled(enabled, Button::new(RichText::new(label).size(BTN_TEXT)));
                let r = if enabled { r.on_hover_text(tip) } else { r.on_disabled_hover_text(tip) };
                if r.clicked() {
                    mark(resp, btn);
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(group.label);
    }
}

fn group_text(group: &Group, with_label: bool) -> String {
    if with_label {
        format!("{} {} ⏷", group.icon, group.label)
    } else {
        format!("{} ⏷", group.icon)
    }
}

fn mark(resp: &mut ToolbarResponse, btn: Btn) {
    match btn {
        Btn::Reload        => resp.reload_clicked = true,
        Btn::Updates       => resp.updates_clicked = true,
        Btn::Sort          => resp.sort_clicked = true,
        Btn::Check         => resp.check_clicked = true,
        Btn::Test          => resp.test_clicked = true,
        Btn::Hunt          => resp.hunt_clicked = true,
        Btn::MarkWorking   => resp.mark_working = true,
        Btn::DisableNew    => resp.disable_new = true,
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
