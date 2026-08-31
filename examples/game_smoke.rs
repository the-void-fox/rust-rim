// Что rust-rim видит в системе: установку игры, префиксы и команду запуска.
//   cargo run --example game_smoke -- <папка игры>
//   cargo run --example game_smoke -- <папка игры> --launch [секунд]
//
// С --launch игра действительно запускается и через указанное время
// (по умолчанию 30 с) снимается — так проверяется, что собранная команда
// рабочая, а не только красиво выглядит.

use rust_rim::game::{launch, paths};
use rust_rim::process::Run;

fn main() {
    let dir = std::env::args().nth(1).expect("usage: game_smoke <game_dir>");
    let game = std::path::Path::new(&dir);

    println!("Папка игры: {}", game.display());
    match paths::find_executable(game) {
        Some(exe) => println!(
            "  Запускаемое: {:?}\n  Нужен слой совместимости: {}",
            exe.path(),
            exe.needs_compat_layer(),
        ),
        None => println!("  Игра не найдена"),
    }
    println!("  Версия: {}", paths::game_version(game).unwrap_or_else(|| "?".into()));

    println!("\nПрефиксы (первым — самый свежий по времени правки конфига):");
    let prefixes = paths::find_prefixes();
    if prefixes.is_empty() {
        println!("  не найдено");
    }
    for p in &prefixes {
        let config = paths::config_dir(&p.data_dir);
        let log = paths::player_log(&p.data_dir);
        println!("  [{}] {}", p.source, p.path.display());
        println!("      данные:      {}", p.data_dir.display());
        println!(
            "      ModsConfig:  {}",
            if config.join("ModsConfig.xml").is_file() { "есть" } else { "нет" },
        );
        println!(
            "      Player.log:  {}",
            if log.is_file() { "есть" } else { "нет" },
        );
    }

    println!("\nКоманды запуска:");
    let settings = launch::LaunchSettings::default();
    for (name, mode) in [
        ("обычный", launch::Mode::Play),
        ("автотест", launch::Mode::QuickTest { log_file: "/tmp/rustrim-quicktest.log".into() }),
    ] {
        match launch::plan(game, &settings, &mode) {
            Ok(plan) => println!("  {name}:\n    {}", plan.display()),
            Err(e) => println!("  {name}: ошибка — {e}"),
        }
    }

    if std::env::args().any(|a| a == "--launch") {
        let secs: u64 = std::env::args()
            .last()
            .and_then(|a| a.parse().ok())
            .unwrap_or(30);
        launch_and_wait(game, &settings, secs);
    }
}

fn launch_and_wait(game: &std::path::Path, settings: &launch::LaunchSettings, secs: u64) {
    let plan = match launch::plan(game, settings, &launch::Mode::Play) {
        Ok(p) => p,
        Err(e) => {
            println!("\nЗапуск невозможен: {e}");
            return;
        }
    };

    println!("\nЗапускаю на {secs} с…");
    let mut run = match Run::spawn(plan.to_command(), plan.display()) {
        Ok(r) => r,
        Err(e) => {
            println!("Не удалось запустить: {e}");
            return;
        }
    };

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if run.poll() {
            println!("Процесс завершился сам: {:?}", run.status());
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    let running = run.is_running();
    if running {
        println!("Время вышло — снимаю процесс");
        run.kill();
    }

    let lines = run.lines();
    println!("\nПерехвачено строк вывода: {} (в буфере {})", run.total_lines(), lines.len());
    println!("Последние 12 строк — это то, что показывает окно запуска:");
    for line in lines.iter().rev().take(12).rev() {
        println!("  | {line}");
    }
}
