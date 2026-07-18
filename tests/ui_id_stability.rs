// Headless-репро предупреждений "Widget rect changed id between passes".
// Гоняет ModList через полный цикл egui (ctx.run) со скроллом, фильтрацией
// и мутациями списка. Любое такое предупреждение из egui — провал теста.
//
// Запуск: cargo test --test ui_id_stability

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use rust_rim::app::{ListCaches, MoveRequest, SearchState};
use rust_rim::mod_data::{ModEntry, ModSource};
use rust_rim::ui::mod_list::ModList;

// ─── Логгер-счётчик ──────────────────────────────────────────────────────────

static WARN_COUNT: AtomicUsize = AtomicUsize::new(0);
static WARN_LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct CountingLogger;

impl log::Log for CountingLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record) {
        let msg = record.args().to_string();
        if msg.contains("changed id between passes") {
            WARN_COUNT.fetch_add(1, Ordering::Relaxed);
            let mut lines = WARN_LINES.lock().unwrap();
            if lines.len() < 10 {
                lines.push(msg);
            }
        }
    }
    fn flush(&self) {}
}

static LOGGER: CountingLogger = CountingLogger;

// ─── Тестовые данные ─────────────────────────────────────────────────────────

fn fake_mod(i: usize, active: bool) -> ModEntry {
    ModEntry {
        name: format!("Mod number {i} with a reasonably long name"),
        package_id: format!("author{}.mod{}", i % 40, i),
        version: if i % 3 == 0 { String::new() } else { format!("1.{}.{}", i % 6, i % 10) },
        author: format!("Author {}", i % 40),
        supported_versions: vec!["1.5".into(), "1.6".into()],
        path: std::path::PathBuf::from(format!("/tmp/fake/{i}")),
        source: if i % 4 == 0 { ModSource::Workshop(1000 + i as u64) } else { ModSource::Local },
        // Часть зависимостей заведомо отсутствует — чтобы warn-флаги были в деле
        dependencies: if i % 7 == 0 { vec![format!("author0.mod{}", i + 1), "missing.dep".into()] } else { Vec::new() },
        load_after: Vec::new(),
        load_before: Vec::new(),
        incompatible_with: if i % 11 == 0 { vec![format!("author{}.mod{}", (i + 1) % 40, i + 1)] } else { Vec::new() },
        is_active: active,
        description: String::new(),
        preview_path: None,
    }
}

struct Harness {
    ctx: egui::Context,
    mods: Vec<ModEntry>,
    caches: ListCaches,
    search: SearchState,
    selected: Option<usize>,
}

impl Harness {
    fn new(n: usize) -> Self {
        let ctx = egui::Context::default();
        let mods: Vec<ModEntry> = (0..n).map(|i| fake_mod(i, i % 2 == 0)).collect();
        Self { ctx, mods, caches: ListCaches::default(), search: SearchState::default(), selected: None }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Option<MoveRequest> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))),
            events,
            ..Default::default()
        };
        let mods = &self.mods;
        let caches = &mut self.caches;
        let search = &self.search;
        let selected = &mut self.selected;
        let mut req = None;
        let _ = self.ctx.run_ui(input, |root| {
            caches.refresh(mods, search);
            egui::CentralPanel::default().show(root, |ui| {
                ui.columns(2, |cols| {
                    if let Some(r) = ModList::new(mods, &caches.inactive, &caches.warn, selected, false)
                        .show(&mut cols[0]) {
                        req = Some(r);
                    }
                    if let Some(r) = ModList::new(mods, &caches.active, &caches.warn, selected, true)
                        .show(&mut cols[1]) {
                        req = Some(r);
                    }
                });
            });
        });
        req
    }
}

fn wheel(delta_y: f32) -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, delta_y),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::default(),
    }
}

fn pointer(x: f32, y: f32) -> egui::Event {
    egui::Event::PointerMoved(egui::pos2(x, y))
}

// ─── Тест ────────────────────────────────────────────────────────────────────

#[test]
fn no_widget_id_warnings() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Warn);

    let mut h = Harness::new(400);

    // Прогрев (первые кадры: sizing pass, инициализация шрифтов)
    for _ in 0..3 {
        h.frame(vec![pointer(300.0, 400.0)]);
    }
    let after_warmup = WARN_COUNT.load(Ordering::Relaxed);

    // 1. Плавный скролл колесом в левом списке — в т.ч. шаги, кратные
    //    высоте строки (22+3=25): раньше именно так совпадали rect'ы.
    for step in [-50.0, -25.0, -75.0, -50.0, 25.0, -100.0, 50.0, -25.0] {
        h.frame(vec![pointer(300.0, 400.0), wheel(step)]);
        for _ in 0..6 {
            h.frame(vec![pointer(300.0, 400.0)]); // дать анимации скролла доехать
        }
    }

    // 2. То же в правом (активном) списке
    for step in [-50.0, -25.0, -500.0, 250.0, -25.0] {
        h.frame(vec![pointer(1000.0, 400.0), wheel(step)]);
        for _ in 0..6 {
            h.frame(vec![pointer(1000.0, 400.0)]);
        }
    }

    // 3. Изменение поискового фильтра: содержимое строк меняется на месте,
    //    rect'ы не двигаются — раньше это давало залп предупреждений.
    for q in ["mod 1", "mod", "number 2", "", "author3", ""] {
        h.search.inactive_query = q.to_string();
        h.search.active_query = q.to_string();
        for _ in 0..3 {
            h.frame(vec![pointer(300.0, 400.0)]);
        }
    }

    // 4. Мутации списка: активация/деактивация сдвигает все строки.
    for i in [0usize, 2, 4, 6, 100, 102] {
        h.mods[i].is_active = !h.mods[i].is_active;
        h.caches.invalidate();
        for _ in 0..3 {
            h.frame(vec![pointer(300.0, 400.0)]);
        }
    }

    // 5. Скролл после мутаций
    for _ in 0..10 {
        h.frame(vec![pointer(300.0, 400.0), wheel(-25.0)]);
    }

    let total = WARN_COUNT.load(Ordering::Relaxed);
    let lines = WARN_LINES.lock().unwrap();
    assert_eq!(
        total, after_warmup,
        "egui выдал {} предупреждений об изменении id виджетов:\n{}",
        total - after_warmup,
        lines.join("\n"),
    );
    // И на прогреве их тоже быть не должно
    assert_eq!(after_warmup, 0, "предупреждения на первых кадрах:\n{}", lines.join("\n"));
}

/// Строки должны реально принимать клики: ловит вырожденную геометрию
/// (например, rect'ы нулевой ширины из-за неверного источника размеров).
#[test]
fn rows_are_clickable() {
    let mut h = Harness::new(60);
    for _ in 0..3 {
        h.frame(vec![pointer(100.0, 45.0)]);
    }
    let expected = h.caches.inactive.first().copied();
    assert!(expected.is_some(), "в тестовых данных нет неактивных модов");

    // Клик по первой строке левого списка: y = отступ панели (8) +
    // шапка колонок (20) + межэлементный отступ (3) + середина строки.
    let press = egui::Event::PointerButton {
        pos: egui::pos2(100.0, 45.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    };
    let release = egui::Event::PointerButton {
        pos: egui::pos2(100.0, 45.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    };
    h.frame(vec![pointer(100.0, 45.0), press]);
    h.frame(vec![release]);
    h.frame(vec![]);

    assert_eq!(h.selected, expected,
        "клик по первой строке не выделил мод — строки не интерактивны");
}
