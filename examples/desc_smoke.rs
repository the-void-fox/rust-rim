// Прогон конвертера описаний по реальной папке модов:
//   cargo run --example desc_smoke -- <папка модов>
//
// Ищет разметку, которую конвертер не разобрал: если в результате остались
// «[tag]…[/tag]», значит такой тег ещё не поддержан.

use std::collections::HashMap;

use rust_rim::description::to_markdown;
use rust_rim::mod_data::scan_local_mods;

fn main() {
    let dir = std::env::args().nth(1).expect("usage: desc_smoke <mods_dir>");
    let mods = scan_local_mods(std::path::Path::new(&dir));

    let mut with_desc = 0usize;
    let mut with_bbcode = 0usize;
    let mut leftovers: HashMap<String, usize> = HashMap::new();
    let mut longest = (0usize, String::new());
    // Ширина, а не длина: неразрывный кусок без пробелов не переносится
    // и распирает панель деталей.
    let mut widest_token = (0usize, String::new(), String::new());
    let mut widest_line = (0usize, String::new());
    // Строки таблиц egui рисует без переноса — их ширина критична.
    let mut widest_table_row = (0usize, String::new());

    for m in &mods {
        if m.description.trim().is_empty() {
            continue;
        }
        with_desc += 1;
        if m.description.contains('[') {
            with_bbcode += 1;
        }

        let md = to_markdown(&m.description);
        if m.description.len() > longest.0 {
            longest = (m.description.len(), m.name.clone());
        }

        for tag in unhandled_tags(&md) {
            *leftovers.entry(tag).or_default() += 1;
        }

        for line in visible(&md).lines() {
            let chars = line.chars().count();
            if chars > widest_line.0 {
                widest_line = (chars, m.name.clone());
            }
            if line.trim_start().starts_with('|') && chars > widest_table_row.0 {
                widest_table_row = (chars, m.name.clone());
            }
            for token in line.split_whitespace() {
                let chars = token.chars().count();
                if chars > widest_token.0 {
                    widest_token = (chars, m.name.clone(), token.chars().take(70).collect());
                }
            }
        }
    }

    println!("Модов: {}, с описанием: {with_desc}, из них с '[': {with_bbcode}", mods.len());
    println!("Самое длинное описание: {} байт — {}", longest.0, longest.1);
    println!("Самая широкая строка: {} символов — {}", widest_line.0, widest_line.1);
    println!(
        "Самая широкая строка таблицы: {} символов — {}",
        widest_table_row.0, widest_table_row.1,
    );
    println!(
        "Самый длинный неразрывный кусок: {} символов — {}\n    {}",
        widest_token.0, widest_token.1, widest_token.2,
    );

    let mut top: Vec<_> = leftovers.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    if top.is_empty() {
        println!("Неразобранной разметки не осталось.");
    } else {
        println!("\nНеразобранные теги (тег — сколько описаний):");
        for (tag, n) in top.iter().take(25) {
            println!("  {n:5}  [{tag}]");
        }
    }
}

/// Убирает адреса ссылок: на экране виден только их текст, а сам URL
/// в ширину не вносит вклада.
fn visible(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        rest = &rest[open + 1..];
        let Some(close) = rest.find("](") else { out.push('['); continue };
        let text = &rest[..close];
        let after = &rest[close + 2..];
        let Some(end) = after.find(')') else { out.push('['); continue };
        out.push_str(text);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Ищет в результате пары «[x]…[/x]» — верный признак неподдержанного тега.
/// Одиночные «[WIP]» в названиях разметкой не считаются.
fn unhandled_tags(md: &str) -> Vec<String> {
    let lower = md.to_lowercase();
    let mut found = Vec::new();
    let mut rest = lower.as_str();

    while let Some(open) = rest.find('[') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find(']') else { break };
        let name = &rest[..close];
        if name.is_empty() || name.len() > 20 || name.contains(' ') || name.starts_with('/') {
            continue;
        }
        let name = name.split('=').next().unwrap_or(name).to_string();
        if rest.contains(&format!("[/{name}]")) && !found.contains(&name) {
            found.push(name);
        }
    }
    found
}
