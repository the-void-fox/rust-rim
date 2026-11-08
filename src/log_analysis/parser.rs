use std::collections::HashMap;

use super::{is_vanilla_method, Frame, LogIssue, Patch, Severity};

/// Максимум строк в одной записи (защита от «бесконечных» XML-дампов).
/// Стек с подписями Harmony длинный: под каждым кадром — строки патчей.
const MAX_ENTRY_LINES: usize = 160;

/// Строки, которые формально выглядят ошибкой, но встречаются в каждом
/// запуске и к модам отношения не имеют.
///
/// «Fallback handler could not load library …» Mono печатает десятками при
/// каждом старте — без этого фильтра любой прогон выглядел бы сбойным.
const BENIGN: &[&str] = &[
    "fallback handler could not load library",
    "could not load signature of",
];

/// Шум перевода. Он есть в каждом запуске с непустым языком и почти ни на что
/// не влияет: игра подставляет английский текст и идёт дальше.
///
/// Первые три формы найдены в логах живой установки, остальные добавлены по
/// той же части игры — если не встретятся, вреда от них нет.
///
/// Обратная сторона: мод, который ломает именно перевод, теперь не попадёт
/// в отчёт. Это осознанный размен — иначе каждый прогон приходил с ошибкой,
/// которую всё равно все игнорируют.
const LOCALIZATION_NOISE: &[&str] = &[
    "failed to resolve text",
    "translation data for language",
    "grammar unresolvable",
    "could not find translation key",
    "duplicate translation key",
    "translation error",
];

/// Хвост строки, которым мод Harmony заменяет повторный стек.
const DUPLICATE_STACK: &str = "Duplicate stacktrace";

/// Шум ли это — запись, которую в отчёт пускать не надо.
fn is_noise(line: &str) -> bool {
    let lower = line.to_lowercase();
    BENIGN.iter().chain(LOCALIZATION_NOISE).any(|n| lower.starts_with(n))
}

/// Определяет, начинается ли с этой строки запись об ошибке/предупреждении.
/// Записи в Player.log не индентированы; кадры стека — индентированы.
fn classify_start(line: &str) -> Option<Severity> {
    if line.is_empty() || line.starts_with(' ') || line.starts_with('\t') {
        return None;
    }
    let lower = line.to_lowercase();

    if lower.starts_with("warning") || lower.starts_with("[warning]") {
        return Some(Severity::Warning);
    }

    let is_error = lower.contains("exception")
        || lower.starts_with("error")
        || lower.contains("error:")
        || lower.starts_with("failed")
        || lower.starts_with("could not")
        || lower.starts_with("couldn't")
        || lower.starts_with("unable to")
        || lower.starts_with("xml error")
        || lower.starts_with("mod errors")
        || lower.starts_with("loader exceptions")
        // Ошибка дефа: «Config error in Corpse_Autocleaner: Nutrition == 0 …».
        || lower.starts_with("config error")
        // «[Имя мода] Patch operation Verse.PatchOperationReplace(…) failed»:
        // слово «failed» в конце строки, а не в начале.
        || is_failed_patch(&lower);

    if is_error {
        // «0 errors» и подобная статистика — не ошибка
        if lower.contains("0 errors") || lower.contains("no errors") {
            return None;
        }
        return Some(Severity::Error);
    }
    None
}

/// `[Имя мода] Patch operation … failed` — патч XML не нашёл, что менять.
///
/// Именно «operation» в единственном числе: XML Extensions пишет сводку
/// «563 total patch operations run in 1232ms, 0 failed», и это не ошибка.
fn is_failed_patch(lower: &str) -> bool {
    let line = lower.trim_end();
    lower.contains("patch operation ") && line.ends_with(" failed") && !line.ends_with(" 0 failed")
}

