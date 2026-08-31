//! Кэши отображаемых списков модов.
//!
//! Фильтрация и анализ зависимостей стоят O(mods × deps) — считать это каждый
//! кадр нельзя (на 2000 модов старый код делал тысячи `to_lowercase()` за кадр).
//! Кэш пересчитывается только когда изменилась сборка ([`ListCaches::invalidate`])
//! или текст поиска.

use std::collections::HashMap;

use crate::mod_data::{ModDb, ModId, Profile};

#[derive(Default)]
pub struct SearchState {
    pub inactive_query: String,
    pub active_query: String,
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
    last_inactive_q: String,
    last_active_q: String,
    dirty: bool,
}

impl Default for ListCaches {
    fn default() -> Self {
        Self {
            keys: HashMap::new(),
            warn: HashMap::new(),
            inactive: Vec::new(),
            active: Vec::new(),
            last_inactive_q: String::new(),
            last_active_q: String::new(),
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

    pub fn refresh(&mut self, db: &ModDb, profile: &Profile, search: &SearchState) {
        let changed = self.dirty;
        if changed {
            self.dirty = false;

            self.keys.clear();
            self.warn.clear();
            for m in db.iter() {
                let mut key = m.name.to_lowercase();
                key.push('\n');
                key.push_str(m.package_id.as_str());
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

        if changed || search.inactive_query != self.last_inactive_q {
            self.last_inactive_q = search.inactive_query.clone();
            let q = search.inactive_query.to_lowercase();
            let list: Vec<ModId> = db
                .ids()
                .filter(|id| !profile.is_active(id))
                .filter(|id| q.is_empty() || self.matches(id, &q))
                .cloned()
                .collect();
            self.inactive = list;
        }

        if changed || search.active_query != self.last_active_q {
            self.last_active_q = search.active_query.clone();
            let q = search.active_query.to_lowercase();
            let list: Vec<ModId> = profile
                .order()
                .iter()
                .filter(|id| q.is_empty() || self.matches(id, &q))
                .cloned()
                .collect();
            self.active = list;
        }
    }

    fn matches(&self, id: &ModId, query: &str) -> bool {
        self.keys.get(id).is_some_and(|k| k.contains(query))
    }
}
