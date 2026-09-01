//! Каждый символ, который приложение пишет на экран, должен рисоваться.
//!
//! Регрессия, ради которой тест написан: в egui 0.35 не рисуется ни одно
//! эмодзи. Половина интерфейса была подписана ими, и на их месте были пустые
//! прямоугольники. Компилятор такое не ловит: строка корректна, символ
//! существует, просто его некому нарисовать.
//!
//! Тест берёт все не-ASCII символы из исходников и спрашивает у настоящего
//! набора шрифтов, есть ли для каждого глиф. Комментарии проверяются заодно:
//! одиночный символ в комментарии обычно и есть тот, который потом попадёт
//! в интерфейс.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui::epaint::text::{Fonts, TextOptions};
use egui::{FontFamily, FontId};

#[test]
fn every_character_in_the_sources_has_a_glyph() {
    let mut fonts = Fonts::new(TextOptions::default(), rust_rim::ui::fonts::definitions());

    let mut missing: BTreeMap<char, Vec<String>> = BTreeMap::new();
    for file in rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src")) {
        let text = std::fs::read_to_string(&file).expect("исходник читается");
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_owned();

        for (line_no, line) in text.lines().enumerate() {
            for ch in line.chars() {
                if ch.is_ascii() {
                    continue;
                }
                let drawn = [FontFamily::Proportional, FontFamily::Monospace]
                    .into_iter()
                    .all(|family| fonts.has_glyph(&FontId::new(12.0, family), ch));
                if !drawn {
                    missing
                        .entry(ch)
                        .or_default()
                        .push(format!("{name}:{}", line_no + 1));
                }
            }
        }
    }

    if !missing.is_empty() {
        let report: Vec<String> = missing
            .iter()
            .map(|(ch, places)| {
                format!(
                    "  {ch} (U+{:04X}) — {}, всего мест: {}",
                    *ch as u32,
                    places.first().map(String::as_str).unwrap_or("?"),
                    places.len(),
                )
            })
            .collect();
        panic!(
            "символы без глифа — на экране будут пустые прямоугольники:\n{}",
            report.join("\n"),
        );
    }
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}
