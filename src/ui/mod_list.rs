use egui::{
    Align2, Color32, FontId, Id, Pos2, Rect, RichText, Sense, Ui, Vec2,
    text::LayoutJob, epaint::text::TextWrapping,
};

use crate::app::{source_color, source_label, theme, DragPayload, MoveRequest, RowWarn};
use crate::mod_data::{ModEntry, ModSource};

pub const ROW_HEIGHT: f32 = 22.0;

const COL_ICON:    f32 = 18.0;
const COL_VERSION: f32 = 60.0;
const COL_WARN:    f32 = 20.0;
/// Зона у верхнего/нижнего края списка, в которой при перетаскивании
/// включается автопрокрутка.
const DRAG_SCROLL_ZONE:  f32 = 28.0;
const DRAG_SCROLL_SPEED: f32 = 9.0;

// Список рисуется вручную поверх виртуализированного ScrollArea:
// ровно ОДИН интерактивный виджет на строку (ui.interact), ячейки — painter.
// Id строки выводится из её экранного rect'а, поэтому «Widget rect changed id
// between passes» невозможен по построению: тот же rect → тот же id.
// Это же делает кадр дешёвым: ~40 видимых строк = ~40 виджетов вместо 4+ на строку.

pub struct ModList<'a> {
    mods:      &'a [ModEntry],
    indices:   &'a [usize],
    warn:      &'a [RowWarn],
    selected:  &'a mut Option<usize>,
    is_active: bool,
}

impl<'a> ModList<'a> {
    pub fn new(
        mods:      &'a [ModEntry],
        indices:   &'a [usize],
        warn:      &'a [RowWarn],
        selected:  &'a mut Option<usize>,
        is_active: bool,
    ) -> Self {
        Self { mods, indices, warn, selected, is_active }
    }

