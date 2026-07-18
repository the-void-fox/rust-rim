use std::collections::HashMap;

use super::{LogIssue, Severity};

/// Максимум строк в одной записи (защита от «бесконечных» XML-дампов).
const MAX_ENTRY_LINES: usize = 80;

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
fn looks_like_exception_line(line: &str) -> bool {
    let Some(first) = line.split_whitespace().next() else { return false };
    first.contains("Exception") && line.contains(':')
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

/// Разбирает текст лога на сгруппированные записи.
pub fn parse_log(text: &str) -> Vec<LogIssue> {
    let lines: Vec<&str> = text.lines().collect();
    let mut issues: Vec<LogIssue> = Vec::new();
    let mut by_signature: HashMap<String, usize> = HashMap::new();

    let mut i = 0;
    while i < lines.len() {
        let Some(severity) = classify_start(lines[i]) else {
            i += 1;
            continue;
        };

        // Собираем запись: заголовок + продолжения
        let title = lines[i].trim_end().to_string();
        let mut entry_lines: Vec<String> = vec![title.clone()];
        let mut frames: Vec<String> = Vec::new();
        let mut j = i + 1;

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
