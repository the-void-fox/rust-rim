// Сквозной прогон автотеста на реальной установке:
//   cargo run --example quicktest_smoke -- <папка игры> <папка модов>
//
// ModsConfig.xml на время прогона подменяется и возвращается на место.

use rust_rim::game::{launch, paths};
use rust_rim::mod_data::{parse_mods_config, scan_dlc_mods, scan_local_mods, ModDb, Profile};
use rust_rim::testing::{Config, Phase, TestRun};

fn main() {
    let mut args = std::env::args().skip(1);
    let game_dir = args.next().expect("usage: quicktest_smoke <game> <mods>");
    let mods_dir = args.next().expect("нужна папка модов");
    let game = std::path::Path::new(&game_dir);

    let prefix = paths::find_prefixes().into_iter().next().expect("префикс не найден");
    let config_dir = paths::config_dir(&prefix.data_dir);
    let log_file = prefix.data_dir.join("rustrim-quicktest.log");
    println!("Префикс: {}\nЛог:     {}", prefix.path.display(), log_file.display());

    let mut entries = scan_dlc_mods(game);
    entries.extend(scan_local_mods(std::path::Path::new(&mods_dir)));
    let db = ModDb::build(entries);
    let active = parse_mods_config(&config_dir.join("ModsConfig.xml")).expect("нет ModsConfig.xml");
    let profile = Profile::from_raw_ids(&active, &db);
    println!("Сборка: {} модов из каталога в {}\n", profile.len(), db.len());

    let settings = launch::LaunchSettings::default();
    let mode = launch::Mode::QuickTest { log_file: log_file.clone() };
    let plan = launch::plan(game, &settings, &mode).expect("не удалось собрать команду");

    let exe = paths::find_executable(game).expect("игра не найдена");
    let mut cfg = Config::new(config_dir, log_file);
    cfg.timeout = std::time::Duration::from_secs(300);
    cfg.cleanup = Some((prefix.path.clone(), exe.path().to_path_buf()));
    println!("Порог тишины: {:?}, предел: {:?}\n", cfg.settle, cfg.timeout);

    let mut run = TestRun::start(plan.to_command(), plan.display(), cfg, profile.order())
        .expect("не удалось запустить");

    let mut last = String::new();
    let verdict = loop {
        let phase = format!("{:?}", run.poll());
        if phase != last {
            println!("[{:>4}s] {phase}", run.elapsed().as_secs());
            last = phase;
        }
        if let Phase::Done(v) = run.phase() {
            break v.clone();
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    };

    println!("\n=== Вердикт: {verdict:?} (за {} с) ===", run.elapsed().as_secs());
    std::thread::sleep(std::time::Duration::from_secs(2));

    let issues = run.issues(&db, &profile);
    if issues.is_empty() {
        println!("Записей в логе нет.");
    }
    for issue in issues.iter().take(10) {
        println!("\n[×{}] {}", issue.count, issue.title.chars().take(120).collect::<String>());
        for s in issue.suspects.iter().take(3) {
            println!("    → {} ({})", s.name, s.evidence.join("; "));
        }
    }
}
