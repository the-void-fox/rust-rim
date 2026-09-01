// Проверка обновлений на реальной установке:
//   cargo run --example updates_smoke -- <папка модов>
//
// Ходит в Steam: по сотне модов за запрос. Ничего не скачивает и не меняет.

use rust_rim::mod_data::{scan_local_mods, ModDb};
use rust_rim::updates;

fn main() {
    let mods = std::env::args().nth(1).expect("usage: updates_smoke <mods>");
    let db = ModDb::build(scan_local_mods(std::path::Path::new(&mods)));

    let installed = updates::installed_workshop_mods(&db);
    println!(
        "Каталог: {} модов, из мастерской: {}\n",
        db.len(),
        installed.len(),
    );
    if installed.is_empty() {
        return;
    }

    let started = std::time::Instant::now();
    let found = match updates::check(&installed) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("Не удалось спросить мастерскую: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "Спросили за {:.1?}, обновлений: {}\n",
        started.elapsed(),
        found.len(),
    );

    for u in found.iter().take(25) {
        let days = u.behind().as_secs() / 86_400;
        let name = db.get(&u.id).map(|m| m.name.as_str()).unwrap_or(u.id.as_str());
        println!("  отстал на {days:>5} дн.  {name}");
    }
    if found.len() > 25 {
        println!("  … и ещё {}", found.len() - 25);
    }
}
