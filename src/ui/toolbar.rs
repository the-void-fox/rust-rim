use egui::{Button, RichText, Ui};
use crate::mod_data::ModEntry;
use crate::app::theme;

/// Результат взаимодействия с панелью инструментов.
#[derive(Default)]
pub struct ToolbarResponse {
    pub save_clicked: bool,
    pub sort_clicked: bool,
    pub settings_clicked: bool,
    pub activate_all: bool,
    pub deactivate_all: bool,
    pub save_list_clicked: bool,
    pub load_list_clicked: bool,
    pub steamcmd_clicked: bool,
    pub workshop_clicked: bool,
    pub logs_clicked: bool,
}

/// Отрисовывает панель инструментов и возвращает информацию о нажатых кнопках.
pub fn show_toolbar(ui: &mut Ui, _mods: &[ModEntry]) -> ToolbarResponse {
    let mut resp = ToolbarResponse::default();

    ui.horizontal(|ui| {
        // Логотип / заголовок
        ui.label(
            RichText::new("RUSTRIM")
                .color(theme::TEXT_ACCENT)
                .size(13.0)
                .strong(),
        );

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // Файловые операции

        let save_btn = Button::new(
            RichText::new("💾 Сохранить").color(theme::TEXT_PRIMARY).size(12.0),
        )
        .fill(theme::BG_ROW_EVEN)
        .stroke(egui::Stroke::new(1.0, theme::BORDER));

        if ui.add(save_btn).on_hover_text("Сохранить ModsConfig.xml").clicked() {
            resp.save_clicked = true;
        }

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // Профили списков модов (совместимо с RimSort)
        let save_list_btn = Button::new(
            RichText::new("📋 Сохранить список").color(theme::TEXT_ACCENT).size(12.0),
        )
        .fill(theme::BG_ROW_EVEN)
        .stroke(egui::Stroke::new(1.0, theme::BORDER_ACCENT.gamma_multiply(0.5)));

        if ui.add(save_list_btn)
            .on_hover_text("Экспортировать список активных модов в файл (совместимо с RimSort)")
            .clicked()
        {
            resp.save_list_clicked = true;
        }

        let load_list_btn = Button::new(
            RichText::new("📂 Загрузить список").color(theme::TEXT_ACCENT).size(12.0),
        )
        .fill(theme::BG_ROW_EVEN)
        .stroke(egui::Stroke::new(1.0, theme::BORDER_ACCENT.gamma_multiply(0.5)));

        if ui.add(load_list_btn)
            .on_hover_text("Импортировать список модов из файла (ModsConfig.xml, .rml, .rws)")
            .clicked()
        {
            resp.load_list_clicked = true;
        }

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // Управление активацией
        let activate_btn = Button::new(
            RichText::new("▶▶ Все активны").color(theme::ACTIVE_GREEN).size(12.0),
        )
        .fill(theme::BG_ROW_EVEN)
        .stroke(egui::Stroke::new(1.0, theme::ACTIVE_GREEN.gamma_multiply(0.4)));

        if ui.add(activate_btn).on_hover_text("Активировать все моды").clicked() {
            resp.activate_all = true;
        }

        let deactivate_btn = Button::new(
            RichText::new("◀◀ Все неактивны").color(theme::ERROR_RED).size(12.0),
        )
        .fill(theme::BG_ROW_EVEN)
        .stroke(egui::Stroke::new(1.0, theme::ERROR_RED.gamma_multiply(0.4)));

        if ui.add(deactivate_btn).on_hover_text("Деактивировать все (кроме Core)").clicked() {
            resp.deactivate_all = true;
        }

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // SteamCMD
        let steam_btn = Button::new(
            RichText::new("⬇ SteamCMD").color(theme::SOURCE_WORKSHOP).size(12.0),
        )
        .fill(theme::BG_ROW_EVEN)
        .stroke(egui::Stroke::new(1.0, theme::SOURCE_WORKSHOP.gamma_multiply(0.4)));

        if ui.add(steam_btn)
            .on_hover_text("Скачать моды из Steam Workshop через SteamCMD")
            .clicked()
        {
            resp.steamcmd_clicked = true;
        }

        let ws_btn = Button::new(
            RichText::new("🔍 Workshop").color(theme::SOURCE_WORKSHOP).size(12.0),
        )
        .fill(theme::BG_ROW_EVEN)
        .stroke(egui::Stroke::new(1.0, theme::SOURCE_WORKSHOP.gamma_multiply(0.4)));

        if ui.add(ws_btn)
            .on_hover_text("Просмотр и поиск модов в Steam Workshop")
            .clicked()
        {
            resp.workshop_clicked = true;
        }

        let logs_btn = Button::new(
            RichText::new("📜 Логи").color(theme::WARNING_AMBER).size(12.0),
        )
        .fill(theme::BG_ROW_EVEN)
        .stroke(egui::Stroke::new(1.0, theme::WARNING_AMBER.gamma_multiply(0.4)));

        if ui.add(logs_btn)
            .on_hover_text("Анализ Player.log: ошибки и предполагаемые моды-виновники")
            .clicked()
        {
            resp.logs_clicked = true;
        }

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // Сортировка
        let sort_btn = Button::new(
            RichText::new("⇅ Сортировать").color(theme::TEXT_PRIMARY).size(12.0),
        )
        .fill(theme::BG_ROW_EVEN)
        .stroke(egui::Stroke::new(1.0, theme::BORDER));

        if ui.add(sort_btn)
            .on_hover_text("Автоматически отсортировать активные моды")
            .clicked()
        {
            resp.sort_clicked = true;
        }

        // Правая часть тулбара — легенда источников
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let settings_btn = Button::new(
                RichText::new("⚙").color(theme::TEXT_MUTED).size(14.0),
            )
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE);

            if ui.add(settings_btn).on_hover_text("Настройки").clicked() {
                resp.settings_clicked = true;
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            // Легенда
            ui.label(RichText::new("◉ Local").color(theme::SOURCE_LOCAL).size(10.5));
            ui.add_space(4.0);
            ui.label(RichText::new("◇ Workshop").color(theme::SOURCE_WORKSHOP).size(10.5));
            ui.add_space(4.0);
            ui.label(RichText::new("★ DLC").color(theme::SOURCE_DLC).size(10.5));
            ui.add_space(4.0);
            ui.label(RichText::new("◆ Core").color(theme::SOURCE_CORE).size(10.5));
        });
    });

    resp
}