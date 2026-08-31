//! Статическая проверка сборки модов — без запуска игры.
//!
//! Каждое правило — отдельная функция, возвращающая диагностики. Так их
//! можно проверять поштучно и добавлять новые, не трогая остальные.
//!
//! Проверяется именно *текущий* порядок загрузки, а не «можно ли его
//! починить»: сортировщик умеет расставить моды правильно, но пока
//! пользователь не нажал «Сортировать», в ModsConfig.xml лежит то, что
//! лежит, и игра прочитает именно его.

use std::collections::HashMap;

use crate::mod_data::{ModDb, ModId, Profile};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    /// Игра почти наверняка упадёт или мод не заработает.
    Error,
    /// Скорее всего проблема, но бывают исключения.
    Warning,
}

/// Что можно сделать одной кнопкой.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Fix {
    Activate(ModId),
    Deactivate(ModId),
    /// Пересортировать активные моды.
    Sort,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Стабильный идентификатор правила — для группировки и тестов.
    pub rule: &'static str,
    pub title: String,
    pub detail: String,
    /// Моды, которых касается запись; первый — «главный».
    pub mods: Vec<ModId>,
    pub fix: Option<Fix>,
}

impl Diagnostic {
    fn new(severity: Severity, rule: &'static str, title: String, detail: String) -> Self {
        Self { severity, rule, title, detail, mods: Vec::new(), fix: None }
    }

    fn about(mut self, mods: impl IntoIterator<Item = ModId>) -> Self {
        self.mods = mods.into_iter().collect();
        self
    }

    fn with_fix(mut self, fix: Fix) -> Self {
        self.fix = Some(fix);
        self
    }
}

/// Проверяет сборку целиком.
///
/// `game_version` — строка из `Version.txt`, например «1.6.4633 rev1260».
pub fn validate(db: &ModDb, profile: &Profile, game_version: Option<&str>) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    core_present(db, profile, &mut out);
    missing_dependencies(db, profile, &mut out);
    active_incompatibilities(db, profile, &mut out);
    load_order_violations(db, profile, &mut out);
    unsupported_game_version(db, profile, game_version, &mut out);
    duplicate_package_ids(db, &mut out);

    // Ошибки выше предупреждений; внутри — в порядке появления правил.
    out.sort_by_key(|d| d.severity);
    out
}

fn name_of(db: &ModDb, id: &ModId) -> String {
    db.get(id).map(|m| m.name.clone()).unwrap_or_else(|| id.to_string())
}

// ─── Правила ─────────────────────────────────────────────────────────────────

/// Без Core игра не запустится вообще.
fn core_present(db: &ModDb, profile: &Profile, out: &mut Vec<Diagnostic>) {
    let core = db.iter().find(|m| m.is_core());
    let Some(core) = core else {
        out.push(Diagnostic::new(
            Severity::Error,
            "core-missing",
            "Core не найден".to_string(),
            "В папке игры нет Data/Core — проверьте путь к игре в настройках.".to_string(),
        ));
        return;
    };
    if !profile.is_active(&core.package_id) {
        out.push(
            Diagnostic::new(
                Severity::Error,
                "core-inactive",
                "Core выключен".to_string(),
                "Без Core игра не запустится.".to_string(),
            )
            .about([core.package_id.clone()])
            .with_fix(Fix::Activate(core.package_id.clone())),
        );
    }
}

/// Мод требует другой мод, которого нет в сборке.
fn missing_dependencies(db: &ModDb, profile: &Profile, out: &mut Vec<Diagnostic>) {
    for id in profile.order() {
        let Some(m) = db.get(id) else { continue };
        for dep in &m.dependencies {
            if profile.is_active(dep) {
                continue;
            }
            let installed = db.contains(dep);
            let detail = if installed {
                format!("«{}» установлен, но выключен.", name_of(db, dep))
            } else {
                format!("«{dep}» вообще не установлен — его нужно скачать.")
            };
            let mut diag = Diagnostic::new(
                Severity::Error,
                "missing-dependency",
                format!("«{}» требует {dep}", m.name),
                detail,
            )
            .about([id.clone(), dep.clone()]);
            if installed {
                diag = diag.with_fix(Fix::Activate(dep.clone()));
            }
            out.push(diag);
        }
    }
}

