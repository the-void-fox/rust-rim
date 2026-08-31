//! Правая панель: подробная информация о выбранном моде.

use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, StrokeKind, Vec2};

use crate::mod_data::{ModEntry, ModSource};
use crate::ui::theme;

pub fn source_color(source: &ModSource) -> Color32 {
    match source {
        ModSource::Core        => theme::SOURCE_CORE,
        ModSource::DLC(_)      => theme::SOURCE_DLC,
        ModSource::Workshop(_) => theme::SOURCE_WORKSHOP,
        ModSource::Local       => theme::SOURCE_LOCAL,
    }
}

pub fn source_label(source: &ModSource) -> &'static str {
    match source {
        ModSource::Core        => "CORE",
        ModSource::DLC(_)      => "DLC",
        ModSource::Workshop(_) => "WORKSHOP",
        ModSource::Local       => "LOCAL",
    }
}

pub fn show_mod_details(
    ui: &mut egui::Ui,
    mod_entry: Option<&ModEntry>,
    preview_tex: Option<&egui::TextureHandle>,
    md_cache: &mut egui_commonmark::CommonMarkCache,
) {
    let Some(m) = mod_entry else {
        ui.add_space(ui.available_height() / 3.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Выберите мод\nдля просмотра")
                .color(theme::TEXT_MUTED).size(12.0).italics());
        });
        return;
    };

    egui::ScrollArea::vertical()
        .id_salt("details_scroll")
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            show_banner(ui, m, preview_tex);
            ui.add_space(10.0);

            // ── Название и версия ────────────────────────────────────────
            let src_col = source_color(&m.source);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(source_label(&m.source))
                    .color(src_col).size(10.0).strong());
                ui.add_space(4.0);
                ui.label(RichText::new(&m.name)
                    .color(theme::TEXT_PRIMARY).size(13.0).strong());
            });
            if !m.version.is_empty() {
                ui.label(RichText::new(format!("v{}", m.version))
                    .color(theme::TEXT_MUTED).size(11.0));
            }

            ui.add_space(6.0);

            // ── Автор и ID ───────────────────────────────────────────────
            field(ui, "Автор:", &m.author, theme::TEXT_ACCENT, 11.0);
            field(ui, "ID:", m.package_id.as_str(), theme::TEXT_MUTED, 10.5);
            field(ui, "Версии RW:", &m.supported_versions.join(", "), theme::TEXT_PRIMARY, 11.0);

            // ── Описание ─────────────────────────────────────────────────
            if !m.description.is_empty() {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(RichText::new("ОПИСАНИЕ")
                    .color(theme::TEXT_MUTED).size(10.0).strong());
                ui.add_space(4.0);
                let desc = clean_unity_tags(&m.description);
                if looks_like_markdown(&desc) {
                    egui_commonmark::CommonMarkViewer::new().show(ui, md_cache, &desc);
                } else {
                    ui.add(egui::Label::new(
                        RichText::new(desc).color(theme::TEXT_PRIMARY).size(11.5)
                    ).wrap());
                }
            }

            // ── Зависимости и несовместимости ────────────────────────────
            let has_deps = !m.dependencies.is_empty();
            let has_incompat = !m.incompatible_with.is_empty();
            if has_deps || has_incompat {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
            }
            if has_deps {
                ui.label(RichText::new("ЗАВИСИМОСТИ")
                    .color(theme::TEXT_MUTED).size(10.0).strong());
                for dep in &m.dependencies {
                    bullet(ui, "→", theme::WARNING_AMBER, dep.as_str());
                }
                ui.add_space(4.0);
            }
            if has_incompat {
                ui.label(RichText::new("НЕСОВМЕСТИМО")
                    .color(theme::TEXT_MUTED).size(10.0).strong());
                for ic in &m.incompatible_with {
                    bullet(ui, "×", theme::ERROR_RED, ic.as_str());
                }
            }
        });
}

fn show_banner(ui: &mut egui::Ui, m: &ModEntry, preview_tex: Option<&egui::TextureHandle>) {
    let img_w = ui.available_width();
    let img_h = 160.0_f32.min(img_w * 0.5625); // 16:9
    let (img_rect, _) = ui.allocate_exact_size(Vec2::new(img_w, img_h), Sense::hover());

    ui.painter().rect_filled(img_rect, 4.0, theme::BG_DARK);

    if let Some(tex) = preview_tex {
        // Вписываем изображение с сохранением пропорций
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let tex_size = tex.size_vec2();
        let scale = (img_w / tex_size.x).min(img_h / tex_size.y);
        let draw_rect = egui::Rect::from_center_size(img_rect.center(), tex_size * scale);
        ui.painter().image(tex.id(), draw_rect, uv, Color32::WHITE);
    } else {
        let icon = if m.preview_path.is_some() { "⏳" } else { "◫" };
        ui.painter().text(
            img_rect.center(),
            Align2::CENTER_CENTER,
            icon,
            FontId::proportional(28.0),
            theme::TEXT_MUTED,
        );
    }

    ui.painter().rect_stroke(img_rect, 4.0, Stroke::new(1.0, theme::BORDER), StrokeKind::Outside);
}

fn field(ui: &mut egui::Ui, label: &str, value: &str, color: Color32, size: f32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(label).color(theme::TEXT_MUTED).size(11.0));
        ui.label(RichText::new(value).color(color).size(size));
    });
}

fn bullet(ui: &mut egui::Ui, marker: &str, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(marker).color(color).size(11.0));
        ui.label(RichText::new(text).color(theme::TEXT_PRIMARY).size(11.0));
    });
}

// ─── Описания модов: Markdown ────────────────────────────────────────────────

/// Переводит Unity rich-text теги RimWorld (`<b>`, `<i>`, `<color=…>`, `<size=…>`)
/// в Markdown-эквиваленты, чтобы описание не показывало сырые теги.
fn clean_unity_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        let Some(close) = tail.find('>') else {
            out.push_str(tail);
            break;
        };
        let tag = &tail[1..close]; // содержимое между < и >
        let lower = tag.to_ascii_lowercase();
        match lower.as_str() {
            "b" | "/b"   => out.push_str("**"),
            "i" | "/i"   => out.push_str("*"),
            "/color" | "/size" => {}
            _ if lower.starts_with("color=") || lower.starts_with("size=") => {}
            // Не тег форматирования (например "x < y") — оставляем как есть
            _ => out.push_str(&tail[..close + 1]),
        }
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Эвристика: похоже ли описание на Markdown. Обычный текст рендерим как раньше,
/// чтобы одиночные переводы строк не склеивались в параграфы.
fn looks_like_markdown(text: &str) -> bool {
    if text.contains("**") || text.contains("](") || text.contains('`') {
        return true;
    }
    text.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("# ") || t.starts_with("## ") || t.starts_with("### ")
            || t.starts_with("- ") || t.starts_with("* ") || t.starts_with("> ")
    })
}
