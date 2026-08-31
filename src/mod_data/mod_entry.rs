use serde::{Deserialize, Serialize};

use super::ModId;

/// Неизменяемые метаданные установленного мода (то, что прочитано из About.xml).
///
/// Признака «активен» здесь намеренно нет: включённость и порядок загрузки —
/// свойство сборки, а не мода, и живут в [`super::Profile`]. Иначе одна и та же
/// запись описывала бы и мод на диске, и состояние конкретной сборки.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModEntry {
    pub name: String,
    pub package_id: ModId,
    pub version: String,
    pub author: String,
    pub supported_versions: Vec<String>,
    pub path: std::path::PathBuf,
    pub source: ModSource,

    // Зависимости и порядок загрузки
    pub dependencies: Vec<ModId>,
    pub load_after: Vec<ModId>,
    pub load_before: Vec<ModId>,
    pub incompatible_with: Vec<ModId>,

    // Метаданные превью
    pub description: String,
    pub preview_path: Option<std::path::PathBuf>,
}

impl ModEntry {
    pub fn id(&self) -> &ModId {
        &self.package_id
    }

    pub fn is_core(&self) -> bool {
        self.source == ModSource::Core
    }

    /// Core или DLC — ванильный контент, который нельзя выключить или удалить.
    pub fn is_vanilla(&self) -> bool {
        matches!(self.source, ModSource::Core | ModSource::DLC(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ModSource {
    Core,           // Data/Core
    DLC(String),    // Data/Royalty, etc.
    Local,          // Mods/
    Workshop(u64),  // Steam Workshop ID
}