/// Два активных мода объявлены несовместимыми.
fn active_incompatibilities(db: &ModDb, profile: &Profile, out: &mut Vec<Diagnostic>) {
    let mut seen: Vec<(ModId, ModId)> = Vec::new();
    for id in profile.order() {
        let Some(m) = db.get(id) else { continue };
        for other in &m.incompatible_with {
            if !profile.is_active(other) {
                continue;
            }
            // Несовместимость часто объявлена с обеих сторон — не дублируем.
            let pair = if id < other {
                (id.clone(), other.clone())
            } else {
                (other.clone(), id.clone())
            };
            if seen.contains(&pair) {
                continue;
            }
            seen.push(pair);

            out.push(
                Diagnostic::new(
                    Severity::Error,
                    "incompatible",
                    format!("«{}» несовместим с «{}»", m.name, name_of(db, other)),
                    "Оба мода включены. Оставьте один из них.".to_string(),
                )
                .about([id.clone(), other.clone()])
                .with_fix(Fix::Deactivate(other.clone())),
            );
        }
    }
}

/// Текущий порядок нарушает объявленные loadAfter/loadBefore.
fn load_order_violations(db: &ModDb, profile: &Profile, out: &mut Vec<Diagnostic>) {
    let pos: HashMap<&ModId, usize> =
        profile.order().iter().enumerate().map(|(i, id)| (id, i)).collect();

    for (i, id) in profile.order().iter().enumerate() {
        let Some(m) = db.get(id) else { continue };
        let mut problems: Vec<String> = Vec::new();

        // Зависимости обязаны грузиться раньше — иначе мод не найдёт их код.
        for earlier in m.load_after.iter().chain(m.dependencies.iter()) {
            if pos.get(earlier).is_some_and(|&j| j > i) {
                problems.push(format!("должен идти после «{}»", name_of(db, earlier)));
            }
        }
        for later in &m.load_before {
            if pos.get(later).is_some_and(|&j| j < i) {
                problems.push(format!("должен идти до «{}»", name_of(db, later)));
            }
        }

        if problems.is_empty() {
            continue;
        }
        out.push(
            Diagnostic::new(
                Severity::Error,
                "load-order",
                format!("«{}» стоит не на своём месте", m.name),
                problems.join("; "),
            )
            .about([id.clone()])
            .with_fix(Fix::Sort),
        );
    }
}

/// Мод не заявляет поддержку текущей версии игры.
fn unsupported_game_version(
    db: &ModDb,
    profile: &Profile,
    game_version: Option<&str>,
    out: &mut Vec<Diagnostic>,
) {
    let Some(target) = game_version.and_then(major_minor) else { return };

    for id in profile.order() {
        let Some(m) = db.get(id) else { continue };
        // Ванильный контент версионируется вместе с игрой.
        if m.is_vanilla() || m.supported_versions.is_empty() {
            continue;
        }
        let supported = m
            .supported_versions
            .iter()
            .filter_map(|v| major_minor(v))
            .any(|v| v == target);
        if supported {
            continue;
        }
        out.push(
            Diagnostic::new(
                Severity::Warning,
                "unsupported-version",
                format!("«{}» не заявляет поддержку {target}", m.name),
                format!("Заявлены версии: {}.", m.supported_versions.join(", ")),
            )
            .about([id.clone()])
            .with_fix(Fix::Deactivate(id.clone())),
        );
    }
}

/// Один и тот же мод установлен в нескольких местах.
fn duplicate_package_ids(db: &ModDb, out: &mut Vec<Diagnostic>) {
    for group in db.duplicates() {
        let extra = group
            .discarded
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        out.push(
            Diagnostic::new(
                Severity::Warning,
                "duplicate",
                format!("«{}» установлен несколько раз", group.id),
                format!("Используется:\n{}\nЛишние копии:\n{extra}", group.kept.display()),
            )
            .about([group.id.clone()]),
        );
    }
}

/// «1.6.4633 rev1260» → «1.6»; «1.5» → «1.5».
fn major_minor(version: &str) -> Option<String> {
    let head = version.trim().split_whitespace().next()?;
    let mut parts = head.split('.');
    let major = parts.next()?.trim();
    let minor = parts.next().unwrap_or("0").trim();
    if major.is_empty() || !major.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("{major}.{minor}"))
}

