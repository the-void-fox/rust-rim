// Golden-тесты сортировщика: фиксируют текущий порядок загрузки, чтобы
// рефакторинг доменного слоя (ModId/ModDb/Profile) не поменял результат.
//
// Проверяется только наблюдаемое поведение — итоговый порядок packageId, —
// поэтому тесты переживают смену внутреннего представления.

use rust_rim::mod_data::{ModDb, ModEntry, ModId, ModSource, Profile};
use rust_rim::sorting::sort_active_mods;

// ─── Построитель модов ───────────────────────────────────────────────────────
// Единственное место, которое знает форму ModEntry: при смене представления
// правится только он.

struct B {
    entry: ModEntry,
    active: bool,
}

impl B {
    fn new(package_id: &str) -> Self {
        B {
            entry: ModEntry {
                name: package_id.to_string(),
                package_id: ModId::new(package_id),
                version: String::new(),
                author: "test".into(),
                supported_versions: vec!["1.6".into()],
                path: std::path::PathBuf::from(format!("/mods/{package_id}")),
                source: ModSource::Local,
                dependencies: Vec::new(),
                load_after: Vec::new(),
                load_before: Vec::new(),
                incompatible_with: Vec::new(),
                description: String::new(),
                preview_path: None,
            },
            active: true,
        }
    }

    /// Имя влияет на алфавитный тай-брейк внутри тира.
    fn name(mut self, n: &str) -> Self {
        self.entry.name = n.to_string();
        self
    }

    fn source(mut self, s: ModSource) -> Self {
        self.entry.source = s;
        self
    }

    fn after(mut self, ids: &[&str]) -> Self {
        self.entry.load_after = ids.iter().map(|s| ModId::new(s)).collect();
        self
    }

    fn before(mut self, ids: &[&str]) -> Self {
        self.entry.load_before = ids.iter().map(|s| ModId::new(s)).collect();
        self
    }

    fn inactive(mut self) -> Self {
        self.active = false;
        self
    }
}

fn core() -> B {
    B::new("ludeon.rimworld").name("Core").source(ModSource::Core)
}

fn dlc(package_id: &str, folder: &str) -> B {
    B::new(package_id)
        .name(folder)
        .source(ModSource::DLC(folder.to_string()))
}

/// Сортирует сборку и возвращает итоговый порядок загрузки.
fn sorted_order(mods: Vec<B>) -> Vec<String> {
    let active: Vec<ModId> = mods
        .iter()
        .filter(|b| b.active)
        .map(|b| b.entry.package_id.clone())
        .collect();
    let db = ModDb::build(mods.into_iter().map(|b| b.entry).collect());

    let mut profile = Profile::new();
    for id in active {
        profile.activate(id);
    }

    sort_active_mods(&mut profile, &db, None);

    profile.order().iter().map(|id| id.as_str().to_string()).collect()
}

// ─── Тесты ───────────────────────────────────────────────────────────────────

#[test]
fn tier_zero_goes_before_core() {
    let order = sorted_order(vec![
        B::new("a.regular").name("Aaa Regular"),
        core(),
        B::new("brrainz.harmony").name("Harmony"),
    ]);

    assert_eq!(order, ["brrainz.harmony", "ludeon.rimworld", "a.regular"]);
}

#[test]
fn tier_zero_pulls_in_its_dependencies() {
    // Harmony грузится после some.lib ⇒ some.lib тоже обязан быть до Core.
    let order = sorted_order(vec![
        core(),
        B::new("a.regular").name("Aaa Regular"),
        B::new("brrainz.harmony").name("Harmony").after(&["some.lib"]),
        B::new("some.lib").name("Some Lib"),
    ]);

    assert_eq!(
        order,
        ["some.lib", "brrainz.harmony", "ludeon.rimworld", "a.regular"],
    );
}

#[test]
fn dlc_follow_release_order_not_input_order() {
    let order = sorted_order(vec![
        dlc("ludeon.rimworld.odyssey", "Odyssey"),
        dlc("ludeon.rimworld.anomaly", "Anomaly"),
        dlc("ludeon.rimworld.biotech", "Biotech"),
        dlc("ludeon.rimworld.ideology", "Ideology"),
        dlc("ludeon.rimworld.royalty", "Royalty"),
        core(),
    ]);

    assert_eq!(
        order,
        [
            "ludeon.rimworld",
            "ludeon.rimworld.royalty",
            "ludeon.rimworld.ideology",
            "ludeon.rimworld.biotech",
            "ludeon.rimworld.anomaly",
            "ludeon.rimworld.odyssey",
        ],
    );
}

#[test]
fn load_after_beats_alphabetical_tiebreak() {
    // По алфавиту "Alpha" должен быть раньше "Zebra", но loadAfter сильнее.
    let order = sorted_order(vec![
        B::new("b.mod").name("Alpha").after(&["a.mod"]),
        B::new("a.mod").name("Zebra"),
    ]);

    assert_eq!(order, ["a.mod", "b.mod"]);
}

#[test]
fn load_before_is_respected() {
    let order = sorted_order(vec![
        B::new("a.mod").name("Zebra").before(&["b.mod"]),
        B::new("b.mod").name("Alpha"),
    ]);

    assert_eq!(order, ["a.mod", "b.mod"]);
}

#[test]
fn frameworks_go_before_regular_mods() {
    let order = sorted_order(vec![
        B::new("aaa.regular").name("Aaa Regular"),
        B::new("unlimitedhugs.hugslib").name("HugsLib"),
        core(),
    ]);

    assert_eq!(order, ["ludeon.rimworld", "unlimitedhugs.hugslib", "aaa.regular"]);
}

#[test]
fn load_bottom_mods_go_last() {
    let order = sorted_order(vec![
        B::new("krkr.rocketman").name("Aaa RocketMan"),
        B::new("zzz.regular").name("Zzz Regular"),
        core(),
    ]);

    assert_eq!(order, ["ludeon.rimworld", "zzz.regular", "krkr.rocketman"]);
}

#[test]
fn alphabetical_tiebreak_within_tier() {
    let order = sorted_order(vec![
        B::new("c.mod").name("Charlie"),
        B::new("a.mod").name("Alpha"),
        B::new("b.mod").name("Bravo"),
    ]);

    assert_eq!(order, ["a.mod", "b.mod", "c.mod"]);
}

#[test]
fn inactive_mods_are_not_sorted_in() {
    // Сортировка меняет только порядок сборки; выключенный мод в неё не попадает.
    let order = sorted_order(vec![
        B::new("z.active").name("Zebra"),
        B::new("m.inactive").name("Middle").inactive(),
        B::new("a.active").name("Alpha"),
    ]);

    assert_eq!(order, ["a.active", "z.active"]);
}

#[test]
fn cycle_does_not_drop_mods() {
    // a → b → a: топосорт невозможен, но ни один мод не должен потеряться.
    let mut order = sorted_order(vec![
        B::new("a.mod").name("Alpha").after(&["b.mod"]),
        B::new("b.mod").name("Bravo").after(&["a.mod"]),
        core(),
    ]);
    order.sort();

    assert_eq!(order, ["a.mod", "b.mod", "ludeon.rimworld"]);
}

#[test]
fn single_active_mod_is_noop() {
    assert_eq!(sorted_order(vec![B::new("only.mod")]), ["only.mod"]);
}
