//! Кэши отображаемых списков модов.
//!
//! Фильтрация и анализ зависимостей стоят O(mods × deps) — считать это каждый
//! кадр нельзя (на 2000 модов старый код делал тысячи `to_lowercase()` за кадр).
//! Кэш пересчитывается только когда изменилась сборка ([`ListCaches::invalidate`])
//! или текст поиска.

use std::collections::HashMap;

use crate::mod_data::{ModDb, ModId, Profile};
use crate::tags::Tags;

#[derive(Default)]
pub struct SearchState {
    pub inactive_query: String,
    pub active_query: String,
    /// Тег, выбранный чипом над списками (имя в нижнем регистре). Фильтрует
    /// обе колонки сразу: искать «где-то среди моих модов» привычнее, чем
    /// помнить, в какой колонке мод сейчас лежит.
    pub tag_filter: Option<String>,
}

/// Предвычисленные флаги предупреждений для одного мода.
#[derive(Clone, Copy, Default)]
pub struct RowWarn {
    pub missing_deps: bool,
    pub incompat: bool,
}

pub struct ListCaches {
    /// Ключ поиска: lowercase "название\npackage_id".
    keys: HashMap<ModId, String>,
    pub warn: HashMap<ModId, RowWarn>,
    pub inactive: Vec<ModId>,
    pub active: Vec<ModId>,
    /// Сколько модов по запросу левой колонки нашлось бы в правой — и наоборот.
    /// Без этой подсказки пустой результат выглядит как «поиск сломан».
    pub inactive_elsewhere: usize,
    pub active_elsewhere: usize,
    last_inactive_q: String,
    last_active_q: String,
    last_tag: Option<String>,
    dirty: bool,
}

impl Default for ListCaches {
    fn default() -> Self {
        Self {
            keys: HashMap::new(),
            warn: HashMap::new(),
            inactive: Vec::new(),
            active: Vec::new(),
            inactive_elsewhere: 0,
            active_elsewhere: 0,
            last_inactive_q: String::new(),
            last_active_q: String::new(),
            last_tag: None,
            dirty: true,
        }
    }
}

impl ListCaches {
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub fn warn_for(&self, id: &ModId) -> RowWarn {
        self.warn.get(id).copied().unwrap_or_default()
    }

    pub fn refresh(&mut self, db: &ModDb, profile: &Profile, tags: &Tags, search: &SearchState) {
        let changed = self.dirty;
        if changed {
            self.dirty = false;

            self.keys.clear();
            self.warn.clear();
            for m in db.iter() {
                let mut key = m.name.to_lowercase();
                key.push('\n');
                key.push_str(m.package_id.as_str());
                // Теги попадают в тот же ключ как «tag:имя» — благодаря этому
                // запрос «tag:фреймворк» фильтрует обычным поиском подстроки,
                // без отдельной ветки разбора запроса.
                for (_, tag) in tags.tags_of(&m.package_id) {
                    key.push_str("\ntag:");
                    key.push_str(&tag.name.to_lowercase());
                }
                self.keys.insert(m.package_id.clone(), key);

                let is_active = profile.is_active(&m.package_id);
                self.warn.insert(
                    m.package_id.clone(),
                    RowWarn {
                        missing_deps: m.dependencies.iter().any(|d| !profile.is_active(d)),
                        incompat: is_active
                            && m.incompatible_with.iter().any(|ic| profile.is_active(ic)),
                    },
                );
            }
        }

        let tag_changed = search.tag_filter != self.last_tag;
        if tag_changed {
            self.last_tag = search.tag_filter.clone();
        }
        let tag = search.tag_filter.as_deref();

        if changed || tag_changed || search.inactive_query != self.last_inactive_q {
            self.last_inactive_q = search.inactive_query.clone();
            let q = Query::parse(&search.inactive_query, tag);
            self.inactive = db
                .ids()
                .filter(|id| !profile.is_active(id))
                .filter(|id| self.matches(id, &q))
                .cloned()
                .collect();
            self.inactive_elsewhere = if q.is_empty() {
                0
            } else {
                profile.order().iter().filter(|id| self.matches(id, &q)).count()
            };
        }

        if changed || tag_changed || search.active_query != self.last_active_q {
            self.last_active_q = search.active_query.clone();
            let q = Query::parse(&search.active_query, tag);
            self.active = profile
                .order()
                .iter()
                .filter(|id| self.matches(id, &q))
                .cloned()
                .collect();
            self.active_elsewhere = if q.is_empty() {
                0
            } else {
                db.ids().filter(|id| !profile.is_active(id) && self.matches(id, &q)).count()
            };
        }
    }

    fn matches(&self, id: &ModId, q: &Query) -> bool {
        if q.is_empty() {
            return true;
        }
        let Some(key) = self.keys.get(id) else { return false };
        q.terms.iter().all(|t| key.contains(t.as_str()))
            && q.tag.as_ref().is_none_or(|tag| key.lines().any(|l| l.strip_prefix("tag:") == Some(tag.as_str())))
    }
}

/// Разобранный запрос: слова через пробел ищутся все сразу, в любом порядке.
struct Query {
    terms: Vec<String>,
    /// Точное имя тега из чипа.
    tag: Option<String>,
}

