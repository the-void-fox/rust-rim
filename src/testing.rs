//! Автотест сборки: прогон игры с `-quicktest` и вердикт по логу.
//!
//! Поведение подсмотрено на живых прогонах, и оно неочевидно:
//!
//! * лог не появляется первую минуту — Proton поднимает окружение, а Unity
//!   создаёт файл не сразу; отсутствие файла не значит провал;
//! * Prepatcher перезапускает игру, и строка `RimWorld <версия>` встречается
//!   в логе дважды — по одному вхождению судить нельзя;
//! * `-quicktest` не выходит сам: игра генерирует карту и остаётся в ней.
//!   Признак «загрузилось» — лог перестал расти, а процесс жив.
//!
//! Поэтому вердикт выносится по тишине в логе, а не по завершению процесса.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::game::teardown;
use crate::log_analysis::{self, LogIssue, ModIndex, Severity};
use crate::mod_data::{write_mods_config, ModDb, ModId, Profile};
use crate::process::Run;

/// Сколько лог должен молчать, чтобы считать загрузку завершённой.
pub const DEFAULT_SETTLE: Duration = Duration::from_secs(25);
/// Общий предел на прогон.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Debug)]
pub struct Config {
    /// Папка с ModsConfig.xml — туда кладётся тестируемая сборка.
    pub config_dir: PathBuf,
    /// Куда игра пишет лог. Должен лежать внутри префикса: под umu игра
    /// работает в контейнере со своим `/tmp`.
    pub log_file: PathBuf,
    pub settle: Duration,
    pub timeout: Duration,
    /// Чем добивать цепочку wine/Proton: (префикс, путь к exe).
    /// Без этого игра переживает снятие процесса-обёртки.
    pub cleanup: Option<(PathBuf, PathBuf)>,
}

impl Config {
    pub fn new(config_dir: PathBuf, log_file: PathBuf) -> Self {
        Self {
            config_dir,
            log_file,
            settle: DEFAULT_SETTLE,
            timeout: DEFAULT_TIMEOUT,
            cleanup: None,
        }
    }
}

/// На какой стадии прогон.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Процесс запущен, лога ещё нет.
    Starting,
    /// Лог растёт.
    Loading { lines: usize },
    /// Лог молчит, ждём подтверждения.
    Settling { lines: usize, quiet: Duration },
    Done(Verdict),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Игра загрузилась, ошибок в логе нет.
    Passed,
    /// Игра загрузилась, но в логе есть ошибки.
    LoadedWithErrors,
    /// Игра завершилась сама, даже не начав загружаться.
    Crashed { code: Option<i32> },
    /// Процессы игры исчезли посреди загрузки модов — до карты не дошло.
    /// Под umu это бывает, когда Prepatcher перезапускает игру, а контейнер
    /// pressure-vessel к тому моменту уже свернулся вслед за обёрткой.
    DiedWhileLoading,
    /// Не уложились в отведённое время.
    TimedOut,
    /// Прогон остановлен пользователем.
    Cancelled,
}

impl Verdict {
    pub fn is_success(&self) -> bool {
        matches!(self, Verdict::Passed)
    }
}

/// Идущий прогон.
pub struct TestRun {
    run: Run,
    config: Config,
    /// Восстанавливает ModsConfig.xml даже при панике.
    _guard: ConfigGuard,
    started: Instant,
    log_len: usize,
    last_growth: Instant,
    phase: Phase,
}

