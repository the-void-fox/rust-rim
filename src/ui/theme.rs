//! Палитра и глобальный стиль egui.

use egui::{Color32, Margin, Stroke, Vec2};

pub const BG_DARK:       Color32 = Color32::from_rgb(18, 20, 24);
pub const BG_PANEL:      Color32 = Color32::from_rgb(25, 28, 34);
pub const BG_HEADER:     Color32 = Color32::from_rgb(30, 33, 41);
pub const BG_ROW_EVEN:   Color32 = Color32::from_rgb(28, 31, 38);
pub const BG_ROW_ODD:    Color32 = Color32::from_rgb(32, 36, 44);
pub const BG_ROW_HOVER:  Color32 = Color32::from_rgb(40, 46, 58);
pub const BG_SELECTED:   Color32 = Color32::from_rgb(45, 85, 130);

pub const BORDER:        Color32 = Color32::from_rgb(45, 50, 62);
pub const BORDER_ACCENT: Color32 = Color32::from_rgb(70, 130, 200);

pub const TEXT_PRIMARY:  Color32 = Color32::from_rgb(210, 215, 225);
pub const TEXT_MUTED:    Color32 = Color32::from_rgb(120, 130, 148);
pub const TEXT_ACCENT:   Color32 = Color32::from_rgb(100, 170, 255);

pub const ACTIVE_GREEN:  Color32 = Color32::from_rgb(80, 200, 120);
pub const WARNING_AMBER: Color32 = Color32::from_rgb(240, 180, 60);
pub const ERROR_RED:     Color32 = Color32::from_rgb(220, 75, 75);

pub const SOURCE_LOCAL:    Color32 = Color32::from_rgb(140, 160, 185);
pub const SOURCE_WORKSHOP: Color32 = Color32::from_rgb(100, 160, 240);
pub const SOURCE_DLC:      Color32 = Color32::from_rgb(180, 130, 240);
pub const SOURCE_CORE:     Color32 = Color32::from_rgb(240, 190, 80);

pub const HEADER_LEFT:  Color32 = Color32::from_rgb(60, 100, 170);
pub const HEADER_RIGHT: Color32 = Color32::from_rgb(60, 150, 100);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    style.visuals.window_fill         = BG_PANEL;
    style.visuals.panel_fill          = BG_DARK;
    style.visuals.override_text_color = Some(TEXT_PRIMARY);
    style.visuals.window_stroke       = Stroke::new(1.0, BORDER);
    style.visuals.selection.bg_fill   = BG_SELECTED;
    style.visuals.selection.stroke    = Stroke::new(1.0, BORDER_ACCENT);
    style.visuals.extreme_bg_color    = BG_DARK;
    style.visuals.faint_bg_color      = BG_ROW_ODD;

    // ── Состояния виджетов ────────────────────────────────────────────────────
    // expansion = 0 на всех состояниях — кнопки не меняют размер при наведении.
    style.visuals.widgets.noninteractive.expansion = 0.0;
    style.visuals.widgets.inactive.expansion       = 0.0;
    style.visuals.widgets.hovered.expansion        = 0.0;
    style.visuals.widgets.active.expansion         = 0.0;
    style.visuals.widgets.open.expansion           = 0.0;

    style.visuals.widgets.noninteractive.bg_fill   = BG_PANEL;
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_MUTED);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;

    style.visuals.widgets.inactive.bg_fill         = BG_ROW_EVEN;
    style.visuals.widgets.inactive.bg_stroke       = Stroke::new(1.0, BORDER);
    style.visuals.widgets.inactive.fg_stroke       = Stroke::new(1.0, TEXT_PRIMARY);

    style.visuals.widgets.hovered.bg_fill          = BG_ROW_HOVER;
    style.visuals.widgets.hovered.bg_stroke        = Stroke::new(1.0, BORDER_ACCENT);
    style.visuals.widgets.hovered.fg_stroke        = Stroke::new(1.0, TEXT_PRIMARY);

    style.visuals.widgets.active.bg_fill           = BG_SELECTED;
    style.visuals.widgets.active.bg_stroke         = Stroke::new(1.0, BORDER_ACCENT);
    style.visuals.widgets.active.fg_stroke         = Stroke::new(1.0, Color32::WHITE);

    style.visuals.widgets.open.bg_fill             = BG_ROW_HOVER;
    style.visuals.widgets.open.bg_stroke           = Stroke::new(1.0, BORDER_ACCENT);

    // ── Отступы ───────────────────────────────────────────────────────────────
    style.spacing.item_spacing   = Vec2::new(6.0, 3.0);
    style.spacing.window_margin  = Margin::same(10);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);

    ctx.set_global_style(style);
}
