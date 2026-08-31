//! Правая панель: подробная информация о выбранном моде.

use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, StrokeKind, Vec2};

use crate::description;
use crate::mod_data::{ModEntry, ModId, ModSource};
use crate::ui::fit_width;
use crate::ui::theme;

/// Правая панель вместе со своими кэшами.
///
/// Описание конвертируется в Markdown один раз на выбранный мод, а не каждый
/// кадр: у крупных модов оно бывает в десятки килобайт.
#[derive(Default)]
pub struct DetailsView {
    md_cache: egui_commonmark::CommonMarkCache,
    rendered: Option<(ModId, description::Options, String)>,
}

impl DetailsView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        mod_entry: Option<&ModEntry>,
        preview_tex: Option<&egui::TextureHandle>,
        opts: description::Options,
    ) {
        let markdown = match mod_entry {
            Some(m) => {
                let stale = self
                    .rendered
                    .as_ref()
                    .is_none_or(|(id, cached, _)| id != &m.package_id || *cached != opts);
                if stale {
                    self.rendered = Some((
                        m.package_id.clone(),
                        opts,
                        description::to_markdown_with(&m.description, opts),
                    ));
                }
                self.rendered.as_ref().map(|(_, _, md)| md.as_str()).unwrap_or("")
            }
            None => "",
        };
        show_mod_details(ui, mod_entry, preview_tex, markdown, &mut self.md_cache);
    }
}

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

fn show_mod_details(
    ui: &mut egui::Ui,
    mod_entry: Option<&ModEntry>,
    preview_tex: Option<&egui::TextureHandle>,
    markdown: &str,
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
            ui.set_width(fit_width(ui));

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
            if !markdown.is_empty() {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(RichText::new("ОПИСАНИЕ")
                    .color(theme::TEXT_MUTED).size(10.0).strong());
                ui.add_space(4.0);
                // Панель справа раскладывает содержимое по его же запросу
                // и прижимает вправо: если контент шире панели, его левая
                // часть уезжает за край и обрезается. Явный предел не даёт
                // переносимым элементам запросить больше, чем есть.
                ui.set_max_width(fit_width(ui));
                egui_commonmark::CommonMarkViewer::new().show(ui, md_cache, markdown);
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
    let img_w = fit_width(ui);
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
