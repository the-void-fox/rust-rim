//! Автотест сборки: прогон игры с `-quicktest` и вердикт по логу.
//!
//! Поведение подсмотрено на живых прогонах, и оно неочевидно:
//!
//! * лог не появляется первую минуту — Proton поднимает окружение, а Unity
//!   создаёт файл не сразу; отсутствие файла не значит провал;
//! * Prepatcher перезапускает игру, и строка `RimWorld <версия>` встречается
//!   в логе дважды — по одному вхождению судить нельзя;
//! * `-quicktest` не выходит сам: игра генерирует карту и остаётся в ней.
//!   Признак «загрузилось» — лог перестал расти, а процесс жив;
//! * тишины может не быть вовсе. Сломанный мод роняет исключение каждый кадр,
//!   и лог растёт до самого конца отведённого времени — как раз в том случае,
//!   ради которого прогон и затевался. Поэтому поток повторяющихся ошибок
//!   распознаётся отдельно, а ожидание тишины ограничено сверху.
//!
//! Поэтому вердикт выносится по тишине в логе, а не по завершению процесса.
//!
//! Конфиг игрока прогон не трогает. Игра получает свою папку данных через
//! `-savedatafolder`: туда кладутся тестируемый ModsConfig.xml и копия
//! настроек игрока, а несколько настроек переопределяются — без этого игра
//! встаёт на паузу или замирает, и прогон «молчит», как будто всё чисто.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::game::teardown;
use crate::log_analysis::{self, LogIssue, ModIndex, Severity};
use crate::mod_data::{write_mods_config, ModDb, ModId, Profile};
use crate::process::Run;
use crate::trf;

/// Сколько лог должен молчать, чтобы считать загрузку завершённой.
pub const DEFAULT_SETTLE: Duration = Duration::from_secs(25);
/// Сколько всего ждать тишины после загрузки, прежде чем судить по тому,
/// что уже написано. Без потолка сборка, которая пишет в лог без остановки,
/// висела бы до самого таймаута.
pub const DEFAULT_SETTLE_CAP: Duration = Duration::from_secs(120);
/// Общий предел на прогон.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

/// Как часто перечитывать лог. Опрос идёт из отрисовки, то есть до 60 раз в
/// секунду, а лог к концу прогона — сотни килобайт.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Этим RimWorld помечает повтор уже напечатанного исключения.
///
/// Отличный признак потока: в обычном Player.log таких строк ноль, а в
/// прогоне со сломанным модом их было 650 на 2545 строк — исключение
/// прилетало каждый кадр из `Update()` и из тика каждого джоба.
const REPEAT_MARKER: &str = "Duplicate stacktrace, see ref for original";
/// Столько повторов — это уже поток, а не хвост загрузки.
const STORM_REPEATS: usize = 40;

#[derive(Clone, Debug)]
pub struct Config {
    /// Папка Config игрока. Из неё копируются настройки игры и модов;
    /// сама она не меняется.
    pub config_dir: PathBuf,
    /// Своя папка данных прогона — игра получает её через `-savedatafolder`.
    /// Пересоздаётся перед каждым прогоном.
    pub save_data: PathBuf,
    /// Куда игра пишет лог. Должен лежать внутри префикса: под umu игра
    /// работает в контейнере со своим `/tmp`.
    pub log_file: PathBuf,
    pub settle: Duration,
    /// Потолок ожидания тишины, отсчитывается от конца загрузки.
    pub settle_cap: Duration,
    pub timeout: Duration,
    /// Чем добивать цепочку wine/Proton: (префикс, путь к exe).
    /// Без этого игра переживает снятие процесса-обёртки.
    pub cleanup: Option<(PathBuf, PathBuf)>,
}

