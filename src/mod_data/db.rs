use std::collections::HashMap;
use std::path::PathBuf;

use super::{ModEntry, ModId, ModSource};

/// Мод, установленный в нескольких местах: какая копия оставлена и какие
/// отброшены. Отброшенные показываются в диалоге дубликатов и могут быть
/// удалены с диска.
#[derive(Clone, Debug)]
pub struct DuplicateGroup {
    pub id: ModId,
    pub kept: PathBuf,
    pub discarded: Vec<PathBuf>,
}

/// Каталог установленных модов: по одной записи на `ModId`.
///
/// Дубликаты packageId (один мод в Mods/ и в папке SteamCMD) схлопываются:
/// побеждает первая найденная копия — как и в игре, где packageId уникален.
/// Отброшенные копии не теряются, а уезжают в [`ModDb::duplicates`].
pub struct ModDb {
    mods: Vec<ModEntry>,
    index: HashMap<ModId, usize>,
    by_workshop: HashMap<u64, ModId>,
    duplicates: Vec<DuplicateGroup>,
}

impl Default for ModDb {
    fn default() -> Self {
        Self::empty()
    }
}

impl ModDb {
    pub fn empty() -> Self {
        Self {
            mods: Vec::new(),
            index: HashMap::new(),
            by_workshop: HashMap::new(),
            duplicates: Vec::new(),
        }
    }

    /// Собирает каталог из результатов сканирования.
    ///
    /// Порядок `entries` определяет, какая копия считается канонической, и
    /// задаёт порядок неактивного списка в UI. Сканер отдаёт Core и DLC
    /// первыми, поэтому ванильный контент никогда не проигрывает копии из
    /// Workshop.
    pub fn build(entries: Vec<ModEntry>) -> Self {
        let mut db = Self::empty();
        // dup_index: ModId → позиция в db.duplicates
        let mut dup_index: HashMap<ModId, usize> = HashMap::new();

        for entry in entries {
            if entry.package_id.is_empty() {
                tracing::warn!("Mod without packageId at {:?}, skipped", entry.path);
                continue;
            }

            if let Some(&kept_idx) = db.index.get(&entry.package_id) {
                let id = entry.package_id.clone();
                match dup_index.get(&id) {
                    Some(&d) => db.duplicates[d].discarded.push(entry.path),
                    None => {
                        dup_index.insert(id.clone(), db.duplicates.len());
                        db.duplicates.push(DuplicateGroup {
                            id,
                            kept: db.mods[kept_idx].path.clone(),
                            discarded: vec![entry.path],
                        });
                    }
                }
                continue;
            }

            if let ModSource::Workshop(wid) = entry.source {
                db.by_workshop.insert(wid, entry.package_id.clone());
            }
            db.index.insert(entry.package_id.clone(), db.mods.len());
            db.mods.push(entry);
        }

        db.duplicates.sort_by(|a, b| a.id.cmp(&b.id));
        db
    }

    pub fn get(&self, id: &ModId) -> Option<&ModEntry> {
        self.index.get(id).map(|&i| &self.mods[i])
    }

    pub fn contains(&self, id: &ModId) -> bool {
        self.index.contains_key(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ModEntry> {
        self.mods.iter()
    }

    pub fn ids(&self) -> impl Iterator<Item = &ModId> {
        self.mods.iter().map(|m| &m.package_id)
    }

    pub fn len(&self) -> usize {
        self.mods.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mods.is_empty()
    }

    pub fn duplicates(&self) -> &[DuplicateGroup] {
        &self.duplicates
    }

    /// Забывает информацию о дубликатах (после удаления копий с диска).
    pub fn clear_duplicates(&mut self) {
        self.duplicates.clear();
    }

    /// Разрешает строку из файла-списка в идентификатор мода.
    ///
    /// Списки сборок бывают двух видов: с packageId (ModsConfig.xml, RimSort)
    /// и с числовыми Workshop ID — поддерживаем оба.
    pub fn resolve(&self, raw: &str) -> Option<ModId> {
        let id = ModId::new(raw);
        if self.index.contains_key(&id) {
            return Some(id);
        }
        raw.trim()
            .parse::<u64>()
            .ok()
            .and_then(|wid| self.by_workshop.get(&wid))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(package_id: &str, path: &str, source: ModSource) -> ModEntry {
        ModEntry {
            name: package_id.to_string(),
            package_id: ModId::new(package_id),
            version: String::new(),
            author: String::new(),
            supported_versions: Vec::new(),
            path: PathBuf::from(path),
            source,
            dependencies: Vec::new(),
            dependency_sources: Vec::new(),
            load_after: Vec::new(),
            load_before: Vec::new(),
            incompatible_with: Vec::new(),
            description: String::new(),
            preview_path: None,
        }
    }

    #[test]
    fn first_copy_wins_and_rest_become_duplicates() {
        let db = ModDb::build(vec![
            entry("a.mod", "/mods/a", ModSource::Local),
            entry("a.mod", "/steamcmd/a", ModSource::Local),
            entry("a.mod", "/other/a", ModSource::Local),
            entry("b.mod", "/mods/b", ModSource::Local),
        ]);

        assert_eq!(db.len(), 2);
        assert_eq!(db.get(&ModId::new("a.mod")).unwrap().path, PathBuf::from("/mods/a"));

        let dupes = db.duplicates();
        assert_eq!(dupes.len(), 1);
        assert_eq!(dupes[0].kept, PathBuf::from("/mods/a"));
        assert_eq!(
            dupes[0].discarded,
            vec![PathBuf::from("/steamcmd/a"), PathBuf::from("/other/a")],
        );
    }

    #[test]
    fn resolves_by_package_id_and_workshop_id() {
        let db = ModDb::build(vec![
            entry("a.mod", "/mods/12345", ModSource::Workshop(12345)),
        ]);

        assert_eq!(db.resolve("A.Mod"), Some(ModId::new("a.mod")));
        assert_eq!(db.resolve("12345"), Some(ModId::new("a.mod")));
        assert_eq!(db.resolve("nope"), None);
    }

    #[test]
    fn entries_without_package_id_are_skipped() {
        let db = ModDb::build(vec![entry("", "/mods/broken", ModSource::Local)]);
        assert!(db.is_empty());
    }
}
