use std::collections::HashMap;

use super::{LogIssue, Severity};

/// Максимум строк в одной записи (защита от «бесконечных» XML-дампов).
const MAX_ENTRY_LINES: usize = 80;

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
        || lower.starts_with("loader exceptions");

    if is_error {
        // «0 errors» и подобная статистика — не ошибка
        if lower.contains("0 errors") || lower.contains("no errors") {
            return None;
        }
        return Some(Severity::Error);
    }
    None
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
        || looks_like_exception_line(line)
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

/// Собирает запись целиком: заголовок плюс строки-продолжения.
/// Возвращает строки записи, кадры стека и индекс следующей строки лога.
fn collect_entry(lines: &[&str], start: usize) -> (Vec<String>, Vec<String>, usize) {
    let mut entry_lines: Vec<String> = vec![lines[start].trim_end().to_string()];
    let mut frames: Vec<String> = Vec::new();
    let mut j = start + 1;

    while j < lines.len() && entry_lines.len() < MAX_ENTRY_LINES {
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
        if let Some(frame) = trimmed.strip_prefix("at ") {
            frames.push(frame.trim_end().to_string());
        } else if trimmed.starts_with("(wrapper") {
            frames.push(trimmed.trim_end().to_string());
        }
        entry_lines.push(line.trim_end().to_string());
        j += 1;
    }

    (entry_lines, frames, j)
}

/// Разбирает текст лога на сгруппированные записи.
pub fn parse_log(text: &str) -> Vec<LogIssue> {
    let lines: Vec<&str> = text.lines().collect();
    let mut issues: Vec<LogIssue> = Vec::new();
    let mut by_signature: HashMap<String, usize> = HashMap::new();

    let mut i = 0;
    while i < lines.len() {
        // Шум пропускается записью целиком, вместе со стеком: иначе строка
        // вида «System.ArgumentNullException: …» из его трассы всплыла бы
        // отдельной ошибкой уже без узнаваемого заголовка.
        if is_noise(lines[i]) {
            let (_, _, next) = collect_entry(&lines, i);
            i = next.max(i + 1);
            continue;
        }

        let Some(severity) = classify_start(lines[i]) else {
            i += 1;
            continue;
        };

        let (entry_lines, frames, j) = collect_entry(&lines, i);
        let title = entry_lines[0].clone();

        // Сигнатура: нормализованный заголовок + первый кадр стека
        let mut signature = normalize(&title);
        if let Some(f) = frames.first() {
            signature.push('\n');
            signature.push_str(&normalize(f));
        }

        match by_signature.get(&signature) {
            Some(&idx) => issues[idx].count += 1,
            None => {
                by_signature.insert(signature, issues.len());
                issues.push(LogIssue {
                    severity,
                    title,
                    full_text: entry_lines.join("\n"),
                    count: 1,
                    frames,
                    suspects: Vec::new(),
                    harmony_hint: None,
                });
            }
        }

        i = j.max(i + 1);
    }

    issues
}
