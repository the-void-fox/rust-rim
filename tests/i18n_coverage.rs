//! Переводы полные и не разъезжаются с кодом.
//!
//! Тест читает исходники, находит каждый ключ перевода — строку внутри
//! `tr(...)`, `trf!(...)` или `k(...)` — и проверяет три вещи:
//!
//! 1. Русских строк вне перевода в интерфейсном коде нет. Иначе новая
//!    подпись так и останется русской во всех языках, и никто не заметит.
//! 2. В каждом `locales/*.json` есть перевод каждого ключа, и нет ключей,
//!    которых в коде уже нет.
//! 3. Подстановки `{имя}` в переводе те же, что в оригинале: потерянная
//!    подстановка — это пустое место вместо числа.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rust_rim::i18n::{Lang, source};

/// Строки, которые русскими и должны остаться: это не подписи интерфейса.
const NOT_UI: &[&str] = &[
    // Название языка в списке языков пишется на нём самом.
    "Русский",
    "Українська",
];

#[test]
fn every_ui_string_is_translated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut keys: BTreeSet<String> = BTreeSet::new();
    let mut bare: Vec<String> = Vec::new();

    for file in rust_files(&root.join("src")) {
        let rel = file.strip_prefix(root).unwrap().display().to_string();
        let text = std::fs::read_to_string(&file).unwrap();
        // Тесты модулей не переводятся.
        let text = match text.find("#[cfg(test)]") {
            Some(at) => &text[..at],
            None => &text[..],
        };
        for lit in literals(text) {
            let before = text[..lit.start].trim_end();
            let wrapped = is_key_call(before);
            if wrapped {
                keys.insert(lit.value.clone());
                continue;
            }
            if !has_cyrillic(&lit.value) || NOT_UI.contains(&lit.value.as_str()) {
                continue;
            }
            // Служебный ли это вызов, видно по началу выражения: многострочный
            // tracing::warn!( или assert!( открывается строками выше литерала.
            let stmt_start = text[..lit.start].rfind([';', '{', '}']).map_or(0, |i| i + 1);
            let line_end = text[lit.start..].find('\n').map_or(text.len(), |i| lit.start + i);
            let stmt = &text[stmt_start..line_end];
            let service = ["tracing::", "expect(", "panic!(", "assert", "unreachable!(", "#["]
                .iter()
                .any(|m| stmt.contains(m));
            if !service {
                let line_no = text[..lit.start].matches('\n').count() + 1;
                bare.push(format!("  {rel}:{line_no}: {:?}", lit.value));
            }
        }
    }

    assert!(
        bare.is_empty(),
        "русские строки без перевода — оберни их в tr()/trf!()/k():\n{}",
        bare.join("\n")
    );

    let mut problems: Vec<String> = Vec::new();
    for lang in Lang::ALL.into_iter().filter(|l| *l != Lang::Ru) {
        let table: BTreeMap<String, String> = serde_json::from_str(source(lang))
            .unwrap_or_else(|e| panic!("locales/{}.json не разбирается: {e}", lang.file()));
        for key in &keys {
            match table.get(key) {
                None => problems.push(format!("  {}: нет перевода {key:?}", lang.file())),
                Some(v) if v.trim().is_empty() => {
                    problems.push(format!("  {}: пустой перевод {key:?}", lang.file()))
                }
                Some(v) if placeholders(v) != placeholders(key) => problems.push(format!(
                    "  {}: подстановки {:?} ≠ {:?} в {key:?}",
                    lang.file(),
                    placeholders(v),
                    placeholders(key)
                )),
                Some(_) => {}
            }
        }
        for key in table.keys().filter(|k| !keys.contains(*k)) {
            problems.push(format!("  {}: ключа больше нет в коде {key:?}", lang.file()));
        }
    }
    assert!(problems.is_empty(), "переводы не сходятся с кодом:\n{}", problems.join("\n"));
}

/// Стоит ли литерал первым аргументом `tr(`, `trf!(` или `k(` — именно их,
/// а не `push_str(` или `check(`, которые тоже кончаются на «tr(» и «k(».
fn is_key_call(before: &str) -> bool {
    ["tr(", "trf!(", "k("].iter().any(|w| {
        before.strip_suffix(w).is_some_and(|head| {
            !head.ends_with(|c: char| c.is_alphanumeric() || c == '_')
        })
    })
}

fn has_cyrillic(s: &str) -> bool {
    s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
}

fn placeholders(s: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else { break };
        let name = &rest[..close];
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.insert(name.to_string());
        }
        rest = &rest[close + 1..];
    }
    out
}

struct Literal {
    /// Смещение открывающей кавычки (или `r`).
    start: usize,
    value: String,
}

/// Строковые литералы Rust с учётом комментариев, символов и raw-строк.
fn literals(src: &str) -> Vec<Literal> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            b'\'' => {
                // Символ: 'x', '\n', '\''. Иначе это время жизни.
                if b.get(i + 1) == Some(&b'\\') {
                    let mut j = i + 2;
                    while j < b.len() && b[j] != b'\'' {
                        j += 1;
                    }
                    i = j + 1;
                } else {
                    let ch_len = src[i + 1..].chars().next().map_or(1, char::len_utf8);
                    if b.get(i + 1 + ch_len) == Some(&b'\'') {
                        i += 2 + ch_len;
                    } else {
                        i += 1;
                    }
                }
            }
            b'r' if (b.get(i + 1) == Some(&b'"') || b.get(i + 1) == Some(&b'#'))
                && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) =>
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while b.get(j) == Some(&b'#') {
                    hashes += 1;
                    j += 1;
                }
                if b.get(j) != Some(&b'"') {
                    i += 1;
                    continue;
                }
                let close = format!("\"{}", "#".repeat(hashes));
                let body_start = j + 1;
                let end = src[body_start..].find(&close).map_or(b.len(), |e| body_start + e);
                out.push(Literal { start: i, value: src[body_start..end].to_string() });
                i = end + close.len();
            }
            b'"' => {
                let (value, end) = cooked(src, i + 1);
                out.push(Literal { start: i, value });
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

/// Разбирает обычный строковый литерал с экранированием. Возвращает
/// значение и позицию после закрывающей кавычки.
fn cooked(src: &str, mut i: usize) -> (String, usize) {
    let b = src.as_bytes();
    let mut out = String::new();
    while i < b.len() {
        match b[i] {
            b'"' => return (out, i + 1),
            b'\\' => {
                let next = b.get(i + 1).copied().unwrap_or(b'\\');
                i += 2;
                match next {
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    b'0' => out.push('\0'),
                    b'\\' => out.push('\\'),
                    b'"' => out.push('"'),
                    b'\'' => out.push('\''),
                    b'\n' => {
                        // Перенос строки внутри литерала: пробелы следующей
                        // строки в значение не входят.
                        while i < b.len() && (b[i] as char).is_whitespace() {
                            i += 1;
                        }
                    }
                    b'u' => {
                        let close = src[i..].find('}').map_or(i, |e| i + e);
                        let hex = src[i + 1..close].trim_matches(['{', '}']);
                        if let Some(c) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                            out.push(c);
                        }
                        i = close + 1;
                    }
                    other => out.push(other as char),
                }
            }
            _ => {
                let ch = src[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    (out, i)
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out.sort();
    out
}
