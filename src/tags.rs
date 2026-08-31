//! Пользовательские теги модов: произвольные метки с цветом.
//!
//! Хранятся отдельно от каталога модов: это данные пользователя, а не то,
//! что прочитано с диска. Привязка — по [`ModId`], поэтому переустановка
//! мода или пересканирование папки теги не теряет.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::mod_data::ModId;

/// Индекс тега в [`Tags::all`]. Живёт только в рамках сессии — на диск
/// теги сохраняются по имени, чтобы файл оставался читаемым и переживал
/// изменение порядка.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TagId(usize);

impl TagId {
    /// Порядковый номер тега — нужен UI как ключ виджетов.
    pub fn index(self) -> usize {
        self.0
    }
}

pub type Rgb = [u8; 3];

/// Палитра для новых тегов: перебирается по кругу, чтобы соседние теги
/// не оказывались одного цвета.
pub const PALETTE: [Rgb; 8] = [
    [100, 160, 240], // синий
    [80, 200, 120],  // зелёный
    [240, 180, 60],  // янтарный
    [220, 75, 75],   // красный
    [180, 130, 240], // фиолетовый
    [90, 200, 200],  // бирюзовый
    [240, 140, 90],  // оранжевый
    [200, 120, 170], // розовый
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub color: Rgb,
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    tags: Vec<Tag>,
    /// packageId → имена тегов.
    assigned: HashMap<String, Vec<String>>,
}

#[derive(Default)]
pub struct Tags {
    tags: Vec<Tag>,
    assigned: HashMap<ModId, Vec<usize>>,
}

impl Tags {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Теги ────────────────────────────────────────────────────────────────

    pub fn all(&self) -> &[Tag] {
        &self.tags
    }

    pub fn get(&self, id: TagId) -> Option<&Tag> {
        self.tags.get(id.0)
    }

    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = TagId> {
        (0..self.tags.len()).map(TagId)
    }

    /// Ищет тег по имени без учёта регистра.
    pub fn find(&self, name: &str) -> Option<TagId> {
        let needle = name.trim().to_lowercase();
        self.tags
            .iter()
            .position(|t| t.name.to_lowercase() == needle)
            .map(TagId)
    }

