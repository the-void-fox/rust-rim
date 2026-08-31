// Регрессия: приложение падало на узком окне с
//   panicked at egui/src/ui.rs: Negative width makes no sense, but got: -0.5
//
// `Ui::available_width()` уходит в минус, когда контейнер переполнен, и
// `set_width`/`allocate_exact_size` роняли egui через debug_assert.
// Ограничения минимального размера окна тут недостаточно: тайловые
// композиторы (niri, sway) игнорируют min_inner_size.
//
// Тест собирает ту же раскладку, что и главный экран, и прогоняет её через
// полный кадр egui на вырожденных размерах. Любая паника = провал.

use rust_rim::app::fits_two_columns;
use rust_rim::mod_data::{ModDb, ModEntry, ModId, ModSource, Profile};
use rust_rim::tags::Tags;
use rust_rim::ui::list_cache::{ListCaches, SearchState};
use rust_rim::ui::mod_list::ModList;
use rust_rim::ui::details::DetailsView;
use rust_rim::ui::{theme, toolbar, widgets};

fn fake_mod(i: usize) -> ModEntry {
    ModEntry {
        name: format!("Мод с достаточно длинным названием номер {i}"),
        package_id: ModId::new(&format!("author{i}.mod{i}")),
        version: format!("1.{i}.0"),
        author: "Автор с длинным именем".into(),
        supported_versions: vec!["1.5".into(), "1.6".into()],
        path: std::path::PathBuf::from(format!("/mods/{i}")),
        source: if i % 3 == 0 { ModSource::Workshop(1000 + i as u64) } else { ModSource::Local },
        dependencies: vec![ModId::new("missing.dependency")],
        load_after: Vec::new(),
        load_before: Vec::new(),
        incompatible_with: vec![ModId::new("author1.mod1")],
        description: "**Описание** мода с *разметкой*, ссылкой [сюда](https://example.invalid) \
                      и достаточно длинным текстом, чтобы точно не влезть в узкую панель."
            .into(),
        preview_path: None,
    }
}

struct Fixture {
    ctx: egui::Context,
    db: ModDb,
    profile: Profile,
    tags: Tags,
    caches: ListCaches,
    search: SearchState,
    selected: Option<ModId>,
    details: DetailsView,
}

impl Fixture {
    fn new() -> Self {
        let db = ModDb::build((0..40).map(fake_mod).collect());
        let mut profile = Profile::new();
        for i in (0..40).filter(|i| i % 2 == 0) {
            profile.activate(ModId::new(&format!("author{i}.mod{i}")));
        }
        Self {
            ctx: egui::Context::default(),
            db,
            profile,
            tags: Tags::new(),
            caches: ListCaches::default(),
            search: SearchState::default(),
            selected: Some(ModId::new("author0.mod0")),
            details: DetailsView::new(),
        }
    }

    /// Рисует полный кадр главного экрана на окне `w` × `h`.
    fn frame(&mut self, w: f32, h: f32) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))),
            ..Default::default()
        };

        let db = &self.db;
        let profile = &self.profile;
        let tags = &self.tags;
        let caches = &mut self.caches;
        let search = &mut self.search;
        let selected = &mut self.selected;
        let details = &mut self.details;

        let _ = self.ctx.run_ui(input, |root| {
            caches.refresh(db, profile, tags, search);

            egui::Panel::top("toolbar_panel")
                .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 6)))
                .show(root, |ui| {
                    toolbar::show_toolbar(ui);
                });

            egui::Panel::bottom("status_bar").show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Активных: 20  •  Всего: 40");
                });
            });

            egui::Panel::right("details_panel")
                .min_size(240.0)
                .default_size(300.0)
                .max_size(500.0)
                .resizable(true)
                .show(root, |ui| {
                    ui.set_width(widgets::fit_width(ui));
                    let m = selected.as_ref().and_then(|id| db.get(id));
                    details.show(ui, m, None, Default::default());
                });

            egui::CentralPanel::default().show(root, |ui| {
                let spacing = ui.spacing().item_spacing.x;
                // Тот же переключатель раскладки, что и в приложении.
                if !fits_two_columns(widgets::fit_width(ui), spacing) {
                    widgets::panel_header(ui, "АКТИВНЫЕ МОДЫ", theme::HEADER_RIGHT, true, 20);
                    widgets::search_bar(ui, &mut search.active_query, "active_search");
                    ModList::new(db, &caches.active, &caches.warn, tags, selected, true).show(ui);
                    return;
                }

                ui.columns(2, |cols| {
                    widgets::panel_header(&mut cols[0], "НЕАКТИВНЫЕ МОДЫ", theme::HEADER_LEFT, false, 20);
                    widgets::search_bar(&mut cols[0], &mut search.inactive_query, "inactive_search");
                    ModList::new(db, &caches.inactive, &caches.warn, tags, selected, false)
                        .show(&mut cols[0]);

                    widgets::panel_header(&mut cols[1], "АКТИВНЫЕ МОДЫ", theme::HEADER_RIGHT, true, 20);
                    widgets::search_bar(&mut cols[1], &mut search.active_query, "active_search");
                    ModList::new(db, &caches.active, &caches.warn, tags, selected, true)
                        .show(&mut cols[1]);
                });
            });
        });
    }
}

/// Ширины, на которых окно физически не вмещает раскладку.
#[test]
fn survives_degenerate_window_widths() {
    let mut f = Fixture::new();
    // Прогрев: первый кадр инициализирует шрифты и размеры панелей.
    f.frame(1400.0, 900.0);

    for w in [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 80.0, 120.0, 200.0, 320.0, 480.0, 640.0] {
        // Два кадра: панели запоминают ширину между кадрами, и падало обычно
        // на втором — когда сохранённый размер уже не влезал в новый экран.
        f.frame(w, 600.0);
        f.frame(w, 600.0);
    }
}

#[test]
fn survives_degenerate_window_heights() {
    let mut f = Fixture::new();
    f.frame(1400.0, 900.0);

    for h in [1.0, 2.0, 5.0, 20.0, 60.0, 120.0, 300.0] {
        f.frame(1000.0, h);
        f.frame(1000.0, h);
    }
}

/// Плавное сжатие: панели переносят состояние между кадрами, поэтому
/// постепенное уменьшение проходит не те же пути, что резкий скачок.
#[test]
fn survives_gradual_shrink_and_regrow() {
    let mut f = Fixture::new();
    f.frame(1400.0, 900.0);

    let mut w = 1400.0_f32;
    while w > 1.0 {
        f.frame(w, 700.0);
        w -= 37.0;
    }
    while w < 1400.0 {
        f.frame(w.max(1.0), 700.0);
        w += 53.0;
    }
}

/// С активным поиском строки перерисовываются с другой геометрией.
#[test]
fn survives_narrow_window_with_active_search() {
    let mut f = Fixture::new();
    f.frame(1400.0, 900.0);
    f.search.inactive_query = "мод".into();
    f.search.active_query = "название".into();

    for w in [1.0, 15.0, 60.0, 150.0, 400.0] {
        f.frame(w, 500.0);
        f.frame(w, 500.0);
    }
}
