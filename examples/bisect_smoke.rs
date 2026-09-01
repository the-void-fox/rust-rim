// Во что обходится поиск виновника на реальной сборке:
//   cargo run --example bisect_smoke -- <папка игры> <папка модов> <папка конфига> [--all]
//
// С --all под подозрение берутся все установленные моды — так видно, как цена
// растёт с размером сборки.
//
// Игру не запускает. Вместо прогонов подставляется оракул с заранее известной
// причиной поломки, и считается, сколько прогонов понадобилось бы. Нужно, чтобы
// знать цену заранее: один прогон — это минуты, и разница между 12 и 200
// прогонами решает, пользуются такой кнопкой или нет.

use std::collections::HashSet;

use rust_rim::bisect::{Deps, Search};
use rust_rim::mod_data::{parse_mods_config, scan_dlc_mods, scan_local_mods, ModDb, ModId, Profile};

/// Минуты на один прогон — по замерам живых прогонов среза 5.
const MINUTES_PER_RUN: f64 = 2.5;

fn main() {
    let mut args = std::env::args().skip(1);
    let game = args.next().expect("usage: bisect_smoke <game> <mods> <config>");
    let mods = args.next().expect("нужна папка модов");
    let config = args.next().expect("нужна папка конфига");

    let mut entries = scan_dlc_mods(std::path::Path::new(&game));
    entries.extend(scan_local_mods(std::path::Path::new(&mods)));
    let db = ModDb::build(entries);

    let xml = std::path::Path::new(&config).join("ModsConfig.xml");
    let raw = parse_mods_config(&xml).expect("не удалось прочитать ModsConfig.xml");
    let profile = if std::env::args().any(|a| a == "--all") {
        let mut p = Profile::new();
        for id in db.ids().cloned().collect::<Vec<_>>() {
            p.activate(id);
        }
        p
    } else {
        Profile::from_raw_ids(&raw, &db)
    };

    let candidates: Vec<ModId> = profile
        .order()
        .iter()
        .filter(|id| db.get(id).is_none_or(|m| !m.is_vanilla()))
        .cloned()
        .collect();
    println!(
        "Сборка: {} модов, под подозрением {} (ваниль не трогаем)\n",
        profile.len(),
        candidates.len(),
    );
    if candidates.len() < 2 {
        println!("Сужать нечего.");
        return;
    }

    let deps = Deps::from_db(&db, &profile);
    let first = candidates.first().unwrap().clone();
    let middle = candidates[candidates.len() / 2].clone();
    let last = candidates.last().unwrap().clone();

    println!("{:<34} {:>8}  {:>10}  {}", "случай", "прогонов", "≈ время", "результат");
    println!("{}", "─".repeat(78));

    for (label, cause, hint) in [
        ("один виновник, начало списка", vec![first.clone()], vec![]),
        ("один виновник, середина", vec![middle.clone()], vec![]),
        ("один виновник, конец списка", vec![last.clone()], vec![]),
        (
            "один виновник, лог подсказал",
            vec![middle.clone()],
            vec![middle.clone()],
        ),
        ("конфликт пары", vec![first.clone(), last.clone()], vec![]),
        (
            "конфликт пары, лог подсказал одного",
            vec![first.clone(), last.clone()],
            vec![first.clone()],
        ),
    ] {
        let mut search = Search::new(candidates.clone(), Vec::new(), deps.clone(), hint);
        let runs = drive(&mut search, &cause);
        let result = search.result();
        // Виновника может тянуть за собой зависимость: если мод X требует
        // виновника, набор {X} воспроизводит проблему не хуже. Это не ошибка
        // поиска, поэтому проверяем не совпадение, а замыкание.
        let closed: HashSet<ModId> = deps.close(&result).into_iter().collect();
        let verdict = if cause.iter().all(|c| closed.contains(c)) {
            format!("виновник в наборе из {}", result.len())
        } else {
            format!("промах: {} мод(ов), причины среди них нет", result.len())
        };
        println!(
            "{label:<34} {runs:>8}  {:>8.0} мин  {verdict}",
            runs as f64 * MINUTES_PER_RUN,
        );
    }

    println!(
        "\nПеребор по одному моду стоил бы {} прогонов ≈ {:.0} ч.",
        candidates.len(),
        candidates.len() as f64 * MINUTES_PER_RUN / 60.0,
    );
}

/// Гоняет поиск против оракула: проблема воспроизводится, когда в наборе
/// присутствуют все моды из `cause`.
fn drive(search: &mut Search, cause: &[ModId]) -> usize {
    let mut runs = 0;
    while let Some(candidate) = search.next() {
        let present: HashSet<&ModId> = candidate.active.iter().collect();
        search.record(cause.iter().all(|c| present.contains(c)));
        runs += 1;
        assert!(runs < 5000, "поиск не сходится");
    }
    runs
}
