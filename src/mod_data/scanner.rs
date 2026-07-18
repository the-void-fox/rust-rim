use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::mod_data::{ModEntry, ModSource};
use super::parser::parse_about_xml;

fn lower_ids(ids: Vec<String>) -> Vec<String> {
    ids.into_iter().map(|s| s.to_lowercase()).collect()
}

/// Ищет превью-изображение в папке `mod_dir/About/` без учёта регистра.
/// Поддерживает: Preview.png, preview.png, Preview.jpg, preview.jpeg и т.д.
fn find_preview_image(mod_dir: &Path) -> Option<std::path::PathBuf> {
    let about_dir = mod_dir.join("About");
    let entries = std::fs::read_dir(&about_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_lowercase();
            if matches!(lower.as_str(),
                "preview.png" | "preview.jpg" | "preview.jpeg" | "preview.webp" | "preview.gif"
            ) {
                return Some(path);
            }
        }
    }
    None
}

// ─── Дисковый кэш результатов скана ──────────────────────────────────────────
// Парсинг About.xml тысяч модов — самая дорогая часть старта. Результат
// кэшируется на диске по mtime About.xml: повторный запуск читает только
// изменившиеся моды. Промахи парсятся параллельно.

/// Поднимать при изменении формата ModEntry или логики парсинга —
/// старый кэш будет отброшен целиком.
const CACHE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Default)]
struct ScanCache {
    version: u32,
    /// Ключ — абсолютный путь папки мода.
    entries: HashMap<String, CachedMod>,
}

#[derive(Serialize, Deserialize, Clone)]
struct CachedMod {
    /// mtime About.xml в секундах с эпохи.
    mtime: u64,
    entry: ModEntry,
}

fn cache_file_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "rustrim", "RustRim")
        .map(|d| d.cache_dir().join("mod_scan_cache.json"))
}

fn load_cache() -> ScanCache {
    let Some(path) = cache_file_path() else { return ScanCache::default() };
    let Ok(data) = std::fs::read_to_string(&path) else { return ScanCache::default() };
    match serde_json::from_str::<ScanCache>(&data) {
        Ok(c) if c.version == CACHE_VERSION => c,
        _ => ScanCache::default(),
    }
}

fn save_cache(cache: &ScanCache) {
    let Some(path) = cache_file_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(data) = serde_json::to_string(cache) {
        let _ = std::fs::write(&path, data);
    }
}

