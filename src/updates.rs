//! Проверка обновлений модов мастерской.
//!
//! Узнать «свежая ли у меня версия» неоткуда: в About.xml номера версии либо
//! нет, либо он произвольный и авторы его не поднимают. Единственный надёжный
//! признак — время последнего обновления предмета в мастерской, а его отдаёт
//! `GetPublishedFileDetails` (работает без ключа API).
//!
//! Локальная сторона — время файлов на диске. SteamCMD переписывает предмет
//! целиком, поэтому mtime у `About.xml` равен моменту скачивания. Проверено на
//! живой установке: у модов, скачанных 19 апреля, mtime именно 19 апреля, а
//! мастерская сообщает более ранние даты обновления — то есть всё актуально.
//!
//! Файла состояния при этом не нужно, и это важно: работает и для модов,
//! поставленных задолго до появления этого лаунчера.

use std::collections::HashMap;

use crate::mod_data::{ModDb, ModId, ModSource};
use crate::steam::workshop_api::{self, PublishedFile};

/// Мод, для которого в мастерской есть версия новее установленной.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub id: ModId,
    pub workshop_id: u64,
    /// Название из мастерской — оно может отличаться от локального.
    pub title: String,
    /// Когда мод обновили в мастерской, секунды с эпохи.
    pub updated: u64,
    /// Когда мы его скачали, секунды с эпохи.
    pub installed: u64,
}

impl Update {
    /// На сколько мы отстали.
    pub fn behind(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.updated.saturating_sub(self.installed))
    }
}

/// Установленный мод из мастерской: что и когда скачано.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub id: ModId,
    pub workshop_id: u64,
    pub installed: u64,
}

/// Часы могут разойтись, а файловые системы — округлять время. Отставание
/// меньше этого за обновление не считаем.
const TOLERANCE: u64 = 60 * 60;

/// Собирает моды из мастерской и время их установки.
///
/// Локальные моды и ваниль пропускаются: обновлять через мастерскую нечего.
pub fn installed_workshop_mods(db: &ModDb) -> Vec<Installed> {
    db.iter()
        .filter_map(|m| {
            let ModSource::Workshop(workshop_id) = m.source else { return None };
            Some(Installed {
                id: m.package_id.clone(),
                workshop_id,
                installed: about_mtime(&m.path)?,
            })
        })
        .collect()
}

/// Время последнего изменения About.xml мода, секунды с эпохи.
fn about_mtime(mod_dir: &std::path::Path) -> Option<u64> {
    let meta = std::fs::metadata(mod_dir.join("About").join("About.xml")).ok()?;
    meta.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Спрашивает мастерскую и оставляет только то, что устарело.
pub fn check(mods: &[Installed]) -> anyhow::Result<Vec<Update>> {
    if mods.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<u64> = mods.iter().map(|m| m.workshop_id).collect();
    let remote = workshop_api::fetch_published_files(&ids)?;
    Ok(compare(mods, &remote))
}

/// Сводит локальное состояние с ответом мастерской.
///
/// Вынесено отдельно от запроса, чтобы правило сравнения проверялось тестами
/// без обращения к сети.
pub fn compare(mods: &[Installed], remote: &[PublishedFile]) -> Vec<Update> {
    let by_id: HashMap<u64, &PublishedFile> = remote.iter().map(|f| (f.id, f)).collect();

    let mut out: Vec<Update> = mods
        .iter()
        .filter_map(|local| {
            let file = by_id.get(&local.workshop_id)?;
            if file.time_updated <= local.installed.saturating_add(TOLERANCE) {
                return None;
            }
            Some(Update {
                id: local.id.clone(),
                workshop_id: local.workshop_id,
                title: file.title.clone(),
                updated: file.time_updated,
                installed: local.installed,
            })
        })
        .collect();

    // Самые залежавшиеся сверху — их обновлять важнее.
    out.sort_by(|a, b| b.behind().cmp(&a.behind()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 60 * 60 * 24;

    fn local(id: &str, workshop_id: u64, installed: u64) -> Installed {
        Installed { id: ModId::new(id), workshop_id, installed }
    }

    fn remote(id: u64, time_updated: u64) -> PublishedFile {
        PublishedFile { id, title: format!("Mod {id}"), time_updated }
    }

    #[test]
    fn a_newer_workshop_version_is_an_update() {
        let updates = compare(
            &[local("a.mod", 111, 100 * DAY)],
            &[remote(111, 110 * DAY)],
        );
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].id, ModId::new("a.mod"));
        assert_eq!(updates[0].behind(), std::time::Duration::from_secs(10 * DAY));
    }

    #[test]
    fn an_older_workshop_version_is_not_an_update() {
        // Обычный случай на живой установке: скачали позже, чем автор обновлял.
        let updates = compare(
            &[local("a.mod", 111, 110 * DAY)],
            &[remote(111, 100 * DAY)],
        );
        assert!(updates.is_empty(), "{updates:?}");
    }

    #[test]
    fn a_small_difference_is_not_an_update() {
        // Часы и округление времени файловой системы не должны порождать
        // обновление на ровном месте.
        let updates = compare(
            &[local("a.mod", 111, 100 * DAY)],
            &[remote(111, 100 * DAY + 60)],
        );
        assert!(updates.is_empty(), "{updates:?}");
    }

    #[test]
    fn mods_the_workshop_does_not_know_are_skipped() {
        // Мод удалили из мастерской — обновлять нечем, но и падать не из-за чего.
        let updates = compare(&[local("gone.mod", 999, 100 * DAY)], &[]);
        assert!(updates.is_empty());
    }

    #[test]
    fn the_most_outdated_comes_first() {
        let updates = compare(
            &[
                local("fresh.mod", 111, 100 * DAY),
                local("stale.mod", 222, 10 * DAY),
                local("middle.mod", 333, 50 * DAY),
            ],
            &[remote(111, 110 * DAY), remote(222, 110 * DAY), remote(333, 110 * DAY)],
        );
        let order: Vec<&str> = updates.iter().map(|u| u.id.as_str()).collect();
        assert_eq!(order, ["stale.mod", "middle.mod", "fresh.mod"]);
    }

    #[test]
    fn nothing_to_check_makes_no_request() {
        // Пустой список не должен уходить в сеть — иначе проверка «обновлений
        // нет» падала бы без интернета.
        assert_eq!(check(&[]).unwrap(), Vec::new());
    }
}