impl TestRun {
    /// Записывает сборку в ModsConfig.xml и запускает игру.
    ///
    /// Оригинальный конфиг сохраняется и возвращается на место при
    /// завершении прогона — в том числе при панике.
    pub fn start(
        command: Command,
        command_text: String,
        config: Config,
        active: &[ModId],
    ) -> io::Result<Self> {
        let guard = ConfigGuard::install(&config.config_dir, active)?;
        // Старый лог мешает: по нему нельзя отличить прошлый прогон от текущего.
        let _ = std::fs::remove_file(&config.log_file);

        let run = Run::spawn(command, command_text)?;
        let now = Instant::now();
        Ok(Self {
            run,
            config,
            _guard: guard,
            started: now,
            log_len: 0,
            last_growth: now,
            phase: Phase::Starting,
        })
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Вывод самого процесса (Proton/umu), а не игры.
    pub fn process_lines(&self) -> Vec<String> {
        self.run.lines()
    }

    pub fn log_text(&self) -> String {
        std::fs::read_to_string(&self.config.log_file).unwrap_or_default()
    }

    /// Двигает состояние. Вызывать регулярно, пока фаза не станет `Done`.
    pub fn poll(&mut self) -> &Phase {
        if matches!(self.phase, Phase::Done(_)) {
            return &self.phase;
        }
        self.run.poll();

        let text = self.log_text();
        let lines = text.lines().count();
        if text.len() != self.log_len {
            self.log_len = text.len();
            self.last_growth = Instant::now();
        }

        // Завершение обёртки ещё ничего не значит: umu-run выходит с кодом 0,
        // пока игра стартует дальше (Prepatcher перезапускает её отдельным
        // процессом). Поэтому спрашиваем систему, жив ли кто-то из нашего
        // запуска, — и только это считаем концом.
        if !self.run.is_running() && !self.game_alive() {
            let code = self.run.status().and_then(|s| s.code());
            self.phase = Phase::Done(match progress(&text) {
                Progress::Loaded => verdict_from_log(&text),
                Progress::Started => Verdict::DiedWhileLoading,
                Progress::NotStarted => Verdict::Crashed { code },
            });
            return &self.phase;
        }

        if self.started.elapsed() >= self.config.timeout {
            self.stop_game();
            self.phase = Phase::Done(Verdict::TimedOut);
            return &self.phase;
        }

        let quiet = self.last_growth.elapsed();
        // Тишина засчитывается только после завершения загрузки: иначе
        // пауза посреди разбора модов сошла бы за успех.
        self.phase = match progress(&text) {
            Progress::Loaded if quiet >= self.config.settle => {
                self.stop_game();
                Phase::Done(verdict_from_log(&text))
            }
            Progress::Loaded => Phase::Settling { lines, quiet },
            Progress::NotStarted if lines == 0 => Phase::Starting,
            _ => Phase::Loading { lines },
        };
        &self.phase
    }

    /// Останавливает прогон досрочно.
    pub fn cancel(&mut self) {
        self.stop_game();
        self.phase = Phase::Done(Verdict::Cancelled);
    }

    /// Остались ли живые процессы игры. Скан /proc делается только когда
    /// обёртка уже вышла, а не каждый опрос.
    fn game_alive(&self) -> bool {
        match &self.config.cleanup {
            Some((prefix, exe)) => teardown::any_running(prefix, exe),
            None => false,
        }
    }

    /// Снимает и обёртку, и всё, что она за собой оставила.
    fn stop_game(&mut self) {
        self.run.kill();
        if let Some((prefix, exe)) = &self.config.cleanup {
            teardown::stop(prefix, exe);
        }
    }

    /// Разбирает лог прогона и ищет виновников среди модов.
    pub fn issues(&self, db: &ModDb, profile: &Profile) -> Vec<LogIssue> {
        let index = ModIndex::build(db, profile);
        log_analysis::analyze(&self.log_text(), &index)
    }
}

/// Как далеко продвинулся прогон.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Progress {
    /// Игра ещё не объявила о себе.
    NotStarted,
    /// Ядро стартовало, моды грузятся.
    Started,
    /// Загрузка завершена — Unity сменила сцену.
    Loaded,
}

/// Строку с версией игра печатает *до* загрузки модов, поэтому одной её
/// мало: проверено, что прогон обрывается сразу после неё и при этом
/// выглядит успешным.
///
/// Признак завершённой загрузки — выгрузка ассетов при смене сцены: в
/// `-quicktest` она происходит на переходе с экрана загрузки в
/// сгенерированную карту.
fn progress(log: &str) -> Progress {
    let mut started = false;
    for line in log.lines() {
        if line
            .strip_prefix("RimWorld ")
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        {
            started = true;
        }
        if started && line.contains("unused Assets to reduce memory usage") {
            return Progress::Loaded;
        }
    }
    if started { Progress::Started } else { Progress::NotStarted }
}

fn verdict_from_log(log: &str) -> Verdict {
    let has_errors = log_analysis::parse_log(log)
        .iter()
        .any(|i| i.severity == Severity::Error);
    if has_errors {
        Verdict::LoadedWithErrors
    } else {
        Verdict::Passed
    }
}

