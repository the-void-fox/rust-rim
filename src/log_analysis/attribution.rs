use std::collections::HashMap;
use std::path::Path;

use crate::mod_data::{ModDb, ModEntry, ModId, Profile};
use super::{is_vanilla_method, LogIssue, Suspect, VANILLA};
use super::parser::method_path;
use crate::trf;

// Веса сигналов. Главное здесь — позиция кадра: исключение брошено в
// верхнем кадре, и мод, который его переписал или в нём исполняется, почти
// всегда и есть причина. Патчи на корнях стека (`Verse.Root.OnGUI`) стоят у
// каждой ошибки интерфейса и уликой не считаются.
const SCORE_PATCH_TOP_TRANSPILER: i32 = 8;
const SCORE_PATCH_TOP: i32 = 5;
const SCORE_PATCH_NEAR: i32 = 2;
/// Патчи глубже этого кадра не считаются.
const PATCH_DEPTH: usize = 3;
const SCORE_TOP_MOD_FRAME: i32 = 6;
const SCORE_MOD_FRAME: i32 = 2;
const SCORE_FAILED_PATCH: i32 = 6;
const SCORE_PATH:      i32 = 5;
const SCORE_TYPE_IN_TEXT: i32 = 4;
const SCORE_PACKAGE:   i32 = 3;
const SCORE_NAME:      i32 = 1;

const MAX_SUSPECTS: usize = 5;
const MAX_EVIDENCE: usize = 4;
/// Сканируем на упоминания packageId/имён только первые строки записи
/// (стек покрыт отдельно), чтобы не гонять 900 contains по всему тексту.
const MAX_SCAN_LINES: usize = 20;
/// Строк сообщения, где ищутся имена типов: заголовок и текст исключения.
const MESSAGE_LINES: usize = 3;

#[derive(Clone)]
struct ModRef {
    package_id: ModId,
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
    /// lowercase имя мода → индекс: для «[Имя мода] Patch operation … failed»
    exact_names: HashMap<String, usize>,
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
    pub fn build(db: &ModDb, profile: &Profile) -> Self {
        let parts: Vec<(&ModEntry, Vec<String>)> = db.iter()
            .map(|m| (m, collect_dll_stems(&m.path)))
            .collect();
        Self::build_with_dlls(&parts, profile)
    }

    /// Строит индекс из готовых списков DLL (для тестов и фонового потока).
    pub fn build_with_dlls(parts: &[(&ModEntry, Vec<String>)], profile: &Profile) -> Self {
        let mut refs = Vec::with_capacity(parts.len());
        let mut dll: HashMap<String, usize> = HashMap::new();
        let mut folder: HashMap<String, usize> = HashMap::new();
        let mut packages = Vec::new();
        let mut names = Vec::new();
        let mut exact_names = HashMap::new();

        for (i, (m, dlls)) in parts.iter().enumerate() {
            let is_active = profile.is_active(&m.package_id);
            refs.push(ModRef {
                package_id: m.package_id.clone(),
                name: m.name.clone(),
                is_active,
            });
            if let Some(f) = m.path.file_name().and_then(|s| s.to_str()) {
                folder.insert(f.to_lowercase(), i);
            }
            if m.package_id.as_str().len() >= 6 {
                packages.push((m.package_id.as_str().to_string(), i));
            }
            if m.name.len() >= 8 {
                names.push((m.name.clone(), i));
            }
            if !m.name.trim().is_empty() {
                exact_names.entry(m.name.trim().to_lowercase()).or_insert(i);
            }
            for stem in dlls {
                let key = stem.to_lowercase();
                if VANILLA.contains(&key.as_str()) {
                    continue;
                }
                // Одна и та же DLL бывает у нескольких модов (общие
                // библиотеки, старые копии). Активный мод важнее: в игре
                // загружен именно он, а выключенный подозревать бессмысленно.
                match dll.get(&key) {
                    Some(&prev) if refs[prev].is_active || !is_active => {}
                    _ => {
                        dll.insert(key, i);
                    }
                }
            }
        }

        Self { refs, dll, folder, packages, names, exact_names }
    }

    /// Мод, чья DLL названа как корень неймспейса: `CameraPlus.Tools.X` →
    /// CameraPlus.dll. Сначала двухсегментный (`VFE.Core`), потом корень.
    fn by_namespace(&self, path: &str) -> Option<usize> {
        if is_vanilla_method(path) {
            return None;
        }
        let (root, two) = namespace_roots(path);
        self.dll.get(&two).or_else(|| self.dll.get(&root)).copied()
    }

    /// Мод по Harmony-id: совпадение с packageId, иначе как неймспейс.
    fn by_owner(&self, owner: &str) -> Option<usize> {
        let lower = owner.to_lowercase();
        self.packages
            .iter()
            .find(|(p, _)| p.eq_ignore_ascii_case(&lower))
            .map(|(_, i)| *i)
            .or_else(|| self.by_namespace(owner))
    }
}

