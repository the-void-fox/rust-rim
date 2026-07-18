#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rust_rim::app::RustRim;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let icon = eframe::icon_data::from_png_bytes(
        include_bytes!("assets/icon.png")
    ).ok();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("RustRim")
        .with_min_inner_size([900.0, 600.0])
        .with_inner_size([1400.0, 900.0]);

    if let Some(icon_data) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(icon_data));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Rust Rim",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());

            let mut fonts = egui::FontDefinitions::default();
            let font_bytes = include_bytes!("assets/NotoSansSC.ttf").to_vec();
            fonts.font_data.insert(
                "NotoSansSC".to_owned(),
                egui::FontData::from_owned(font_bytes).into(),
            );

            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "NotoSansSC".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "NotoSansSC".to_owned());

            cc.egui_ctx.set_fonts(fonts);
            cc.egui_ctx.set_pixels_per_point(1.4);

            Ok(Box::new(RustRim::default()))
        }),
    )
}