// ─── Подмена ModsConfig.xml на время прогона ─────────────────────────────────

/// Кладёт тестируемую сборку в ModsConfig.xml и возвращает исходный файл
/// на место при уничтожении.
struct ConfigGuard {
    path: PathBuf,
    backup: PathBuf,
    original: Option<Vec<u8>>,
}

impl ConfigGuard {
    fn install(config_dir: &Path, active: &[ModId]) -> io::Result<Self> {
        let path = config_dir.join("ModsConfig.xml");
        let original = std::fs::read(&path).ok();

        // Копия на диске — страховка на случай, если приложение убьют
        // посреди прогона и Drop не отработает.
        let backup = config_dir.join("ModsConfig.xml.rustrim-backup");
        if let Some(bytes) = &original {
            std::fs::write(&backup, bytes)?;
        }

        let ids: Vec<String> = active.iter().map(|id| id.as_str().to_string()).collect();
        write_mods_config(&path, &ids).map_err(io::Error::other)?;

        Ok(Self { path, backup, original })
    }
}

impl Drop for ConfigGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(bytes) => {
                if let Err(e) = std::fs::write(&self.path, bytes) {
                    tracing::error!(
                        "Не удалось вернуть ModsConfig.xml: {e}. Копия лежит в {:?}",
                        self.backup,
                    );
                    return;
                }
            }
            // Файла не было — не оставляем свой.
            None => {
                let _ = std::fs::remove_file(&self.path);
            }
        }
        let _ = std::fs::remove_file(&self.backup);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("rustrim_testing_{}_{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ids(list: &[&str]) -> Vec<ModId> {
        list.iter().map(|s| ModId::new(s)).collect()
    }

    /// Скрипт, изображающий игру: пишет строки в лог с задержками.
    fn fake_game(log: &Path, script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script.replace("{log}", &log.display().to_string()));
        cmd
    }

    fn drive(run: &mut TestRun, limit: Duration) -> Verdict {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if let Phase::Done(v) = run.poll() {
                return v.clone();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("прогон не завершился: {:?}", run.phase());
    }

    #[test]
    fn successful_run_is_detected_by_silence() {
        // Игра пишет строку версии и замолкает, продолжая работать —
        // именно так ведёт себя -quicktest.
        let t = Temp::new("ok");
        let log = t.0.join("game.log");
        let mut cfg = Config::new(t.0.clone(), log.clone());
        cfg.settle = Duration::from_millis(300);

        let cmd = fake_game(&log, "printf 'RimWorld 1.6.4633 rev1261\\nUnloading 99 unused Assets to reduce memory usage.\\n' > '{log}'; sleep 30");
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["a.mod"])).unwrap();

        assert_eq!(drive(&mut run, Duration::from_secs(10)), Verdict::Passed);
    }

    #[test]
    fn errors_in_log_downgrade_the_verdict() {
        let t = Temp::new("errors");
        let log = t.0.join("game.log");
        let mut cfg = Config::new(t.0.clone(), log.clone());
        cfg.settle = Duration::from_millis(300);

        let cmd = fake_game(
            &log,
            "printf 'RimWorld 1.6.4633\\nSystem.NullReferenceException: boom\\nUnloading 99 unused Assets to reduce memory usage.\\n' > '{log}'; sleep 30",
        );
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["a.mod"])).unwrap();

        assert_eq!(drive(&mut run, Duration::from_secs(10)), Verdict::LoadedWithErrors);
    }

    #[test]
    fn exit_before_loading_is_a_crash() {
        let t = Temp::new("crash");
        let log = t.0.join("game.log");
        let mut cfg = Config::new(t.0.clone(), log.clone());
        cfg.settle = Duration::from_millis(300);

        let cmd = fake_game(&log, "printf 'Mono path[0] = x\\n' > '{log}'; exit 1");
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["a.mod"])).unwrap();

        assert_eq!(
            drive(&mut run, Duration::from_secs(10)),
            Verdict::Crashed { code: Some(1) },
        );
    }

    #[test]
    fn slow_start_without_log_is_not_a_failure() {
        // Первую минуту реального прогона файла лога вообще нет.
        let t = Temp::new("slow");
        let log = t.0.join("game.log");
        let mut cfg = Config::new(t.0.clone(), log.clone());
        cfg.settle = Duration::from_millis(200);

        let cmd = fake_game(&log, "sleep 1; printf 'RimWorld 1.6\\nUnloading 99 unused Assets to reduce memory usage.\\n' > '{log}'; sleep 30");
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["a.mod"])).unwrap();

        run.poll();
        assert_eq!(run.phase(), &Phase::Starting, "нет лога — это ещё не провал");
        assert_eq!(drive(&mut run, Duration::from_secs(15)), Verdict::Passed);
    }

    #[test]
    fn timeout_is_reported() {
        let t = Temp::new("timeout");
        let log = t.0.join("game.log");
        let mut cfg = Config::new(t.0.clone(), log.clone());
        cfg.timeout = Duration::from_millis(400);

        let cmd = fake_game(&log, "sleep 30");
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["a.mod"])).unwrap();

        assert_eq!(drive(&mut run, Duration::from_secs(10)), Verdict::TimedOut);
    }

    #[test]
    fn config_is_written_and_restored() {
        let t = Temp::new("config");
        let path = t.0.join("ModsConfig.xml");
        std::fs::write(&path, "<?xml version=\"1.0\"?>\n<ModsConfigData>\n\t<version>1.6</version>\n\t<activeMods>\n\t\t<li>original.mod</li>\n\t</activeMods>\n</ModsConfigData>\n").unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        let log = t.0.join("game.log");
        let mut cfg = Config::new(t.0.clone(), log.clone());
        cfg.settle = Duration::from_millis(200);

        {
            let cmd = fake_game(&log, "printf 'RimWorld 1.6\\nUnloading 99 unused Assets to reduce memory usage.\\n' > '{log}'; sleep 30");
            let mut run =
                TestRun::start(cmd, "fake".into(), cfg, &ids(&["tested.mod"])).unwrap();

            // Во время прогона в конфиге лежит тестируемая сборка.
            let during = std::fs::read_to_string(&path).unwrap();
            assert!(during.contains("tested.mod"), "{during}");
            assert!(!during.contains("original.mod"), "{during}");

            drive(&mut run, Duration::from_secs(10));
        }

        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "конфиг не восстановлен");
        assert!(!t.0.join("ModsConfig.xml.rustrim-backup").exists(), "копия не убрана");
    }

    #[test]
    fn config_is_restored_even_if_run_is_dropped_early() {
        let t = Temp::new("drop");
        let path = t.0.join("ModsConfig.xml");
        std::fs::write(&path, "<ModsConfigData><activeMods><li>original.mod</li></activeMods></ModsConfigData>").unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        let log = t.0.join("game.log");
        let cfg = Config::new(t.0.clone(), log.clone());
        {
            let cmd = fake_game(&log, "sleep 30");
            let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["tested.mod"])).unwrap();
            run.cancel();
        }

        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn progress_needs_more_than_the_version_line() {
        // Реальный обрыв: лог заканчивался строкой версии, и прогон
        // засчитывался как успешный, хотя моды не догрузились.
        assert_eq!(progress(""), Progress::NotStarted);
        assert_eq!(progress("Command line arguments: -quicktest\n"), Progress::NotStarted);
        assert_eq!(progress("RimWorld by Ludeon Studios\n"), Progress::NotStarted);
        assert_eq!(
            progress("Mono path[0] = x\nRimWorld 1.6.4633 rev1261\n"),
            Progress::Started,
        );
        assert_eq!(
            progress("RimWorld 1.6.4633\nUnloading 99 unused Assets to reduce memory usage.\n"),
            Progress::Loaded,
        );
    }

    #[test]
    fn death_during_mod_loading_is_not_a_pass() {
        let t = Temp::new("died");
        let log = t.0.join("game.log");
        let mut cfg = Config::new(t.0.clone(), log.clone());
        cfg.settle = Duration::from_millis(200);

        // Игра объявила версию и умерла, не догрузив моды.
        let cmd = fake_game(&log, "printf 'RimWorld 1.6.4633\\n' > '{log}'; exit 0");
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["a.mod"])).unwrap();

        assert_eq!(drive(&mut run, Duration::from_secs(10)), Verdict::DiedWhileLoading);
    }
}