/// Есть ли среди диагностик хоть одна ошибка.
pub fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(|d| d.severity == Severity::Error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_data::{ModEntry, ModSource};

    struct B(ModEntry);

    impl B {
        fn new(id: &str) -> Self {
            B(ModEntry {
                name: id.to_string(),
                package_id: ModId::new(id),
                version: String::new(),
                author: String::new(),
                supported_versions: vec!["1.6".into()],
                path: std::path::PathBuf::from(format!("/mods/{id}")),
                source: ModSource::Local,
                dependencies: Vec::new(),
                load_after: Vec::new(),
                load_before: Vec::new(),
                incompatible_with: Vec::new(),
                description: String::new(),
                preview_path: None,
            })
        }
        fn core(mut self) -> Self {
            self.0.source = ModSource::Core;
            self
        }
        fn deps(mut self, ids: &[&str]) -> Self {
            self.0.dependencies = ids.iter().map(|s| ModId::new(s)).collect();
            self
        }
        fn after(mut self, ids: &[&str]) -> Self {
            self.0.load_after = ids.iter().map(|s| ModId::new(s)).collect();
            self
        }
        fn before(mut self, ids: &[&str]) -> Self {
            self.0.load_before = ids.iter().map(|s| ModId::new(s)).collect();
            self
        }
        fn incompatible(mut self, ids: &[&str]) -> Self {
            self.0.incompatible_with = ids.iter().map(|s| ModId::new(s)).collect();
            self
        }
        fn versions(mut self, vs: &[&str]) -> Self {
            self.0.supported_versions = vs.iter().map(|s| s.to_string()).collect();
            self
        }
    }

    /// Каталог из модов и сборка из перечисленных активных (в этом порядке).
    fn setup(mods: Vec<B>, active: &[&str]) -> (ModDb, Profile) {
        let db = ModDb::build(mods.into_iter().map(|b| b.0).collect());
        let mut profile = Profile::new();
        for id in active {
            profile.activate(ModId::new(id));
        }
        (db, profile)
    }

    fn rules(diags: &[Diagnostic]) -> Vec<&str> {
        diags.iter().map(|d| d.rule).collect()
    }

    fn core() -> B {
        B::new("ludeon.rimworld").core()
    }

    #[test]
    fn clean_profile_has_no_diagnostics() {
        let (db, profile) = setup(
            vec![core(), B::new("a.mod"), B::new("b.mod")],
            &["ludeon.rimworld", "a.mod", "b.mod"],
        );
        assert_eq!(validate(&db, &profile, Some("1.6.4633 rev1260")), []);
    }

    #[test]
    fn missing_dependency_is_reported_and_fixable() {
        let (db, profile) = setup(
            vec![core(), B::new("a.mod").deps(&["lib.mod"]), B::new("lib.mod")],
            &["ludeon.rimworld", "a.mod"],
        );
        let diags = validate(&db, &profile, None);

        let dep = diags.iter().find(|d| d.rule == "missing-dependency").expect("нет диагностики");
        assert_eq!(dep.severity, Severity::Error);
        assert_eq!(dep.fix, Some(Fix::Activate(ModId::new("lib.mod"))));
    }

    #[test]
    fn dependency_that_is_not_installed_has_no_fix() {
        let (db, profile) = setup(
            vec![core(), B::new("a.mod").deps(&["nowhere.mod"])],
            &["ludeon.rimworld", "a.mod"],
        );
        let diags = validate(&db, &profile, None);
        let dep = diags.iter().find(|d| d.rule == "missing-dependency").unwrap();
        assert_eq!(dep.fix, None, "включать нечего — мод не установлен");
        assert!(dep.detail.contains("не установлен"), "{}", dep.detail);
    }

    #[test]
    fn incompatibility_is_reported_once_per_pair() {
        // Несовместимость часто объявлена с обеих сторон.
        let (db, profile) = setup(
            vec![
                core(),
                B::new("a.mod").incompatible(&["b.mod"]),
                B::new("b.mod").incompatible(&["a.mod"]),
            ],
            &["ludeon.rimworld", "a.mod", "b.mod"],
        );
        let diags = validate(&db, &profile, None);
        assert_eq!(
            diags.iter().filter(|d| d.rule == "incompatible").count(),
            1,
            "{:#?}",
            diags,
        );
    }

    #[test]
    fn incompatibility_with_inactive_mod_is_fine() {
        let (db, profile) = setup(
            vec![core(), B::new("a.mod").incompatible(&["b.mod"]), B::new("b.mod")],
            &["ludeon.rimworld", "a.mod"],
        );
        assert!(!rules(&validate(&db, &profile, None)).contains(&"incompatible"));
    }

    #[test]
    fn wrong_load_order_is_reported() {
        // b.mod объявил loadAfter a.mod, но стоит раньше него.
        let (db, profile) = setup(
            vec![core(), B::new("a.mod"), B::new("b.mod").after(&["a.mod"])],
            &["ludeon.rimworld", "b.mod", "a.mod"],
        );
        let diags = validate(&db, &profile, None);
        let order = diags.iter().find(|d| d.rule == "load-order").expect("нет диагностики");
        assert_eq!(order.fix, Some(Fix::Sort));
        assert!(order.detail.contains("после"), "{}", order.detail);
    }

    #[test]
    fn correct_load_order_is_silent() {
        let (db, profile) = setup(
            vec![core(), B::new("a.mod"), B::new("b.mod").after(&["a.mod"])],
            &["ludeon.rimworld", "a.mod", "b.mod"],
        );
        assert!(!rules(&validate(&db, &profile, None)).contains(&"load-order"));
    }

    #[test]
    fn load_before_violation_is_reported() {
        let (db, profile) = setup(
            vec![core(), B::new("a.mod").before(&["b.mod"]), B::new("b.mod")],
            &["ludeon.rimworld", "b.mod", "a.mod"],
        );
        let diags = validate(&db, &profile, None);
        let order = diags.iter().find(|d| d.rule == "load-order").unwrap();
        assert!(order.detail.contains("до"), "{}", order.detail);
    }

    #[test]
    fn dependency_order_is_checked_too() {
        // modDependencies тоже задаёт порядок, не только loadAfter.
        let (db, profile) = setup(
            vec![core(), B::new("lib.mod"), B::new("a.mod").deps(&["lib.mod"])],
            &["ludeon.rimworld", "a.mod", "lib.mod"],
        );
        assert!(rules(&validate(&db, &profile, None)).contains(&"load-order"));
    }

    #[test]
    fn inactive_mods_do_not_affect_order_check() {
        let (db, profile) = setup(
            vec![core(), B::new("a.mod"), B::new("b.mod").after(&["a.mod"])],
            &["ludeon.rimworld", "b.mod"],
        );
        assert!(!rules(&validate(&db, &profile, None)).contains(&"load-order"));
    }

    #[test]
    fn version_mismatch_is_a_warning() {
        let (db, profile) = setup(
            vec![core(), B::new("old.mod").versions(&["1.4", "1.5"])],
            &["ludeon.rimworld", "old.mod"],
        );
        let diags = validate(&db, &profile, Some("1.6.4633 rev1260"));
        let v = diags.iter().find(|d| d.rule == "unsupported-version").expect("нет диагностики");
        assert_eq!(v.severity, Severity::Warning);
        assert!(v.detail.contains("1.4"), "{}", v.detail);
    }

    #[test]
    fn matching_version_is_silent() {
        let (db, profile) = setup(
            vec![core(), B::new("ok.mod").versions(&["1.5", "1.6"])],
            &["ludeon.rimworld", "ok.mod"],
        );
        assert!(!rules(&validate(&db, &profile, Some("1.6.4633"))).contains(&"unsupported-version"));
    }

    #[test]
    fn vanilla_content_is_not_version_checked() {
        // Core и DLC версионируются вместе с игрой и своих версий не заявляют.
        let (db, profile) = setup(vec![core().versions(&["1.4"])], &["ludeon.rimworld"]);
        assert!(!rules(&validate(&db, &profile, Some("1.6"))).contains(&"unsupported-version"));
    }

    #[test]
    fn core_must_be_active() {
        let (db, profile) = setup(vec![core(), B::new("a.mod")], &["a.mod"]);
        let diags = validate(&db, &profile, None);
        let core_diag = diags.iter().find(|d| d.rule == "core-inactive").expect("нет диагностики");
        assert_eq!(core_diag.fix, Some(Fix::Activate(ModId::new("ludeon.rimworld"))));
    }

    #[test]
    fn missing_core_is_reported() {
        let (db, profile) = setup(vec![B::new("a.mod")], &["a.mod"]);
        assert!(rules(&validate(&db, &profile, None)).contains(&"core-missing"));
    }

    #[test]
    fn duplicates_are_reported() {
        let mut a = B::new("a.mod");
        a.0.path = "/mods/first".into();
        let mut b = B::new("a.mod");
        b.0.path = "/steamcmd/second".into();
        let (db, profile) = setup(vec![core(), a, b], &["ludeon.rimworld"]);

        let diags = validate(&db, &profile, None);
        let dup = diags.iter().find(|d| d.rule == "duplicate").expect("нет диагностики");
        assert!(dup.detail.contains("/steamcmd/second"), "{}", dup.detail);
    }

    #[test]
    fn errors_come_before_warnings() {
        let (db, profile) = setup(
            vec![core(), B::new("a.mod").deps(&["lib.mod"]).versions(&["1.4"]), B::new("lib.mod")],
            &["ludeon.rimworld", "a.mod"],
        );
        let diags = validate(&db, &profile, Some("1.6"));
        let first_warning = diags.iter().position(|d| d.severity == Severity::Warning);
        let last_error = diags.iter().rposition(|d| d.severity == Severity::Error);
        assert!(matches!((first_warning, last_error), (Some(w), Some(e)) if e < w), "{diags:#?}");
        assert!(has_errors(&diags));
    }

    #[test]
    fn version_parsing() {
        assert_eq!(major_minor("1.6.4633 rev1260").as_deref(), Some("1.6"));
        assert_eq!(major_minor("1.5").as_deref(), Some("1.5"));
        assert_eq!(major_minor("  1.4.3  ").as_deref(), Some("1.4"));
        assert_eq!(major_minor("не версия"), None);
        assert_eq!(major_minor(""), None);
    }
}