    pub fn show(self, ui: &mut Ui) -> Option<MoveRequest> {
        let ctx = ui.ctx().clone();
        let panel_key = if self.is_active { "active_list" } else { "inactive_list" };

        draw_header(ui);

        let spacing_y = ui.spacing().item_spacing.y;
        let pitch     = ROW_HEIGHT + spacing_y;
        let num_rows  = self.indices.len();

        let is_dragging = egui::DragAndDrop::has_any_payload(&ctx);
        let dragged_idx: Option<usize> =
            egui::DragAndDrop::payload::<DragPayload>(&ctx).map(|p| p.orig_idx);
        let pointer = ctx.pointer_latest_pos();

        let mut move_request: Option<MoveRequest> = None;

        egui::ScrollArea::vertical()
            .id_salt(panel_key)
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                // Геометрию берём из max_rect/available_width: min_rect после
                // set_height имеет нулевую ширину, и от него ломается всё
                // (нулевые rect'ы строк, обрезка имён, смещение колонок).
                let x0 = ui.max_rect().left();
                let content_top = ui.max_rect().top();
                let width = ui.available_width();

                let total_h = num_rows as f32 * pitch;
                ui.set_height(total_h.max(viewport.height()));

                // Видимая (экранная) область списка
                let visible = Rect::from_min_size(
                    Pos2::new(x0, content_top + viewport.top()),
                    Vec2::new(width, viewport.height()),
                );

                // ── Автопрокрутка при перетаскивании к краю ──────────────
                if is_dragging {
                    if let Some(p) = pointer.filter(|p| visible.contains(*p)) {
                        let dy = if p.y < visible.top() + DRAG_SCROLL_ZONE {
                            DRAG_SCROLL_SPEED
                        } else if p.y > visible.bottom() - DRAG_SCROLL_ZONE {
                            -DRAG_SCROLL_SPEED
                        } else {
                            0.0
                        };
                        if dy != 0.0 {
                            ui.scroll_with_delta(Vec2::new(0.0, dy));
                            ctx.request_repaint();
                        }
                    }
                }

                // ── Строка дропа под курсором (та же для линии и отпускания)
                let drop_row: Option<usize> = if is_dragging {
                    pointer
                        .filter(|p| visible.contains(*p))
                        .map(|p| {
                            (((p.y - content_top) / pitch).floor() as usize).min(num_rows)
                        })
                } else {
                    None
                };

                let first = ((viewport.top() / pitch).floor().max(0.0)) as usize;
                let last  = (((viewport.bottom() / pitch).ceil()) as usize).min(num_rows);

                let accent = if self.is_active { theme::HEADER_RIGHT } else { theme::HEADER_LEFT };

                for row_pos in first..last {
                    let orig_idx = self.indices[row_pos];
                    let m = &self.mods[orig_idx];
                    let rw = self.warn.get(orig_idx).copied().unwrap_or_default();

                    let row_top = content_top + row_pos as f32 * pitch;
                    let rect = Rect::from_min_size(
                        Pos2::new(x0, row_top),
                        Vec2::new(width, ROW_HEIGHT),
                    );

                    // Id — функция экранного rect'а: стабилен между проходами.
                    let id = Id::new((panel_key, rect.top() as i32));
                    let resp = ui.interact(rect, id, Sense::click_and_drag());

                    let is_selected      = *self.selected == Some(orig_idx);
                    let is_being_dragged = dragged_idx == Some(orig_idx);
                    let is_hovered       = !is_dragging && resp.hovered() && !is_selected;

                    let has_incompat     = rw.incompat;
                    let has_missing_deps = rw.missing_deps;

                    // ── Фон ──────────────────────────────────────────────
                    let base_color = if row_pos % 2 == 0 { theme::BG_ROW_EVEN } else { theme::BG_ROW_ODD };
                    let row_bg = if is_being_dragged {
                        Color32::from_rgb(22, 24, 30)
                    } else if is_selected {
                        theme::BG_SELECTED
                    } else if is_hovered {
                        theme::BG_ROW_HOVER
                    } else {
                        base_color
                    };
                    let painter = ui.painter();
                    painter.rect_filled(rect, 0.0, row_bg);

                    if is_selected && !is_being_dragged {
                        painter.rect_filled(
                            Rect::from_min_size(rect.left_top(), Vec2::new(2.0, ROW_HEIGHT)),
                            0.0, accent,
                        );
                    }

                    // ── Колонки ──────────────────────────────────────────
                    let icon_rect = Rect::from_min_size(
                        rect.left_top(), Vec2::new(COL_ICON, ROW_HEIGHT));
                    let warn_rect = Rect::from_min_size(
                        Pos2::new(rect.right() - COL_WARN, rect.top()),
                        Vec2::new(COL_WARN, ROW_HEIGHT));
                    let ver_rect = Rect::from_min_size(
                        Pos2::new(warn_rect.left() - COL_VERSION, rect.top()),
                        Vec2::new(COL_VERSION, ROW_HEIGHT));
                    let name_left  = icon_rect.right() + 4.0;
                    let name_width = (ver_rect.left() - 4.0 - name_left).max(10.0);

                    // Иконка источника
                    let (src_char, src_col) = match &m.source {
                        ModSource::Core        => ("◆", source_color(&m.source)),
                        ModSource::DLC(_)      => ("★", source_color(&m.source)),
                        ModSource::Workshop(_) => ("◇", source_color(&m.source)),
                        ModSource::Local       => ("◉", source_color(&m.source)),
                    };
                    painter.text(
                        icon_rect.center(), Align2::CENTER_CENTER,
                        src_char, FontId::proportional(11.0), src_col,
                    );

                    // Название (с обрезкой по ширине)
                    let name_color = if is_selected && !is_being_dragged {
                        Color32::WHITE
                    } else if has_incompat {
                        theme::ERROR_RED
                    } else if has_missing_deps && self.is_active {
                        theme::WARNING_AMBER
                    } else if is_being_dragged {
                        theme::TEXT_MUTED
                    } else {
                        theme::TEXT_PRIMARY
                    };
                    let mut job = LayoutJob::simple_singleline(
                        m.name.clone(), FontId::proportional(12.0), name_color);
                    job.wrap = TextWrapping::truncate_at_width(name_width);
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    let name_pos = Pos2::new(
                        name_left,
                        rect.center().y - galley.size().y * 0.5,
                    );
                    ui.painter().galley(name_pos, galley, name_color);

                    // Версия
                    let ver = if !m.version.is_empty() {
                        m.version.as_str()
                    } else {
                        m.supported_versions.last().map(String::as_str).unwrap_or("")
                    };
                    if !ver.is_empty() {
                        let mut vjob = LayoutJob::simple_singleline(
                            ver.to_string(), FontId::proportional(11.0), theme::TEXT_MUTED);
                        vjob.wrap = TextWrapping::truncate_at_width(COL_VERSION);
                        let vgalley = ui.fonts_mut(|f| f.layout_job(vjob));
                        let vpos = Pos2::new(
                            ver_rect.left(),
                            rect.center().y - vgalley.size().y * 0.5,
                        );
                        ui.painter().galley(vpos, vgalley, theme::TEXT_MUTED);
                    }

                    // Предупреждение
                    let warn_text: Option<(&str, Color32, &str)> = if has_incompat {
                        Some(("✕", theme::ERROR_RED, "Конфликт с активным модом"))
                    } else if has_missing_deps && self.is_active {
                        Some(("⚠", theme::WARNING_AMBER, "Отсутствуют зависимости"))
                    } else {
                        None
                    };
                    if let Some((ch, col, _)) = warn_text {
                        ui.painter().text(
                            warn_rect.center(), Align2::CENTER_CENTER,
                            ch, FontId::proportional(11.0), col,
                        );
                    }

                    // Линия дропа
                    if let Some(dr) = drop_row {
                        if dr == row_pos && !is_being_dragged {
                            paint_drop_line(ui, rect.top(), x0, width);
                        }
                    }

                    // ── Интеракции ───────────────────────────────────────
                    if resp.drag_started() {
                        egui::DragAndDrop::set_payload(&ctx, DragPayload { orig_idx });
                    }
                    if resp.clicked() {
                        *self.selected = Some(orig_idx);
                    }
                    if resp.double_clicked() {
                        move_request = Some(if self.is_active {
                            MoveRequest::Deactivate(orig_idx)
                        } else {
                            MoveRequest::Activate(orig_idx)
                        });
                    }

                    // Тултипы только над соответствующими колонками
                    let resp = if let Some(p) = resp.hover_pos() {
                        if icon_rect.contains(p) {
                            resp.on_hover_text(source_label(&m.source))
                        } else if warn_rect.contains(p) {
                            if let Some((_, _, tip)) = warn_text {
                                resp.on_hover_text(tip)
                            } else {
                                resp
                            }
                        } else {
                            resp
                        }
                    } else {
                        resp
                    };

                    resp.context_menu(|ui| {
                        ui.set_min_width(180.0);
                        ui.label(RichText::new(&m.name)
                            .color(theme::TEXT_ACCENT).size(11.0).strong());
                        ui.separator();
                        if self.is_active {
                            if ui.button("⬅  Деактивировать").clicked() {
                                move_request = Some(MoveRequest::Deactivate(orig_idx));
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("⬆  Переместить вверх").clicked() {
                                move_request = Some(MoveRequest::MoveUp(orig_idx));
                                ui.close();
                            }
                            if ui.button("⬇  Переместить вниз").clicked() {
                                move_request = Some(MoveRequest::MoveDown(orig_idx));
                                ui.close();
                            }
                        } else if ui.button("➡  Активировать").clicked() {
                            move_request = Some(MoveRequest::Activate(orig_idx));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("📁  Открыть папку").clicked() {
                            move_request = Some(MoveRequest::OpenFolder(orig_idx));
                            ui.close();
                        }
                    });
                }

                // Линия дропа в конец списка (ниже последней строки)
                if let Some(dr) = drop_row {
                    if dr >= last && dr == num_rows && num_rows > 0 {
                        let y = content_top + num_rows as f32 * pitch - spacing_y;
                        paint_drop_line(ui, y, x0, width);
                    }
                }

                // ── Отпускание перетаскивания над этим списком ───────────
                if is_dragging && ctx.input(|i| i.pointer.primary_released()) {
                    if let (Some(drop_pos), Some(payload_idx)) = (drop_row, dragged_idx) {
                        move_request = Some(MoveRequest::DragDrop {
                            orig_idx:  payload_idx,
                            to_active: self.is_active,
                            to_pos:    drop_pos,
                        });
                        egui::DragAndDrop::clear_payload(&ctx);
                    }
                }
            });

        move_request
    }
}

/// Шапка колонок над списком.
fn draw_header(ui: &mut Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 20.0), Sense::hover());
    let painter = ui.painter();
    painter.text(
        Pos2::new(rect.left() + COL_ICON + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        "НАЗВАНИЕ", FontId::proportional(10.0), theme::TEXT_MUTED,
    );
    painter.text(
        Pos2::new(rect.right() - COL_WARN - COL_VERSION, rect.center().y),
        Align2::LEFT_CENTER,
        "ВЕРСИЯ", FontId::proportional(10.0), theme::TEXT_MUTED,
    );
}

fn paint_drop_line(ui: &Ui, y: f32, x0: f32, width: f32) {
    ui.painter().rect_filled(
        Rect::from_min_size(Pos2::new(x0, y - 1.0), Vec2::new(width, 2.0)),
        0.0,
        theme::BORDER_ACCENT,
    );
}