impl Query {
    fn parse(raw: &str, tag: Option<&str>) -> Self {
        let lower = raw.to_lowercase();
        // «tag: имя» с пробелом — естественная опечатка; без склейки «tag:»
        // превращалось в отдельное слово и совпадало вообще с любым тегом.
        let mut glued = String::with_capacity(lower.len());
        let mut rest = lower.as_str();
        while let Some(at) = rest.find("tag:") {
            glued.push_str(&rest[..at + 4]);
            rest = rest[at + 4..].trim_start();
        }
        glued.push_str(rest);
        Self {
            terms: glued.split_whitespace().map(str::to_string).collect(),
            tag: tag.map(str::to_string),
        }
    }

    fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.tag.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_data::{ModEntry, ModSource};

    fn entry(package_id: &str, name: &str) -> ModEntry {
        ModEntry {
            name: name.to_string(),
            package_id: ModId::new(package_id),
            version: String::new(),
            author: String::new(),
            supported_versions: Vec::new(),
            path: std::path::PathBuf::from(format!("/mods/{package_id}")),
            source: ModSource::Local,
            dependencies: Vec::new(),
            dependency_sources: Vec::new(),
            load_after: Vec::new(),
            load_before: Vec::new(),
            incompatible_with: Vec::new(),
            description: String::new(),
            preview_path: None,
        }
    }

    /// Каталог из трёх модов, где «a.mod» помечен тегом «Фреймворк».
    fn fixture() -> (ModDb, Profile, Tags) {
        let db = ModDb::build(vec![
            entry("a.mod", "Alpha Framework"),
            entry("b.mod", "Bravo Content"),
            entry("c.mod", "Charlie Content"),
        ]);
        let mut tags = Tags::new();
        let framework = tags.create("Фреймворк").unwrap();
        tags.toggle(&ModId::new("a.mod"), framework);
        (db, Profile::new(), tags)
    }

    fn inactive_for(query: &str) -> Vec<String> {
        let (db, profile, tags) = fixture();
        let search = SearchState { inactive_query: query.to_string(), ..Default::default() };
        let mut caches = ListCaches::default();
        caches.refresh(&db, &profile, &tags, &search);
        caches.inactive.iter().map(|id| id.as_str().to_string()).collect()
    }

    #[test]
    fn empty_query_lists_everything() {
        assert_eq!(inactive_for(""), ["a.mod", "b.mod", "c.mod"]);
    }

    #[test]
    fn tag_query_keeps_only_tagged_mods() {
        assert_eq!(inactive_for("tag:фреймворк"), ["a.mod"]);
    }

    #[test]
    fn tag_query_is_case_insensitive_and_partial() {
        assert_eq!(inactive_for("TAG:Фрейм"), ["a.mod"]);
    }

    #[test]
    fn plain_query_still_matches_name_and_id() {
        assert_eq!(inactive_for("charlie"), ["c.mod"]);
        assert_eq!(inactive_for("b.mod"), ["b.mod"]);
        assert_eq!(inactive_for("content"), ["b.mod", "c.mod"]);
    }

    #[test]
    fn tag_name_does_not_leak_into_plain_search() {
        // «Фреймворк» — только тег, в названии мода его нет; но искать
        // по нему без префикса тоже допустимо: ключ у мода общий.
        assert_eq!(inactive_for("фреймворк"), ["a.mod"]);
        // А мод без тега по нему не находится.
        assert!(!inactive_for("фреймворк").contains(&"b.mod".to_string()));
    }

    #[test]
    fn tag_query_tolerates_space_after_colon() {
        assert_eq!(inactive_for("tag: Фреймворк"), ["a.mod"]);
    }

    #[test]
    fn words_match_in_any_order() {
        assert_eq!(inactive_for("content charlie"), ["c.mod"]);
    }

    #[test]
    fn tag_chip_filters_and_counts_other_column() {
        let (db, _, tags) = fixture();
        let mut profile = Profile::new();
        profile.activate(ModId::new("a.mod"));
        let search = SearchState { tag_filter: Some("фреймворк".into()), ..Default::default() };
        let mut caches = ListCaches::default();
        caches.refresh(&db, &profile, &tags, &search);
        // Помеченный мод активен: слева пусто, но подсказка знает, где он.
        assert!(caches.inactive.is_empty());
        assert_eq!(caches.inactive_elsewhere, 1);
        assert_eq!(caches.active.len(), 1);
    }

    #[test]
    fn active_list_follows_profile_order() {
        let (db, _, tags) = fixture();
        let mut profile = Profile::new();
        profile.activate(ModId::new("c.mod"));
        profile.activate(ModId::new("a.mod"));

        let mut caches = ListCaches::default();
        caches.refresh(&db, &profile, &tags, &SearchState::default());

        let active: Vec<&str> = caches.active.iter().map(ModId::as_str).collect();
        let inactive: Vec<&str> = caches.inactive.iter().map(ModId::as_str).collect();
        assert_eq!(active, ["c.mod", "a.mod"], "порядок загрузки должен сохраняться");
        assert_eq!(inactive, ["b.mod"]);
    }
}
