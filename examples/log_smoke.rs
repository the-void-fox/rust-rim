// Отладка анализатора логов на реальных модах:
//   cargo run --example log_smoke -- <папка модов> <файл лога>
use rust_rim::log_analysis::{analyze, ModIndex};
use rust_rim::mod_data::scan_local_mods;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mods_dir = args.next().expect("usage: log_smoke <mods_dir> <log_file>");
    let log_file = args.next().expect("usage: log_smoke <mods_dir> <log_file>");

    let mods = scan_local_mods(std::path::Path::new(&mods_dir));
    println!("Модов: {}", mods.len());
    let index = ModIndex::build(&mods);
    let text = std::fs::read_to_string(&log_file)?;

    for issue in analyze(&text, &index) {
        println!("\n[{}×{}] {}",
            if issue.suspects.is_empty() { " " } else { "!" },
            issue.count,
            issue.title.chars().take(100).collect::<String>());
        for s in &issue.suspects {
            println!("    → {} [{}] score={}  ({})",
                s.name, s.package_id, s.score, s.evidence.join("; "));
        }
        if let Some(h) = &issue.harmony_hint {
            println!("    ~ Harmony-патч на {h}");
        }
    }
    Ok(())
}