impl Config {
    pub fn new(config_dir: PathBuf, save_data: PathBuf, log_file: PathBuf) -> Self {
        Self {
            config_dir,
            save_data,
            log_file,
            settle: DEFAULT_SETTLE,
            settle_cap: DEFAULT_SETTLE_CAP,
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
    /// Одна и та же ошибка повторяется без остановки: игра работает, но
    /// сыплет исключениями каждый кадр. Прогон прерван, ждать нечего.
    ErrorStorm,
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
    /// Состав прогоняемой сборки.
    active: Vec<ModId>,
    started: Instant,
    log_len: usize,
    last_growth: Instant,
    /// Когда лог впервые сообщил, что загрузка позади.
    loaded_at: Option<Instant>,
    last_poll: Option<Instant>,
    phase: Phase,
}

impl TestRun {
    /// Готовит папку данных прогона со сборкой и запускает игру.
    ///
    /// Команда должна передать игре ту же папку через `-savedatafolder` —
    /// её собирает [`crate::game::launch::plan`] из `Mode::QuickTest`.
    pub fn start(
        command: Command,
        command_text: String,
        config: Config,
        active: &[ModId],
    ) -> io::Result<Self> {
        prepare_save_data(&config.config_dir, &config.save_data, active)?;
        // Старый лог мешает: по нему нельзя отличить прошлый прогон от текущего.
        let _ = std::fs::remove_file(&config.log_file);

        let run = Run::spawn(command, command_text)?;
        let now = Instant::now();
        Ok(Self {
            run,
            config,
            active: active.to_vec(),
            started: now,
            log_len: 0,
            last_growth: now,
            loaded_at: None,
            last_poll: None,
            phase: Phase::Starting,
        })
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Состав сборки, с которой идёт прогон.
    pub fn active(&self) -> &[ModId] {
        &self.active
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
        if self.last_poll.is_some_and(|t| t.elapsed() < POLL_INTERVAL) {
            return &self.phase;
        }
        self.last_poll = Some(Instant::now());
        self.run.poll();

        let text = self.log_text();
        let lines = text.lines().count();
        if text.len() != self.log_len {
            self.log_len = text.len();
            self.last_growth = Instant::now();
        }
        let stage = progress(&text);

        // Поток повторяющихся исключений: тишины не будет никогда, ждать её
        // бессмысленно. Ответ у нас уже есть — сборка неиграбельна.
        if stage != Progress::NotStarted && text.matches(REPEAT_MARKER).count() >= STORM_REPEATS {
            self.stop_game();
            self.phase = Phase::Done(Verdict::ErrorStorm);
            return &self.phase;
        }

        // Завершение обёртки ещё ничего не значит: umu-run выходит с кодом 0,
        // пока игра стартует дальше (Prepatcher перезапускает её отдельным
        // процессом). Поэтому спрашиваем систему, жив ли кто-то из нашего
        // запуска, — и только это считаем концом.
        if !self.run.is_running() && !self.game_alive() {
            let code = self.run.status().and_then(|s| s.code());
            self.phase = Phase::Done(match stage {
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
        self.phase = match stage {
            Progress::Loaded => {
                let waiting = self.loaded_at.get_or_insert_with(Instant::now).elapsed();
                // Потолок нужен и без повторов: мод может писать в лог по
                // таймеру, и тогда «тихих» окон не будет вообще.
                if quiet >= self.config.settle || waiting >= self.config.settle_cap {
                    self.stop_game();
                    Phase::Done(verdict_from_log(&text))
                } else {
                    Phase::Settling { lines, quiet }
                }
            }
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

// ─── Своя папка данных прогона ───────────────────────────────────────────────

/// Метка папки, которую прогон создал сам. Стирать можно только такую:
/// путь к папке собирается из настроек, и ошибка в них не должна стоить
/// игроку сохранений.
const MARKER: &str = ".rustrim-test";

/// Настройки игры, которые прогон переопределяет.
///
/// Всё это найдено на живых прогонах, и каждое без переопределения делает
/// результат ложно-чистым:
/// * `runInBackground` — после загрузки игра применяет настройку игрока, и
///   без фокуса Unity перестаёт обновлять кадры: карта не генерируется,
///   тики не идут, лог молчит, а тишина засчитывается как успех;
/// * `automaticPauseMode` — по умолчанию игра сама встаёт на паузу при
///   крупной угрозе: эталонный прогон замер на 305-м тике;
/// * `pauseOnError` — пауза на первой же ошибке прячет все остальные.
const PREFS_OVERRIDES: &[(&str, &str)] = &[
    ("runInBackground", "True"),
    ("automaticPauseMode", "Never"),
    ("pauseOnError", "False"),
];

/// Пересоздаёт папку данных прогона: копия Config игрока, тестируемый
/// список модов и переопределённые настройки.
///
/// Настройки модов (`Mod_*.xml`, подпапки Config) копируются как есть — от
/// них зависит поведение модов, а значит и ошибки.
fn prepare_save_data(config_dir: &Path, save_data: &Path, active: &[ModId]) -> io::Result<()> {
    if save_data.exists() {
        if save_data.join(MARKER).is_file() {
            std::fs::remove_dir_all(save_data)?;
        } else if std::fs::read_dir(save_data)?.next().is_some() {
            return Err(io::Error::other(trf!(
                "Папка {path} не пуста и создана не прогоном — стирать её не стану.",
                path = save_data.display()
            )));
        }
    }
    std::fs::create_dir_all(save_data)?;
    std::fs::write(save_data.join(MARKER), b"")?;

    let target = save_data.join("Config");
    if config_dir.is_dir() {
        crate::fs_util::copy_dir_recursive(config_dir, &target)?;
    } else {
        std::fs::create_dir_all(&target)?;
    }

    // Версия и knownExpansions берутся из скопированного файла игрока.
    let ids: Vec<String> = active.iter().map(|id| id.as_str().to_string()).collect();
    let mods_config = target.join("ModsConfig.xml");
    write_mods_config(&mods_config, &ids).map_err(io::Error::other)?;
    let _ = std::fs::remove_file(mods_config.with_extension("xml.bak"));

    let prefs = target.join("Prefs.xml");
    let text = std::fs::read_to_string(&prefs)
        .unwrap_or_else(|_| "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<PrefsData>\n</PrefsData>\n".into());
    std::fs::write(&prefs, override_prefs(&text))
}

/// Подставляет [`PREFS_OVERRIDES`] в текст Prefs.xml: меняет значение, если
/// поле есть, и дописывает его перед `</PrefsData>`, если нет. Остальные
/// настройки (язык, разрешение, режим разработчика) не трогаются.
fn override_prefs(text: &str) -> String {
    let mut out = text.to_string();
    for (field, value) in PREFS_OVERRIDES {
        let open = format!("<{field}>");
        let close = format!("</{field}>");
        let existing = out.find(&open).and_then(|start| {
            let body = start + open.len();
            out[body..].find(&close).map(|len| (body, body + len))
        });
        match existing {
            Some((from, to)) => out.replace_range(from..to, value),
            None => {
                let line = format!("\t{open}{value}{close}\n");
                match out.rfind("</PrefsData>") {
                    Some(at) => out.insert_str(at, &line),
                    None => out.push_str(&line),
                }
            }
        }
    }
    out
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
        let mut cfg = Config::new(t.0.join("Config"), t.0.join("run"), log.clone());
        cfg.settle = Duration::from_millis(300);

        let cmd = fake_game(&log, "printf 'RimWorld 1.6.4633 rev1261\\nUnloading 99 unused Assets to reduce memory usage.\\n' > '{log}'; sleep 30");
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["a.mod"])).unwrap();

        assert_eq!(drive(&mut run, Duration::from_secs(10)), Verdict::Passed);
    }

    #[test]
    fn errors_in_log_downgrade_the_verdict() {
        let t = Temp::new("errors");
        let log = t.0.join("game.log");
        let mut cfg = Config::new(t.0.join("Config"), t.0.join("run"), log.clone());
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
        let mut cfg = Config::new(t.0.join("Config"), t.0.join("run"), log.clone());
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
        let mut cfg = Config::new(t.0.join("Config"), t.0.join("run"), log.clone());
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
        let mut cfg = Config::new(t.0.join("Config"), t.0.join("run"), log.clone());
        cfg.timeout = Duration::from_millis(400);

        let cmd = fake_game(&log, "sleep 30");
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["a.mod"])).unwrap();

        assert_eq!(drive(&mut run, Duration::from_secs(10)), Verdict::TimedOut);
    }

    const PLAYER_MODS: &str = "<?xml version=\"1.0\"?>\n<ModsConfigData>\n\t<version>1.6.4633 rev1261</version>\n\t<activeMods>\n\t\t<li>original.mod</li>\n\t</activeMods>\n\t<knownExpansions>\n\t\t<li>ludeon.rimworld.biotech</li>\n\t</knownExpansions>\n</ModsConfigData>\n";
    const PLAYER_PREFS: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<PrefsData>\n  <runInBackground>False</runInBackground>\n  <automaticPauseMode>MajorThreat</automaticPauseMode>\n  <langFolderName>Russian (Русский)</langFolderName>\n  <devMode>True</devMode>\n</PrefsData>\n";

    /// Config игрока: список модов, Prefs, настройки модов и подпапка мода.
    fn player_config(t: &Temp) -> PathBuf {
        let dir = t.0.join("Config");
        std::fs::create_dir_all(dir.join("RimHUD")).unwrap();
        std::fs::write(dir.join("ModsConfig.xml"), PLAYER_MODS).unwrap();
        std::fs::write(dir.join("Prefs.xml"), PLAYER_PREFS).unwrap();
        std::fs::write(dir.join("Mod_123_SomeMod.xml"), "<SettingsBlock/>").unwrap();
        std::fs::write(dir.join("RimHUD/Config.xml"), "<Hud/>").unwrap();
        dir
    }

    #[test]
    fn the_build_goes_into_its_own_folder_and_player_config_is_untouched() {
        let t = Temp::new("sandbox");
        let config = player_config(&t);
        let log = t.0.join("game.log");
        let mut cfg = Config::new(config.clone(), t.0.join("run"), log.clone());
        cfg.settle = Duration::from_millis(200);

        let cmd = fake_game(&log, "printf 'RimWorld 1.6\\nUnloading 99 unused Assets to reduce memory usage.\\n' > '{log}'; sleep 30");
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["tested.mod"])).unwrap();

        let tested = std::fs::read_to_string(t.0.join("run/Config/ModsConfig.xml")).unwrap();
        assert!(tested.contains("tested.mod") && !tested.contains("original.mod"), "{tested}");
        // Версия и известные DLC — от игрока: иначе игра покажет окно
        // «найдено новое дополнение» и прогон встанет.
        assert!(tested.contains("1.6.4633 rev1261") && tested.contains("ludeon.rimworld.biotech"), "{tested}");
        // Настройки модов — как у игрока.
        assert!(t.0.join("run/Config/Mod_123_SomeMod.xml").is_file());
        assert!(t.0.join("run/Config/RimHUD/Config.xml").is_file());

        drive(&mut run, Duration::from_secs(10));
        // Игрока не тронули ни во время прогона, ни после.
        assert_eq!(std::fs::read_to_string(config.join("ModsConfig.xml")).unwrap(), PLAYER_MODS);
        assert_eq!(std::fs::read_to_string(config.join("Prefs.xml")).unwrap(), PLAYER_PREFS);
        assert!(!config.join("ModsConfig.xml.bak").exists());
    }

    #[test]
    fn prefs_that_freeze_or_pause_the_game_are_overridden() {
        let t = Temp::new("prefs");
        let config = player_config(&t);
        prepare_save_data(&config, &t.0.join("run"), &ids(&["a.mod"])).unwrap();

        let prefs = std::fs::read_to_string(t.0.join("run/Config/Prefs.xml")).unwrap();
        assert!(prefs.contains("<runInBackground>True</runInBackground>"), "{prefs}");
        assert!(prefs.contains("<automaticPauseMode>Never</automaticPauseMode>"), "{prefs}");
        // Поля не было — дописано внутрь PrefsData.
        let at = prefs.find("<pauseOnError>False</pauseOnError>").expect("pauseOnError не дописан");
        assert!(at < prefs.find("</PrefsData>").unwrap(), "{prefs}");
        // Остальное — как у игрока: язык влияет на ошибки перевода.
        assert!(prefs.contains("<langFolderName>Russian (Русский)</langFolderName>"), "{prefs}");
        assert!(!prefs.contains("False</runInBackground>"), "{prefs}");
    }

    #[test]
    fn a_fresh_install_without_prefs_still_gets_the_overrides() {
        let t = Temp::new("noprefs");
        prepare_save_data(&t.0.join("Config"), &t.0.join("run"), &ids(&["a.mod"])).unwrap();
        let prefs = std::fs::read_to_string(t.0.join("run/Config/Prefs.xml")).unwrap();
        assert!(prefs.contains("<runInBackground>True</runInBackground>"), "{prefs}");
        assert!(prefs.trim_end().ends_with("</PrefsData>"), "{prefs}");
    }

    #[test]
    fn the_previous_run_does_not_leak_into_the_next() {
        let t = Temp::new("rerun");
        let config = player_config(&t);
        let run_dir = t.0.join("run");
        prepare_save_data(&config, &run_dir, &ids(&["first.mod"])).unwrap();
        // Мод что-то записал в папку прогона.
        std::fs::write(run_dir.join("Config/LeftBehind.xml"), "x").unwrap();

        prepare_save_data(&config, &run_dir, &ids(&["second.mod"])).unwrap();
        assert!(!run_dir.join("Config/LeftBehind.xml").exists());
        let tested = std::fs::read_to_string(run_dir.join("Config/ModsConfig.xml")).unwrap();
        assert!(tested.contains("second.mod") && !tested.contains("first.mod"), "{tested}");
    }

    #[test]
    fn a_foreign_folder_is_never_wiped() {
        // Путь собирается из настроек: если он укажет на настоящую папку
        // данных игры, стирать её нельзя ни при каких условиях.
        let t = Temp::new("foreign");
        let config = player_config(&t);
        let foreign = t.0.join("RealData");
        std::fs::create_dir_all(foreign.join("Saves")).unwrap();
        std::fs::write(foreign.join("Saves/colony.rws"), "save").unwrap();

        assert!(prepare_save_data(&config, &foreign, &ids(&["a.mod"])).is_err());
        assert!(foreign.join("Saves/colony.rws").is_file(), "сохранение стёрто");
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
    fn endless_repeats_end_the_run_instead_of_waiting_for_silence() {
        // Живой случай: игра дошла до карты и роняет исключение каждый кадр.
        // Тишины не будет никогда, а ответ уже известен.
        let t = Temp::new("storm");
        let log = t.0.join("game.log");
        let mut cfg = Config::new(t.0.join("Config"), t.0.join("run"), log.clone());
        cfg.settle = Duration::from_secs(30);
        cfg.settle_cap = Duration::from_secs(30);

        let cmd = fake_game(
            &log,
            "printf 'RimWorld 1.6\\nUnloading 99 unused Assets to reduce memory usage.\\n' > '{log}'; \
             i=0; while [ $i -lt 60 ]; do \
             printf 'System.NullReferenceException: boom\\n[Ref ABC] Duplicate stacktrace, see ref for original\\n' >> '{log}'; \
             i=$((i+1)); done; sleep 30",
        );
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["a.mod"])).unwrap();

        assert_eq!(drive(&mut run, Duration::from_secs(10)), Verdict::ErrorStorm);
    }

    #[test]
    fn a_log_that_never_goes_quiet_still_gets_a_verdict() {
        // Мод пишет в лог по таймеру: повторов нет, но и тишины тоже.
        // Без потолка прогон висел бы до самого таймаута.
        let t = Temp::new("chatty");
        let log = t.0.join("game.log");
        let mut cfg = Config::new(t.0.join("Config"), t.0.join("run"), log.clone());
        cfg.settle = Duration::from_secs(30);
        cfg.settle_cap = Duration::from_millis(800);

        let cmd = fake_game(
            &log,
            "printf 'RimWorld 1.6\\nUnloading 99 unused Assets to reduce memory usage.\\n' > '{log}'; \
             i=0; while true; do printf 'chatty mod tick %s\\n' \"$i\" >> '{log}'; \
             i=$((i+1)); sleep 0.1; done",
        );
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["a.mod"])).unwrap();

        assert_eq!(drive(&mut run, Duration::from_secs(10)), Verdict::Passed);
    }

    #[test]
    fn death_during_mod_loading_is_not_a_pass() {
        let t = Temp::new("died");
        let log = t.0.join("game.log");
        let mut cfg = Config::new(t.0.join("Config"), t.0.join("run"), log.clone());
        cfg.settle = Duration::from_millis(200);

        // Игра объявила версию и умерла, не догрузив моды.
        let cmd = fake_game(&log, "printf 'RimWorld 1.6.4633\\n' > '{log}'; exit 0");
        let mut run = TestRun::start(cmd, "fake".into(), cfg, &ids(&["a.mod"])).unwrap();

        assert_eq!(drive(&mut run, Duration::from_secs(10)), Verdict::DiedWhileLoading);
    }
}