/// Корневые сегменты пути метода: ("Root", "Root.Second").
fn namespace_roots(path: &str) -> (String, String) {
    let mut segs = path.split(['.', ':', '+']);
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

/// Имя мода из `[Имя мода] Patch operation … failed`.
fn failed_patch_owner(title: &str) -> Option<&str> {
    let rest = title.strip_prefix('[')?;
    let (name, _) = rest.rsplit_once("] Patch operation")?;
    Some(name.trim())
}

/// Короткая форма метода для улик.
fn short(method: &str) -> String {
    method.chars().take(80).collect()
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

    // 1. Кадры стека: код мода и Harmony-патчи на ванильных методах.
    let mut seen_mod_frame: Vec<usize> = Vec::new();
    for (pos, frame) in issue.frames.iter().enumerate() {
        if let Some(i) = index.by_namespace(&frame.method) {
            let first = seen_mod_frame.is_empty();
            if !seen_mod_frame.contains(&i) {
                let score = if first { SCORE_TOP_MOD_FRAME } else { SCORE_MOD_FRAME };
                add(i, score, trf!("стек: {short}", short = short(&frame.method)));
                seen_mod_frame.push(i);
            }
        }

        if pos >= PATCH_DEPTH {
            continue;
        }
        // Один мод на одном кадре считается один раз — по самому сильному
        // патчу: PREFIX+POSTFIX+FINALIZER одного мода не втрое весомее.
        let mut best: HashMap<usize, (i32, &str)> = HashMap::new();
        for patch in &frame.patches {
            let Some(i) = index.by_namespace(&patch.class).or_else(|| index.by_owner(&patch.owner)) else {
                continue;
            };
            let score = match (pos, patch.kind.as_str()) {
                (0, "TRANSPILER") => SCORE_PATCH_TOP_TRANSPILER,
                (0, _) => SCORE_PATCH_TOP,
                _ => SCORE_PATCH_NEAR,
            };
            let slot = best.entry(i).or_insert((0, ""));
            if score > slot.0 {
                *slot = (score, patch.kind.as_str());
            }
        }
        for (i, (score, kind)) in best {
            let frame_no = pos + 1;
            add(i, score, trf!("{kind} на кадре {frame_no}: {method}", kind, frame_no, method = short(&frame.method)));
        }
    }

    // 2. «[Имя мода] Patch operation … failed» — мод назвал себя сам.
    if let Some(name) = failed_patch_owner(&issue.title) {
        if let Some(&i) = index.exact_names.get(&name.to_lowercase()) {
            add(i, SCORE_FAILED_PATCH, trf!("патч мода «{name}» не применился", name));
        }
    }

    // 3. Типы из DLL мода, названные в самом сообщении:
    //    «Could not load type of field 'IsoRim.AlienFigures:local'…».
    let message = issue
        .full_text
        .lines()
        .filter(|l| !l.starts_with([' ', '\t']) && !l.starts_with("[Ref "))
        .take(MESSAGE_LINES);
    for line in message {
        for token in line.split(|c: char| !(c.is_alphanumeric() || c == '.' || c == '_')) {
            let token = token.trim_matches('.');
            if token.len() < 5 || !token.contains('.') {
                continue;
            }
            if let Some(i) = index.by_namespace(token) {
                let name = short(token);
                add(i, SCORE_TYPE_IN_TEXT, trf!("тип мода в тексте: {name}", name));
            }
        }
    }

    // 4-6. Текстовые сигналы: пути, packageId, названия
    for line in issue.full_text.lines().take(MAX_SCAN_LINES) {
        let lower = line.to_lowercase();

        for marker in ["mods/", "mods\\", "294100/", "294100\\"] {
            if let Some(seg) = path_segment_after(&lower, marker) {
                if let Some(&i) = index.folder.get(seg) {
                    add(i, SCORE_PATH, trf!("путь: …{marker}{seg}/…", marker, seg));
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
                add(*i, SCORE_NAME, trf!("упомянуто название «{name}»", name));
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
    // Активные раньше выключенных: в игре был загружен именно активный.
    suspects.sort_by(|a, b| {
        b.is_active.cmp(&a.is_active).then(b.score.cmp(&a.score)).then(a.name.cmp(&b.name))
    });
    suspects.truncate(MAX_SUSPECTS);
    issue.suspects = suspects;

    // След Harmony без явного виновника: пропатченный метод в кадре
    if issue.suspects.is_empty() {
        issue.harmony_hint = issue.frames.iter().find_map(|f| {
            let path = method_path(&f.method);
            let has_patch = path.rsplit(['.', ':']).next()
                .is_some_and(|m| m.contains("_Patch"));
            (has_patch || f.patched).then(|| {
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
