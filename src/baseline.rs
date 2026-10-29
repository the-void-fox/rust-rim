//! «Рабочая сборка» — от неё отсчитывается, какие моды новые.
//!
//! Типичная история: собрал сборку, всё работает, докинул 5–10 модов —
//! сломалось, а какие именно докинул, уже не вспомнить. Поэтому менеджер
//! запоминает набор модов, про который известно, что он работал: вручную
//! по кнопке или сам, когда тест игры прошёл. Всё, что включено сверх этого
//! набора, помечается в списке как «новое».

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::mod_data::ModId;

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    /// Когда запомнили, секунды с эпохи.
    set_at: u64,
    mods: Vec<String>,
}

#[derive(Default)]
pub struct Baseline {
    /// `None` — рабочую сборку ещё ни разу не отмечали: тогда новых нет.
    mods: Option<HashSet<ModId>>,
    set_at: u64,
}

impl Baseline {
    pub fn is_set(&self) -> bool {
        self.mods.is_some()
    }

    /// Когда отмечена, секунды с эпохи.
    pub fn set_at(&self) -> u64 {
        self.set_at
    }

    /// Запоминает набор модов как рабочий.
    pub fn remember<'a>(&mut self, active: impl IntoIterator<Item = &'a ModId>) {
        self.mods = Some(active.into_iter().cloned().collect());
        self.set_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
    }

    pub fn forget(&mut self) {
        self.mods = None;
        self.set_at = 0;
    }

    /// Включён после рабочей сборки.
    pub fn is_new(&self, id: &ModId) -> bool {
        self.mods.as_ref().is_some_and(|m| !m.contains(id))
    }

    /// Новые среди активных, в порядке загрузки.
    pub fn new_among<'a>(&self, active: &'a [ModId]) -> Vec<&'a ModId> {
        active.iter().filter(|id| self.is_new(id)).collect()
    }

    fn file_path() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("com", "rustrim", "RustRim")
            .map(|d| d.config_dir().join("baseline.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::file_path() else { return Self::default() };
        let Ok(data) = std::fs::read_to_string(&path) else { return Self::default() };
        match serde_json::from_str::<Stored>(&data) {
            Ok(s) => Self {
                mods: Some(s.mods.iter().map(|m| ModId::new(m)).collect()),
                set_at: s.set_at,
            },
            Err(e) => {
                tracing::warn!("Cannot read baseline.json: {}", e);
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::file_path() else { return };
        let Some(mods) = &self.mods else {
            let _ = std::fs::remove_file(&path);
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut list: Vec<String> = mods.iter().map(|m| m.as_str().to_string()).collect();
        list.sort();
        let stored = Stored { set_at: self.set_at, mods: list };
        match serde_json::to_string_pretty(&stored) {
            Ok(data) => {
                if let Err(e) = crate::fs_util::write_atomic(&path, data.as_bytes()) {
                    tracing::error!("Cannot write baseline.json: {}", e);
                }
            }
            Err(e) => tracing::error!("Cannot serialize baseline: {}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<ModId> {
        list.iter().map(|s| ModId::new(s)).collect()
    }

    #[test]
    fn без_отметки_новых_нет() {
        let b = Baseline::default();
        assert!(!b.is_new(&ModId::new("a")));
    }

    #[test]
    fn новые_это_включённые_после_отметки() {
        let mut b = Baseline::default();
        b.remember(&ids(&["core", "a"]));
        let active = ids(&["core", "a", "b", "c"]);
        let fresh: Vec<&str> = b.new_among(&active).iter().map(|m| m.as_str()).collect();
        assert_eq!(fresh, ["b", "c"]);
    }
}