fn about_xml_mtime(mod_dir: &Path) -> Option<u64> {
    let meta = std::fs::metadata(mod_dir.join("About").join("About.xml")).ok()?;
    let mtime = meta.modified().ok()?;
    mtime.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Определяет источник по имени папки: чисто числовое имя — Workshop ID.
fn source_for_folder(folder_name: &str) -> ModSource {
    if !folder_name.is_empty() && folder_name.chars().all(|c| c.is_ascii_digit()) {
        folder_name.parse::<u64>().map(ModSource::Workshop).unwrap_or(ModSource::Local)
    } else {
        ModSource::Local
    }
}

/// Полный разбор одной папки мода (About.xml + превью).
fn parse_mod_dir(mod_dir: &Path) -> Option<ModEntry> {
    let about_xml = mod_dir.join("About").join("About.xml");
    let folder_name = mod_dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();

    match parse_about_xml(&about_xml) {
        Ok(data) => Some(ModEntry {
            name: if data.name.is_empty() { folder_name.clone() } else { data.name },
            package_id: data.package_id.to_lowercase(),
            version: data.version,
            author: data.author,
            supported_versions: data.supported_versions,
            path: mod_dir.to_path_buf(),
            source: source_for_folder(&folder_name),
            dependencies:      lower_ids(data.dependencies),
            load_after:        lower_ids(data.load_after),
            load_before:       lower_ids(data.load_before),
            incompatible_with: lower_ids(data.incompatible_with),
            is_active: false,
            description: data.description,
            preview_path: find_preview_image(mod_dir),
        }),
        Err(e) => {
            tracing::warn!("Skipping {:?}: {}", about_xml, e);
            None
        }
    }
}

pub fn scan_local_mods(mods_dir: &Path) -> Vec<ModEntry> {
    let started = Instant::now();

    let entries = match std::fs::read_dir(mods_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Cannot read mods directory {:?}: {}", mods_dir, e);
            return Vec::new();
        }
    };

    // Папки модов в детерминированном порядке
    let mut dirs: Vec<PathBuf> = entries.flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("About").join("About.xml").exists())
        .collect();
    dirs.sort();

    let mut cache = load_cache();
    cache.version = CACHE_VERSION;

    // Разделяем на попадания в кэш и промахи
    let mut result_slots: Vec<Option<ModEntry>> = vec![None; dirs.len()];
    let mut misses: Vec<(usize, PathBuf, Option<u64>)> = Vec::new();
    let mut hits = 0usize;

    for (i, dir) in dirs.iter().enumerate() {
        let mtime = about_xml_mtime(dir);
        let key = dir.to_string_lossy().into_owned();
        match (mtime, cache.entries.get(&key)) {
            (Some(mt), Some(cached)) if cached.mtime == mt => {
                let mut entry = cached.entry.clone();
                // Превью могли добавить/удалить без правки About.xml
                if entry.preview_path.as_deref().map(|p| !p.exists()).unwrap_or(true) {
                    entry.preview_path = find_preview_image(dir);
                }
                entry.is_active = false;
                result_slots[i] = Some(entry);
                hits += 1;
            }
            (mt, _) => misses.push((i, dir.clone(), mt)),
        }
    }

    // Промахи парсим параллельно
    if !misses.is_empty() {
        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get()).unwrap_or(4)
            .min(16)
            .min(misses.len());
        let chunk_size = misses.len().div_ceil(n_threads);
        let parsed: Vec<(usize, Option<u64>, Option<ModEntry>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = misses
                .chunks(chunk_size)
                .map(|chunk| {
                    scope.spawn(move || {
                        chunk.iter()
                            .map(|(i, dir, mt)| (*i, *mt, parse_mod_dir(dir)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });

        for (i, mtime, entry) in parsed {
            if let Some(entry) = entry {
                if let Some(mt) = mtime {
                    cache.entries.insert(
                        entry.path.to_string_lossy().into_owned(),
                        CachedMod { mtime: mt, entry: entry.clone() },
                    );
                }
                result_slots[i] = Some(entry);
            }
        }
    }

    // Выбрасываем из кэша записи об удалённых модах и сохраняем
    cache.entries.retain(|key, _| Path::new(key).is_dir());
    save_cache(&cache);

    let result: Vec<ModEntry> = result_slots.into_iter().flatten().collect();
    tracing::info!(
        "Scanned {:?}: {} mods in {:.0?} ({} from cache, {} parsed)",
        mods_dir, result.len(), started.elapsed(), hits, result.len() - hits,
    );
    result
}

/// Сканирует папку `game_path/Data/` и возвращает Core + DLC как ModEntry.
/// Core всегда идёт первым, остальные — по алфавиту.
/// Папок мало (Core + несколько DLC), кэш не нужен.
pub fn scan_dlc_mods(game_path: &Path) -> Vec<ModEntry> {
    let data_dir = game_path.join("Data");
    let mut result = Vec::new();

    let entries = match std::fs::read_dir(&data_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Cannot read Data directory {:?}: {}", data_dir, e);
            return result;
        }
    };

    let mut folders: Vec<_> = entries.flatten()
        .filter(|e| e.path().is_dir())
        .collect();
    // Core первый, затем остальные по алфавиту
    folders.sort_by(|a, b| {
        let a_core = a.file_name() == "Core";
        let b_core = b.file_name() == "Core";
        match (a_core, b_core) {
            (true, false)  => std::cmp::Ordering::Less,
            (false, true)  => std::cmp::Ordering::Greater,
            _              => a.file_name().cmp(&b.file_name()),
        }
    });

    for entry in folders {
        let mod_dir = entry.path();
        let about_xml = mod_dir.join("About").join("About.xml");
        if !about_xml.exists() { continue; }

        let folder_name = mod_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let source = if folder_name == "Core" {
            ModSource::Core
        } else {
            ModSource::DLC(folder_name.clone())
        };

        match parse_about_xml(&about_xml) {
            Ok(data) => {
                result.push(ModEntry {
                    name: if data.name.is_empty() { folder_name } else { data.name },
                    package_id: data.package_id.to_lowercase(),
                    version: data.version,
                    author: data.author,
                    supported_versions: data.supported_versions,
                    path: mod_dir.clone(),
                    source,
                    dependencies:     lower_ids(data.dependencies),
                    load_after:       lower_ids(data.load_after),
                    load_before:      lower_ids(data.load_before),
                    incompatible_with: lower_ids(data.incompatible_with),
                    is_active: false,
                    description: data.description,
                    preview_path: find_preview_image(&mod_dir),
                });
            }
            Err(e) => {
                tracing::warn!("Skipping DLC {:?}: {}", about_xml, e);
            }
        }
    }

    result
}