/// Неиндентированная строка вида "System.NullReferenceException: …" —
/// RimWorld часто пишет сообщение и текст исключения двумя строками подряд.
///
/// Тип исключения — одно слово вплотную к двоеточию. Проверка именно такая,
/// потому что «Exception filling window for RimWorld.MainTabWindow_Inspect: …»
/// тоже начинается со слова Exception, но это самостоятельная запись, и
/// прежняя проверка приклеивала её к предыдущей.
fn looks_like_exception_line(line: &str) -> bool {
    let Some(head) = line.split(':').next() else { return false };
    !head.contains(char::is_whitespace) && head.ends_with("Exception") && line.contains(':')
}

/// Кадр в формате Unity, без отступа: `UnityEngine.Debug:LogException(Exception)`.
///
/// Такие идут хвостом за исключением из Unity-колбэка; без этой проверки
/// каждый кадр всплывал отдельной «ошибкой».
fn looks_like_unity_frame(line: &str) -> bool {
    let Some(paren) = line.find('(') else { return false };
    let head = line[..paren].trim_end();
    !head.is_empty()
        && head.contains(':')
        && !head.ends_with(':')
        && head.chars().all(|c| c.is_alphanumeric() || "._:<>`+[],$".contains(c))
        && line.trim_end().ends_with(')')
}

/// Строка-продолжение записи (стек, вложенные исключения, XML-контекст).
fn is_continuation(line: &str) -> bool {
    line.starts_with(' ')
        || line.starts_with('\t')
        || line.starts_with("at ")
        || line.starts_with("--- ")
        || line.starts_with("--->")
        || line.starts_with("=>")
        || line.starts_with("Rethrow as")
        || line.starts_with("Parameter name")
        || line.starts_with("(wrapper")
        || line.starts_with("[Ref ")
        // Вторая строка неудавшегося патча: `file: …\Mods\<id>\Patches\x.xml`.
        || line.starts_with("file: ")
        || looks_like_exception_line(line)
        || looks_like_unity_frame(line)
}

/// Нормализация для группировки: числа → '#', обрезка по длине.
fn normalize(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect();
    out.truncate(160);
    out
}

