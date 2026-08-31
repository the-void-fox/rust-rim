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
    }

    println!("Модов: {}, с описанием: {with_desc}, из них с '[': {with_bbcode}", mods.len());
    println!("Самое длинное описание: {} байт — {}", longest.0, longest.1);

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
