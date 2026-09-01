// Статическая проверка реальной сборки:
//   cargo run --example validate_smoke -- <папка игры> <папка модов> <папка конфига> [--all]
//
// С --all в сборку включаются все установленные моды: так видно, срабатывают
// ли правила на нагрузке и сколько это стоит по времени.

use rust_rim::game::paths;
use rust_rim::mod_data::{parse_mods_config, scan_dlc_mods, scan_local_mods, ModDb, Profile};
use rust_rim::validation::{self, Severity};

fn main() {
    let mut args = std::env::args().skip(1);
    let game = args.next().expect("usage: validate_smoke <game> <mods> <config>");
    let mods = args.next().expect("нужна папка модов");
    let config = args.next().expect("нужна папка конфига");

    let mut entries = scan_dlc_mods(std::path::Path::new(&game));
    entries.extend(scan_local_mods(std::path::Path::new(&mods)));
    let db = ModDb::build(entries);

    let all = std::env::args().any(|a| a == "--all");
    let xml = std::path::Path::new(&config).join("ModsConfig.xml");
    let active = parse_mods_config(&xml).expect("не удалось прочитать ModsConfig.xml");
    let profile = if all {
        let mut p = Profile::new();
        for id in db.ids().cloned().collect::<Vec<_>>() {
            p.activate(id);
        }
        p
    } else {
        Profile::from_raw_ids(&active, &db)
    };

    let with_sources = db.iter().filter(|m| !m.dependency_sources.is_empty()).count();
    let total_sources: usize = db.iter().map(|m| m.dependency_sources.len()).sum();
    println!("Модов со ссылками на зависимости: {with_sources}, ссылок всего: {total_sources}");

    let version = paths::game_version(std::path::Path::new(&game));
    println!(
        "Каталог: {} модов, сборка: {} из {} записей конфига, версия игры: {}",
        db.len(),
        profile.len(),
        active.len(),
        version.as_deref().unwrap_or("?"),
    );

    let started = std::time::Instant::now();
    let diagnostics = validation::validate(&db, &profile, version.as_deref());
    let elapsed = started.elapsed();
    let errors = diagnostics.iter().filter(|d| d.severity == Severity::Error).count();
    println!(
        "Ошибок: {errors}, предупреждений: {} — проверка заняла {elapsed:.0?}\n",
        diagnostics.len() - errors,
    );

    let mut by_rule = std::collections::HashMap::new();
    for d in &diagnostics {
        *by_rule.entry(d.rule).or_insert(0usize) += 1;
    }
    let mut counts: Vec<_> = by_rule.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    for (rule, n) in &counts {
        println!("  {n:5}  {rule}");
    }
    println!();

    // Что из этого чинится одной кнопкой.
    use rust_rim::validation::Fix;
    let (mut activate, mut download, mut none) = (0usize, 0usize, 0usize);
    for d in &diagnostics {
        match d.fix {
            Some(Fix::Download(_)) => download += 1,
            Some(_) => activate += 1,
            None => none += 1,
        }
    }
    println!(
        "Чинится кнопкой: {activate}, скачивается из мастерской: {download}, вручную: {none}\n",
    );

    for d in diagnostics.iter().take(if all { 6 } else { usize::MAX }) {
        let mark = if d.severity == Severity::Error { "✕" } else { "⚠" };
        println!("{mark} [{}] {}", d.rule, d.title);
        if !d.detail.is_empty() {
            println!("    {}", d.detail.replace('\n', "\n    "));
        }
    }
}
