use std::collections::HashMap;
use std::path::Path;

use crate::mod_data::ModEntry;
use super::{LogIssue, Suspect};

// Веса сигналов
const SCORE_PATH:      i32 = 5;
const SCORE_NAMESPACE: i32 = 4;
const SCORE_PACKAGE:   i32 = 3;
const SCORE_NAME:      i32 = 1;

const MAX_SUSPECTS: usize = 5;
const MAX_EVIDENCE: usize = 4;
/// Сканируем на упоминания packageId/имён только первые строки записи
/// (стек покрыт отдельно), чтобы не гонять 900 contains по всему тексту.
const MAX_SCAN_LINES: usize = 20;

/// Ванильные и системные неймспейсы — не считаются виновниками.
const VANILLA: &[&str] = &[
    "system", "verse", "rimworld", "unityengine", "unity", "harmonylib",
    "mono", "microsoft", "tmpro", "steamworks", "ludeontk", "mscorlib",
    "0harmony", "ionic", "newtonsoft", "runtimeaudioclip", "object",
];

#[derive(Clone)]
struct ModRef {
    package_id: String,
    name: String,
    is_active: bool,
}

pub struct ModIndex {
    refs: Vec<ModRef>,
    /// lowercase имя DLL (без .dll) → индекс мода
    dll: HashMap<String, usize>,
    /// lowercase имя папки мода → индекс мода
    folder: HashMap<String, usize>,
    /// (lowercase packageId, индекс) — для contains-поиска
    packages: Vec<(String, usize)>,
    /// (имя мода как в About.xml, индекс) — только достаточно длинные имена
    names: Vec<(String, usize)>,
}

/// Собирает имена DLL мода: Assemblies/*.dll в корне и в версионных
/// подпапках (1.4/, 1.5/, Common/, …) на один уровень вглубь.
fn collect_dll_stems(mod_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut scan = |dir: &Path| {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("dll")) {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        out.push(stem.to_string());
                    }
                }
            }
        }
    };
    scan(&mod_dir.join("Assemblies"));
    if let Ok(entries) = std::fs::read_dir(mod_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                scan(&p.join("Assemblies"));
            }
        }
    }
    out
}

impl ModIndex {
    /// Строит индекс по установленным модам (читает Assemblies/ с диска).
    pub fn build(mods: &[ModEntry]) -> Self {
        let parts: Vec<(&ModEntry, Vec<String>)> = mods.iter()
            .map(|m| (m, collect_dll_stems(&m.path)))
            .collect();
        Self::build_with_dlls(&parts)
    }

    /// Строит индекс из готовых списков DLL (для тестов и фонового потока).
    pub fn build_with_dlls(parts: &[(&ModEntry, Vec<String>)]) -> Self {
        let mut refs = Vec::with_capacity(parts.len());
        let mut dll: HashMap<String, usize> = HashMap::new();
        let mut folder: HashMap<String, usize> = HashMap::new();
        let mut packages = Vec::new();
        let mut names = Vec::new();

        for (i, (m, dlls)) in parts.iter().enumerate() {
            refs.push(ModRef {
                package_id: m.package_id.clone(),
                name: m.name.clone(),
                is_active: m.is_active,
            });
            if let Some(f) = m.path.file_name().and_then(|s| s.to_str()) {
                folder.insert(f.to_lowercase(), i);
            }
            if m.package_id.len() >= 6 {
                packages.push((m.package_id.to_lowercase(), i));
            }
            if m.name.len() >= 8 {
                names.push((m.name.clone(), i));
            }
            for stem in dlls {
                let key = stem.to_lowercase();
                if !VANILLA.contains(&key.as_str()) {
                    dll.entry(key).or_insert(i);
                }
            }
        }

        Self { refs, dll, folder, packages, names }
    }
}

/// Извлекает «путь метода» из кадра стека:
/// `Ns.Class.Method (args) [0x...]` → `Ns.Class.Method`
/// `(wrapper dynamic-method) Verse.Pawn.Verse.Pawn.SpawnSetup_Patch2(...)` → часть после wrapper.
fn method_path(frame: &str) -> &str {
    let mut s = frame;
    if let Some(rest) = s.strip_prefix("(wrapper dynamic-method)") {
        s = rest.trim_start();
    } else if let Some(rest) = s.strip_prefix("(wrapper") {
        // другие wrapper-формы: "(wrapper xxx) Ns.Class:Method (...)"
        s = rest.split_once(')').map(|(_, r)| r).unwrap_or(rest).trim_start();
    }
    let end = s.find([' ', '(']).unwrap_or(s.len());
    &s[..end]
}

