use std::borrow::Borrow;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Идентификатор мода — нормализованный (всегда нижний регистр) packageId.
///
/// Это идентичность мода во всей программе: ModsConfig.xml, зависимости,
/// правила сообщества и анализатор логов ключуются именно по packageId,
/// поэтому отдельного синтетического ключа не заводим.
///
/// Раньше моды адресовались индексом в `Vec<ModEntry>`, и любая пересортировка
/// (`sort_active_mods`, `MoveUp`) незаметно меняла смысл сохранённого индекса —
/// выделение «перепрыгивало» на соседний мод.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModId(String);

impl ModId {
    /// Нормализует произвольную строку в идентификатор.
    pub fn new(raw: &str) -> Self {
        ModId(raw.trim().to_lowercase())
    }

    /// Для строк, уже приведённых к нижнему регистру (данные сканера).
    /// При несоблюдении инварианта нормализует.
    pub fn from_normalized(s: String) -> Self {
        if s.bytes().any(|b| b.is_ascii_uppercase()) || s.trim().len() != s.len() {
            return ModId::new(&s);
        }
        ModId(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

// Позволяет искать в HashMap<ModId, _> по &str без аллокации.
impl Borrow<str> for ModId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ModId {
    fn from(s: &str) -> Self {
        ModId::new(s)
    }
}

impl From<String> for ModId {
    fn from(s: String) -> Self {
        ModId::from_normalized(s)
    }
}

impl fmt::Display for ModId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_case_and_whitespace() {
        assert_eq!(ModId::new("  Brrainz.Harmony ").as_str(), "brrainz.harmony");
        assert_eq!(ModId::new("Brrainz.Harmony"), ModId::new("brrainz.harmony"));
    }

    #[test]
    fn from_normalized_repairs_bad_input() {
        assert_eq!(ModId::from_normalized("Foo.Bar".into()).as_str(), "foo.bar");
        assert_eq!(ModId::from_normalized("foo.bar".into()).as_str(), "foo.bar");
    }

    #[test]
    fn looks_up_by_str() {
        let mut map = std::collections::HashMap::new();
        map.insert(ModId::new("a.b"), 1);
        assert_eq!(map.get("a.b"), Some(&1));
    }
}