/// «Путь метода» из кадра стека:
/// `Ns.Class.Method (args) [0x...]` → `Ns.Class.Method`;
/// `(wrapper dynamic-method) Verse.Pawn.SpawnSetup_Patch2(...)` → часть после wrapper.
pub(super) fn method_path(frame: &str) -> &str {
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

/// Строка патча под кадром:
/// `    - POSTFIX owner.id: Void Ns.Class:Method(args)`.
fn parse_patch(line: &str) -> Option<Patch> {
    let rest = line.trim_start().strip_prefix("- ")?;
    let (kind, rest) = rest.split_once(' ')?;
    if !matches!(kind, "PREFIX" | "POSTFIX" | "TRANSPILER" | "FINALIZER" | "ILMANIPULATOR") {
        return None;
    }
    let (owner, signature) = rest.split_once(": ")?;
    // Сначала тип возвращаемого значения, потом `Ns.Class:Method(...)`.
    let target = signature.split_once(' ').map_or(signature, |(_, t)| t);
    let class = target.split_once(':').map_or(target, |(c, _)| c);
    Some(Patch {
        kind: kind.to_string(),
        owner: owner.trim().to_string(),
        class: class.trim().to_string(),
    })
}

/// Ссылка `[Ref 3B3B4C0]`: номер и признак дубликата.
fn parse_ref(line: &str) -> Option<(String, bool)> {
    let rest = line.trim_start().strip_prefix("[Ref ")?;
    let id: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    (!id.is_empty()).then(|| (id, rest.contains(DUPLICATE_STACK)))
}

/// Тип исключения: первое слово вида `Ns.SomethingException` в начале записи.
fn exception_type(lines: &[String]) -> Option<String> {
    lines.iter().take(3).find_map(|line| {
        line.split(|c: char| c.is_whitespace() || matches!(c, ':' | '(' | ')' | '\'' | '"'))
            .find(|tok| {
                tok.len() > "Exception".len()
                    && tok.ends_with("Exception")
                    && tok.starts_with(|c: char| c.is_ascii_alphabetic())
            })
            .map(str::to_string)
    })
}

/// Одна запись лога, как она есть в тексте.
struct Entry {
    lines: Vec<String>,
    frames: Vec<Frame>,
    /// `[Ref …]` и признак «стек напечатан раньше».
    reference: Option<(String, bool)>,
}

/// Собирает запись целиком: заголовок плюс строки-продолжения.
/// Возвращает запись и индекс следующей строки лога.
fn collect_entry(lines: &[&str], start: usize) -> (Entry, usize) {
    let mut entry = Entry {
        lines: vec![lines[start].trim_end().to_string()],
        frames: Vec::new(),
        reference: None,
    };
    let mut j = start + 1;

    while j < lines.len() && entry.lines.len() < MAX_ENTRY_LINES {
        let line = lines[j];

        // Терминатор Unity-записи
        if line.trim_start().starts_with("(Filename:") {
            j += 1;
            break;
        }
        // Пустая строка заканчивает запись, если дальше не продолжение
        if line.trim().is_empty() {
            if j + 1 < lines.len() && is_continuation(lines[j + 1]) {
                j += 1;
                continue;
            }
            break;
        }
        if !is_continuation(line) {
            break;
        }

        let trimmed = line.trim_start();
        if let Some(reference) = parse_ref(trimmed) {
            // Вложенные исключения дают по своему Ref; группируем по первому.
            entry.reference.get_or_insert(reference);
        } else if let Some(patch) = parse_patch(line) {
            if let Some(frame) = entry.frames.last_mut() {
                frame.patches.push(patch);
            }
        } else if let Some(frame) = trimmed.strip_prefix("at ") {
            entry.frames.push(Frame {
                method: method_path(frame).to_string(),
                // Harmony печатает адрес вместо IL-смещения у подменённых методов.
                patched: frame.contains(" <0x"),
                patches: Vec::new(),
            });
        } else if trimmed.starts_with("(wrapper") {
            entry.frames.push(Frame {
                method: method_path(trimmed).to_string(),
                patched: true,
                patches: Vec::new(),
            });
        }
        entry.lines.push(line.trim_end().to_string());
        j += 1;
    }

    (entry, j)
}

/// Причина ошибки — то, что совпадает между прогонами одной и той же сборки.
///
/// Заголовок для этого не годится: проверено на живых прогонах, что в двух
/// запусках одной сборки было 153 и 57 разных заголовков
/// `Exception ticking Elk20919 (at (168, 0, 56))`, и ни одного общего. Не
/// годится и стек целиком: одна и та же поломка роняет то один ванильный
/// метод, то другой. Стабильна улика — первый кадр, к которому причастен
/// мод: пропатченный им метод (по владельцам патчей, а если есть
/// транспайлер — по его владельцу) или метод из кода мода.
pub(super) fn cause_key(exception: Option<&str>, frames: &[Frame], title: &str) -> String {
    let exc = exception.unwrap_or("");
    let anchor = frames
        .iter()
        .find(|f| !f.patches.is_empty() || !is_vanilla_method(&f.method));
    match anchor {
        Some(frame) if !frame.patches.is_empty() => {
            let transpilers: Vec<&str> = frame
                .patches
                .iter()
                .filter(|p| p.kind == "TRANSPILER")
                .map(|p| p.owner.as_str())
                .collect();
            let mut owners: Vec<&str> = if transpilers.is_empty() {
                frame.patches.iter().map(|p| p.owner.as_str()).collect()
            } else {
                transpilers
            };
            owners.sort_unstable();
            owners.dedup();
            format!("{exc}|patched:{}", owners.join(","))
        }
        Some(frame) => format!("{exc}|at:{}", frame.method),
        None => match frames.first() {
            Some(top) => format!("{exc}|at:{}", top.method),
            // Без стека — заголовок без всего, где есть цифры: номера вещей,
            // координаты, ID работ.
            None => {
                let words: Vec<&str> = title
                    .split_whitespace()
                    .filter(|w| !w.chars().any(|c| c.is_ascii_digit()))
                    .collect();
                let text: String = words.join(" ").chars().take(160).collect();
                format!("{exc}|msg:{text}")
            }
        },
    }
}

/// Разбирает текст лога на сгруппированные записи.
pub fn parse_log(text: &str) -> Vec<LogIssue> {
    let lines: Vec<&str> = text.lines().collect();
    let mut issues: Vec<LogIssue> = Vec::new();
    let mut by_signature: HashMap<String, usize> = HashMap::new();
    // Ref → стек оригинала: дубликаты получают его отсюда.
    let mut originals: HashMap<String, Vec<Frame>> = HashMap::new();
    // Записи, чей стек ещё надо взять у оригинала: (запись, Ref).
    let mut borrowed: Vec<(usize, String)> = Vec::new();

    let mut i = 0;
    while i < lines.len() {
        // Шум пропускается записью целиком, вместе со стеком: иначе строка
        // вида «System.ArgumentNullException: …» из его трассы всплыла бы
        // отдельной ошибкой уже без узнаваемого заголовка.
        if is_noise(lines[i]) {
            let (_, next) = collect_entry(&lines, i);
            i = next.max(i + 1);
            continue;
        }

        let Some(severity) = classify_start(lines[i]) else {
            i += 1;
            continue;
        };

        let (entry, j) = collect_entry(&lines, i);
        i = j.max(i + 1);

        if let Some((id, false)) = &entry.reference {
            if !entry.frames.is_empty() {
                originals.entry(id.clone()).or_insert_with(|| entry.frames.clone());
            }
        }

        // Один Ref — один и тот же стек, сколько бы ни менялся заголовок:
        // «Exception ticking Elk…» и «Exception ticking Josh…» — одна ошибка.
        let title = entry.lines[0].clone();
        let signature = match &entry.reference {
            Some((id, _)) => format!("ref:{id}"),
            None => {
                let mut s = normalize(&title);
                if let Some(f) = entry.frames.first() {
                    s.push('\n');
                    s.push_str(&normalize(&f.method));
                }
                s
            }
        };

        match by_signature.get(&signature) {
            Some(&idx) => issues[idx].count += 1,
            None => {
                by_signature.insert(signature, issues.len());
                if entry.frames.is_empty() {
                    if let Some((id, true)) = &entry.reference {
                        borrowed.push((issues.len(), id.clone()));
                    }
                }
                issues.push(LogIssue {
                    severity,
                    exception: exception_type(&entry.lines),
                    cause: String::new(),
                    title,
                    full_text: entry.lines.join("\n"),
                    count: 1,
                    frames: entry.frames,
                    suspects: Vec::new(),
                    harmony_hint: None,
                });
            }
        }
    }

    // Оригинал мог напечататься позже дубликата: игра пишет стек при первом
    // вычислении, а оно не всегда попадает в лог.
    for (idx, id) in borrowed {
        if let Some(frames) = originals.get(&id) {
            issues[idx].frames = frames.clone();
        }
    }
    for issue in &mut issues {
        issue.cause = cause_key(issue.exception.as_deref(), &issue.frames, &issue.title);
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harmony_patch_lines_are_parsed() {
        let p = parse_patch(
            "    - TRANSPILER PerformanceOptimizer.Main: IEnumerable`1 PerformanceOptimizer.Optimization_FasterGetCompReplacement:Transpiler(IEnumerable`1 instructions, MethodBase method)",
        )
        .unwrap();
        assert_eq!(p.kind, "TRANSPILER");
        assert_eq!(p.owner, "PerformanceOptimizer.Main");
        assert_eq!(p.class, "PerformanceOptimizer.Optimization_FasterGetCompReplacement");

        let p = parse_patch("    - POSTFIX net.pardeike.rimworld.mod.camera+: Void CameraPlus.Root_OnGUI_Patch:Postfix()").unwrap();
        assert_eq!(p.owner, "net.pardeike.rimworld.mod.camera+");
        assert_eq!(p.class, "CameraPlus.Root_OnGUI_Patch");

        assert!(parse_patch("  - something else: x").is_none());
    }

    #[test]
    fn patch_summary_with_zero_failures_is_not_an_error() {
        assert_eq!(classify_start("[XML Extensions] 563 total patch operations run in 1232ms, 0 failed"), None);
        assert_eq!(
            classify_start("[Mod] Patch operation Verse.PatchOperationRemove(/Defs/X) failed"),
            Some(Severity::Error),
        );
    }

    #[test]
    fn unity_frames_are_continuations_not_errors() {
        assert!(looks_like_unity_frame("UnityEngine.Debug:LogException(Exception)"));
        assert!(looks_like_unity_frame("UnityEngine.DebugLogHandler:Internal_LogException(Exception, Object)"));
        assert!(!looks_like_unity_frame("Error: could not do (something)"));
        assert!(!looks_like_unity_frame("Config error in Foo: bar (baz)"));
    }

    #[test]
    fn cause_ignores_pawn_names_and_coordinates() {
        let titles = [
            "Exception ticking Elk20919 (at (168, 0, 56)): System.NullReferenceException: Object reference not set",
            "Exception ticking Josh (at (161, 0, 105)): System.NullReferenceException: Object reference not set",
        ];
        let frame = |method: &str| Frame {
            method: method.into(),
            patched: true,
            patches: vec![Patch {
                kind: "TRANSPILER".into(),
                owner: "PerformanceOptimizer.Main".into(),
                class: "PerformanceOptimizer.X".into(),
            }],
        };
        // Одна и та же поломка в двух прогонах роняет разные методы.
        let a = cause_key(Some("System.NullReferenceException"), &[frame("Verse.PawnStatusEffecters.EffectersTick")], titles[0]);
        let b = cause_key(Some("System.NullReferenceException"), &[frame("RimWorld.InvisibilityUtility.IsHiddenFromPlayer")], titles[1]);
        assert_eq!(a, b);
        assert_eq!(a, "System.NullReferenceException|patched:PerformanceOptimizer.Main");
    }

    #[test]
    fn transpiler_owner_wins_over_other_patches_on_the_frame() {
        let frame = Frame {
            method: "RimWorld.GlobalControls.GlobalControlsOnGUI".into(),
            patched: true,
            patches: vec![
                Patch { kind: "TRANSPILER".into(), owner: "PerformanceOptimizer.Main".into(), class: "P.X".into() },
                Patch { kind: "PREFIX".into(), owner: "net.pardeike.rimworld.mod.camera+".into(), class: "CameraPlus.Y".into() },
            ],
        };
        assert_eq!(
            cause_key(Some("System.NullReferenceException"), &[frame], "Root level exception in OnGUI()"),
            "System.NullReferenceException|patched:PerformanceOptimizer.Main",
        );
    }

    #[test]
    fn cause_of_mod_code_is_its_method() {
        let frames = [
            Frame { method: "Verse.Root.Update".into(), ..Default::default() },
            Frame { method: "CameraPlus.Tools.IsHiddenFromPlayer".into(), ..Default::default() },
        ];
        assert_eq!(cause_key(Some("System.NullReferenceException"), &frames, "x"), "System.NullReferenceException|at:CameraPlus.Tools.IsHiddenFromPlayer");
    }

    #[test]
    fn stackless_cause_drops_numbers() {
        assert_eq!(
            cause_key(None, &[], "Could not find Def named Steel42 at (1, 0, 3)"),
            "|msg:Could not find Def named at",
        );
    }
}