/// Корневые сегменты пути метода: ("Root", "Root.Second").
fn namespace_roots(path: &str) -> (String, String) {
    let mut segs = path.split(['.', ':']);
    let root = segs.next().unwrap_or("").to_lowercase();
    let second = segs.next().unwrap_or("").to_lowercase();
    let two = if second.is_empty() { root.clone() } else { format!("{root}.{second}") };
    (root, two)
}

/// Ищет в строке сегмент пути после маркера ("mods/", "294100/"):
/// `.../Mods/MyMod/Textures/x.png` → "mymod".
fn path_segment_after<'a>(lower_line: &'a str, marker: &str) -> Option<&'a str> {
    let start = lower_line.find(marker)? + marker.len();
    let rest = &lower_line[start..];
    let end = rest.find(['/', '\\', '"', '\'', ' ']).unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Заполняет issue.suspects и issue.harmony_hint.
pub fn attribute(issue: &mut LogIssue, index: &ModIndex) {
    // индекс мода → (счёт, улики)
    let mut scores: HashMap<usize, (i32, Vec<String>)> = HashMap::new();
    let mut add = |mod_idx: usize, score: i32, evidence: String| {
        let e = scores.entry(mod_idx).or_default();
        e.0 += score;
        if e.1.len() < MAX_EVIDENCE && !e.1.contains(&evidence) {
            e.1.push(evidence);
        }
    };

    // 1. Кадры стека → неймспейс → DLL мода
    for frame in &issue.frames {
        let path = method_path(frame);
        let (root, two) = namespace_roots(path);
        if root.is_empty() || VANILLA.contains(&root.as_str()) {
            continue;
        }
        if let Some(&i) = index.dll.get(&two).or_else(|| index.dll.get(&root)) {
            let short: String = path.chars().take(80).collect();
            add(i, SCORE_NAMESPACE, format!("стек: {short}"));
        }
    }

    // 2-4. Текстовые сигналы: пути, packageId, названия
    for line in issue.full_text.lines().take(MAX_SCAN_LINES) {
        let lower = line.to_lowercase();

        for marker in ["mods/", "mods\\", "294100/", "294100\\"] {
            if let Some(seg) = path_segment_after(&lower, marker) {
                if let Some(&i) = index.folder.get(seg) {
                    add(i, SCORE_PATH, format!("путь: …{marker}{seg}/…"));
                }
            }
        }

        for (pkg, i) in &index.packages {
            if lower.contains(pkg.as_str()) {
                add(*i, SCORE_PACKAGE, format!("packageId: {pkg}"));
            }
        }
        for (name, i) in &index.names {
            if line.contains(name.as_str()) {
                add(*i, SCORE_NAME, format!("упомянуто название «{name}»"));
            }
        }
    }

    let mut suspects: Vec<Suspect> = scores.into_iter()
        .map(|(i, (score, evidence))| {
            let r = &index.refs[i];
            Suspect {
                package_id: r.package_id.clone(),
                name: r.name.clone(),
                is_active: r.is_active,
                score,
                evidence,
            }
        })
        .collect();
    suspects.sort_by(|a, b| b.score.cmp(&a.score).then(a.name.cmp(&b.name)));
    suspects.truncate(MAX_SUSPECTS);
    issue.suspects = suspects;

    // След Harmony без явного виновника: пропатченный метод в кадре
    if issue.suspects.is_empty() {
        issue.harmony_hint = issue.frames.iter().find_map(|f| {
            let path = method_path(f);
            let has_patch = path.rsplit(['.', ':']).next()
                .is_some_and(|m| m.contains("_Patch"));
            has_patch.then(|| {
                let clean = path.trim_end_matches(|c: char| c.is_ascii_digit())
                    .trim_end_matches("_Patch");
                undouble(clean)
            })
        });
    }
}

/// Mono именует wrapper-методы с задвоением: `Ns.Class.Ns.Class.Method`.
/// Возвращает путь без повторённого префикса.
fn undouble(path: &str) -> String {
    let segs: Vec<&str> = path.split('.').collect();
    let n = segs.len();
    if n >= 3 && n % 2 == 1 {
        let half = n / 2;
        if segs[..half] == segs[half..n - 1] {
            return segs[half..].join(".");
        }
    }
    path.to_string()
}