    /// Создаёт тег или возвращает существующий с таким же именем.
    pub fn create(&mut self, name: &str) -> Option<TagId> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(id) = self.find(name) {
            return Some(id);
        }
        let color = PALETTE[self.tags.len() % PALETTE.len()];
        self.tags.push(Tag { name: name.to_string(), color });
        Some(TagId(self.tags.len() - 1))
    }

    /// Переименовывает тег. Отказывает, если имя пустое или уже занято другим.
    pub fn rename(&mut self, id: TagId, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        if self.find(name).is_some_and(|existing| existing != id) {
            return false;
        }
        match self.tags.get_mut(id.0) {
            Some(tag) => {
                tag.name = name.to_string();
                true
            }
            None => false,
        }
    }

    pub fn set_color(&mut self, id: TagId, color: Rgb) {
        if let Some(tag) = self.tags.get_mut(id.0) {
            tag.color = color;
        }
    }

    /// Удаляет тег и снимает его со всех модов.
    pub fn delete(&mut self, id: TagId) {
        if id.0 >= self.tags.len() {
            return;
        }
        self.tags.remove(id.0);
        // Индексы правее сдвинулись — чиним привязки.
        for list in self.assigned.values_mut() {
            list.retain(|&i| i != id.0);
            for i in list.iter_mut() {
                if *i > id.0 {
                    *i -= 1;
                }
            }
        }
        self.assigned.retain(|_, list| !list.is_empty());
    }

    // ── Привязка к модам ────────────────────────────────────────────────────

    pub fn of(&self, mod_id: &ModId) -> &[usize] {
        self.assigned.get(mod_id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn tags_of(&self, mod_id: &ModId) -> impl Iterator<Item = (TagId, &Tag)> {
        self.of(mod_id)
            .iter()
            .filter_map(|&i| self.tags.get(i).map(|t| (TagId(i), t)))
    }

    pub fn has(&self, mod_id: &ModId, tag: TagId) -> bool {
        self.of(mod_id).contains(&tag.0)
    }

    pub fn toggle(&mut self, mod_id: &ModId, tag: TagId) {
        if tag.0 >= self.tags.len() {
            return;
        }
        let list = self.assigned.entry(mod_id.clone()).or_default();
        match list.iter().position(|&i| i == tag.0) {
            Some(at) => {
                list.remove(at);
            }
            None => list.push(tag.0),
        }
        if list.is_empty() {
            self.assigned.remove(mod_id);
        }
    }

    /// Цвет для полоски в строке списка — по первому назначенному тегу.
    pub fn stripe_color(&self, mod_id: &ModId) -> Option<Rgb> {
        let first = *self.of(mod_id).first()?;
        self.tags.get(first).map(|t| t.color)
    }

    /// Сколько модов помечено этим тегом.
    pub fn usage(&self, tag: TagId) -> usize {
        self.assigned.values().filter(|l| l.contains(&tag.0)).count()
    }

    // ── Хранение ────────────────────────────────────────────────────────────

    fn file_path() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("com", "rustrim", "RustRim")
            .map(|d| d.config_dir().join("tags.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::file_path() else { return Self::new() };
        let Ok(data) = std::fs::read_to_string(&path) else { return Self::new() };
        match serde_json::from_str::<Stored>(&data) {
            Ok(stored) => Self::from_stored(stored),
            Err(e) => {
                tracing::warn!("Cannot read tags.json: {}", e);
                Self::new()
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::file_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self.to_stored()) {
            Ok(data) => {
                if let Err(e) = std::fs::write(&path, data) {
                    tracing::error!("Cannot write tags.json: {}", e);
                }
            }
            Err(e) => tracing::error!("Cannot serialize tags: {}", e),
        }
    }

    fn to_stored(&self) -> Stored {
        let assigned = self
            .assigned
            .iter()
            .map(|(mod_id, list)| {
                let names = list
                    .iter()
                    .filter_map(|&i| self.tags.get(i).map(|t| t.name.clone()))
                    .collect();
                (mod_id.as_str().to_string(), names)
            })
            .collect();
        Stored { tags: self.tags.clone(), assigned }
    }

    fn from_stored(stored: Stored) -> Self {
        let mut tags = Self { tags: stored.tags, assigned: HashMap::new() };
        for (raw_id, names) in stored.assigned {
            let mod_id = ModId::new(&raw_id);
            for name in names {
                // Имя, которого нет в списке тегов, просто игнорируем:
                // файл могли отредактировать руками.
                if let Some(id) = tags.find(&name) {
                    let list = tags.assigned.entry(mod_id.clone()).or_default();
                    if !list.contains(&id.0) {
                        list.push(id.0);
                    }
                }
            }
        }
        tags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mod_id(s: &str) -> ModId {
        ModId::new(s)
    }

    #[test]
    fn create_is_idempotent_and_case_insensitive() {
        let mut t = Tags::new();
        let a = t.create("Фреймворк").unwrap();
        let b = t.create("фреймворк").unwrap();
        assert_eq!(a, b);
        assert_eq!(t.all().len(), 1);
        assert_eq!(t.create("   ").map(|_| ()), None);
    }

    #[test]
    fn new_tags_cycle_through_palette() {
        let mut t = Tags::new();
        for i in 0..PALETTE.len() + 1 {
            t.create(&format!("t{i}")).unwrap();
        }
        assert_eq!(t.all()[0].color, PALETTE[0]);
        assert_eq!(t.all()[PALETTE.len()].color, PALETTE[0]);
    }

    #[test]
    fn toggle_adds_and_removes() {
        let mut t = Tags::new();
        let tag = t.create("VFE").unwrap();
        let m = mod_id("a.mod");

        t.toggle(&m, tag);
        assert!(t.has(&m, tag));
        t.toggle(&m, tag);
        assert!(!t.has(&m, tag));
        assert!(t.stripe_color(&m).is_none());
    }

    #[test]
    fn stripe_color_follows_first_tag() {
        let mut t = Tags::new();
        let a = t.create("a").unwrap();
        let b = t.create("b").unwrap();
        let m = mod_id("x.mod");
        t.toggle(&m, b);
        t.toggle(&m, a);
        assert_eq!(t.stripe_color(&m), Some(t.get(b).unwrap().color));
    }

    #[test]
    fn delete_shifts_remaining_assignments() {
        // Индексы тегов сдвигаются при удалении — привязки не должны «съехать».
        let mut t = Tags::new();
        let a = t.create("a").unwrap();
        let b = t.create("b").unwrap();
        let c = t.create("c").unwrap();
        let m = mod_id("x.mod");
        t.toggle(&m, a);
        t.toggle(&m, c);

        t.delete(b);

        assert_eq!(t.all().len(), 2);
        let names: Vec<&str> = t.tags_of(&m).map(|(_, tag)| tag.name.as_str()).collect();
        assert_eq!(names, ["a", "c"]);
    }

    #[test]
    fn delete_removes_tag_from_mods() {
        let mut t = Tags::new();
        let a = t.create("a").unwrap();
        let m = mod_id("x.mod");
        t.toggle(&m, a);
        t.delete(a);
        assert!(t.tags_of(&m).next().is_none());
        assert_eq!(t.usage(a), 0);
    }

    #[test]
    fn rename_rejects_duplicates_and_empty() {
        let mut t = Tags::new();
        let a = t.create("a").unwrap();
        t.create("b").unwrap();

        assert!(!t.rename(a, "B"), "занятое имя не должно приниматься");
        assert!(!t.rename(a, "  "), "пустое имя не должно приниматься");
        assert!(t.rename(a, "A"), "смена регистра своего же имени допустима");
        assert_eq!(t.all()[0].name, "A");
    }

    #[test]
    fn round_trips_through_storage() {
        let mut t = Tags::new();
        let a = t.create("фреймворк").unwrap();
        let b = t.create("мусор").unwrap();
        t.set_color(b, [1, 2, 3]);
        let m = mod_id("Some.Mod");
        t.toggle(&m, a);
        t.toggle(&m, b);

        let restored = Tags::from_stored(t.to_stored());

        assert_eq!(restored.all().len(), 2);
        assert_eq!(restored.get(b).unwrap().color, [1, 2, 3]);
        let names: Vec<&str> = restored.tags_of(&m).map(|(_, tag)| tag.name.as_str()).collect();
        assert_eq!(names, ["фреймворк", "мусор"]);
    }

    #[test]
    fn unknown_tag_names_in_file_are_ignored() {
        // tags.json могли отредактировать руками.
        let stored = Stored {
            tags: vec![Tag { name: "есть".into(), color: [0, 0, 0] }],
            assigned: HashMap::from([(
                "a.mod".to_string(),
                vec!["есть".to_string(), "нет такого".to_string()],
            )]),
        };
        let t = Tags::from_stored(stored);
        let names: Vec<&str> = t.tags_of(&mod_id("a.mod")).map(|(_, tag)| tag.name.as_str()).collect();
        assert_eq!(names, ["есть"]);
    }
}
