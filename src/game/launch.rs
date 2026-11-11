//! Сборка команды запуска RimWorld.
//!
//! Способов запустить игру под Linux много и они несовместимы между собой:
//! нативная сборка, Proton через umu, обычный wine, Steam. У одного
//! пользователя их легко оказывается несколько, с разными префиксами.
//! Поэтому запуск описывается настраиваемым профилем, а не хардкодом.
//!
//! Функция [`plan`] только собирает команду и ничего не запускает — так её
//! можно проверить тестами целиком.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::paths::{self, Executable, APP_ID};
use crate::i18n::tr;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Runner {
    /// Определить по установке: нативная сборка — напрямую, Windows — umu.
    #[default]
    Auto,
    /// Нативный бинарник игры.
    Native,
    /// Proton через umu-run (вне Steam).
    Umu,
    /// Обычный wine.
    Wine,
    /// Через клиент Steam.
    Steam,
    /// Произвольная команда — например самописный скрипт запуска.
    Custom,
}

impl Runner {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => tr("Автоматически"),
            Self::Native => tr("Нативный запуск"),
            Self::Umu => "Proton (umu-run)",
            Self::Wine => "Wine",
            Self::Steam => tr("Через Steam"),
            Self::Custom => tr("Своя команда"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchSettings {
    pub runner: Runner,
    /// WINEPREFIX / STEAM_COMPAT_DATA_PATH. Пусто — определить самому.
    pub prefix: String,
    /// PROTONPATH: «GE-Proton» или полный путь к версии.
    pub proton: String,
    /// Команда для [`Runner::Custom`].
    pub custom_command: String,
    /// Дополнительные аргументы игры, через пробел.
    pub extra_args: String,
    /// Прогоны автотеста и поиска — без окна на экране, через cage.
    pub background_tests: bool,
    /// Сколько прогонов поиска виновника держать одновременно.
    pub parallel_runs: u8,
}

impl Default for LaunchSettings {
    fn default() -> Self {
        Self {
            runner: Runner::default(),
            prefix: String::new(),
            proton: String::new(),
            custom_command: String::new(),
            extra_args: String::new(),
            background_tests: true,
            parallel_runs: 2,
        }
    }
}

/// Больше трёх экземпляров игры одновременно не потянет память обычной
/// машины: каждый — от 2 ГБ на маленькой сборке.
pub const MAX_PARALLEL_RUNS: u8 = 3;

impl LaunchSettings {
    /// Сколько прогонов поиска держать одновременно, в разумных пределах.
    pub fn parallel(&self) -> usize {
        self.parallel_runs.clamp(1, MAX_PARALLEL_RUNS) as usize
    }
}

/// Зачем запускаем.
#[derive(Clone, Debug)]
pub enum Mode {
    /// Обычный запуск.
    Play,
    /// Прогон для автотеста сборки: игра стартует, генерирует карту
    /// и пишет лог в известное место.
    QuickTest {
        log_file: PathBuf,
        /// Своя папка данных прогона (`-savedatafolder`): там лежат
        /// тестируемый ModsConfig.xml и копия настроек игрока. Конфиг игрока
        /// при этом не трогается вовсе.
        save_data: Option<PathBuf>,
    },
}

impl Mode {
    fn is_test(&self) -> bool {
        matches!(self, Mode::QuickTest { .. })
    }
}

/// Готовая к исполнению команда.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
}

impl Plan {
    pub fn to_command(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(&self.program);
        cmd.args(&self.args).current_dir(&self.cwd);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd
    }

    /// Строка для показа пользователю и записи в лог.
    pub fn display(&self) -> String {
        let env: String = self.env.iter().map(|(k, v)| format!("{k}={v} ")).collect();
        format!("{env}{} {}", self.program, self.args.join(" "))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("в папке {0} не найдена игра (нужен RimWorldWin64.exe или RimWorldLinux)")]
    GameNotFound(PathBuf),
    #[error("не найден {0} — установите его или выберите другой способ запуска")]
    MissingProgram(String),
    #[error("для запуска Windows-сборки нужен префикс: укажите его в настройках")]
    NoPrefix,
    #[error("команда запуска не задана")]
    NoCustomCommand,
    #[error("через Steam нельзя прогнать автотест: клиент не отдаёт код возврата игры")]
    SteamCannotTest,
    #[error("в пути {0} есть «=»: RimWorld не разберёт такой -savedatafolder")]
    BadSaveDataPath(PathBuf),
}

/// Собирает команду запуска.
pub fn plan(
    game_path: &Path,
    settings: &LaunchSettings,
    mode: &Mode,
) -> Result<Plan, LaunchError> {
    let exe = paths::find_executable(game_path)
        .ok_or_else(|| LaunchError::GameNotFound(game_path.to_path_buf()))?;
    let runner = resolve_runner(settings.runner, &exe);

    if matches!(runner, Runner::Steam) && matches!(mode, Mode::QuickTest { .. }) {
        return Err(LaunchError::SteamCannotTest);
    }

    if let Mode::QuickTest { save_data: Some(dir), .. } = mode {
        // RimWorld делит аргумент по «=» и ждёт ровно две части.
        if dir.to_string_lossy().contains('=') {
            return Err(LaunchError::BadSaveDataPath(dir.clone()));
        }
    }

    let windows_side = exe.needs_compat_layer() && !matches!(runner, Runner::Native);
    // Префикс нужен не только как переменная окружения: путь к логу
    // приходится переводить относительно него.
    let prefix = if windows_side { require_prefix(settings).ok() } else { None };
    let game_args = game_args(settings, mode, windows_side, prefix.as_deref());
    let cwd = game_path.to_path_buf();

    let plan = match runner {
        Runner::Native | Runner::Auto => Ok(Plan {
            program: exe.path().to_string_lossy().into_owned(),
            args: game_args,
            env: Vec::new(),
            cwd,
        }),

        Runner::Umu => {
            require_program("umu-run")?;
            let prefix = prefix.ok_or(LaunchError::NoPrefix)?;
            let mut env = vec![
                ("WINEPREFIX".to_string(), prefix.to_string_lossy().into_owned()),
                // umu опознаёт игру по GAMEID и подбирает свои правки.
                ("GAMEID".to_string(), format!("umu-{APP_ID}")),
            ];
            env.push(("PROTONPATH".to_string(), proton_path(settings, mode)));
            if mode.is_test() {
                // Без этого umu на каждом старте сверяет рантайм с сервером.
                // Если рантайма ещё нет, он всё равно установится.
                env.push(("UMU_RUNTIME_UPDATE".to_string(), "0".to_string()));
            }

            let mut args = vec![exe.path().to_string_lossy().into_owned()];
            args.extend(game_args);
            Ok(Plan { program: "umu-run".to_string(), args, env, cwd })
        }

        Runner::Wine => {
            require_program("wine")?;
            let prefix = prefix.ok_or(LaunchError::NoPrefix)?;
            let mut args = vec![exe.path().to_string_lossy().into_owned()];
            args.extend(game_args);
            Ok(Plan {
                program: "wine".to_string(),
                args,
                env: vec![("WINEPREFIX".to_string(), prefix.to_string_lossy().into_owned())],
                cwd,
            })
        }

        Runner::Steam => {
            require_program("steam")?;
            let mut args = vec!["-applaunch".to_string(), APP_ID.to_string()];
            args.extend(game_args);
            Ok(Plan { program: "steam".to_string(), args, env: Vec::new(), cwd })
        }

        Runner::Custom => {
            let mut parts = settings.custom_command.split_whitespace().map(str::to_string);
            let program = parts.next().ok_or(LaunchError::NoCustomCommand)?;
            let mut args: Vec<String> = parts.collect();
            args.extend(game_args);
            Ok(Plan { program, args, env: Vec::new(), cwd })
        }
    }?;

    if mode.is_test() && settings.background_tests && runner == Runner::Umu && has_program("cage") {
        return Ok(in_background(plan));
    }
    Ok(plan)
}

/// Есть ли cage — без него прогоны идут с обычным окном.
pub fn background_available() -> bool {
    has_program("cage")
}

/// Заворачивает команду в cage с headless-выводом: игра рисует на настоящей
/// видеокарте, но в невидимый экран, и окна на рабочем столе нет.
///
/// Так, а не флагами Unity: `-batchmode` и `-nographics` ломают шейдеры и
/// дают ложные NullReferenceException. И не скрытым окном: невидимому окну
/// Xwayland не отдаёт кадры, игра встаёт, и тишина в логе засчитывается как
/// успех. Проверено на живых прогонах: внутри cage — те же 60 тиков в секунду,
/// что у видимого окна, и те же ошибки.
fn in_background(plan: Plan) -> Plan {
    let mut args = vec!["--".to_string(), plan.program];
    args.extend(plan.args);
    let mut env = plan.env;
    for (k, v) in [
        ("WLR_BACKENDS", "headless"),
        ("WLR_LIBINPUT_NO_DEVICES", "1"),
        ("WLR_RENDERER", "gles2"),
        // Wayland-драйвер Wine: через Xwayland кадры отдаются медленнее,
        // и игра шла 25 тиков в секунду вместо 60.
        ("PROTON_ENABLE_WAYLAND", "1"),
        // Не ждать вертикальной синхронизации от невидимого экрана,
        // но и не жечь видеокарту сотнями кадров.
        ("DXVK_CONFIG", "dxgi.syncInterval = 0"),
        ("DXVK_FRAME_RATE", "60"),
    ] {
        env.push((k.to_string(), v.to_string()));
    }
    Plan { program: "cage".to_string(), args, env, cwd: plan.cwd }
}

/// Что подставить в PROTONPATH.
///
/// «GE-Proton» для umu значит «сходить на GitHub за последней версией». Для
/// прогона это лишние 11 секунд на каждом запуске и версия Proton, которая
/// может смениться посреди серии прогонов. Поэтому прогону, если версия не
/// задана явно, достаётся самая свежая уже установленная GE-Proton.
fn proton_path(settings: &LaunchSettings, mode: &Mode) -> String {
    let configured = settings.proton.trim();
    let wants_latest = configured.is_empty() || configured == "GE-Proton";
    if wants_latest && mode.is_test() {
        if let Some(path) = paths::latest_ge_proton() {
            return path.to_string_lossy().into_owned();
        }
    }
    if configured.is_empty() { "GE-Proton".to_string() } else { configured.to_string() }
}

/// `Auto` превращается в конкретный способ по тому, что лежит в папке игры.
fn resolve_runner(requested: Runner, exe: &Executable) -> Runner {
    if requested != Runner::Auto {
        return requested;
    }
    if exe.needs_compat_layer() {
        Runner::Umu
    } else {
        Runner::Native
    }
}

fn require_prefix(settings: &LaunchSettings) -> Result<PathBuf, LaunchError> {
    let configured = settings.prefix.trim();
    if !configured.is_empty() {
        return Ok(PathBuf::from(configured));
    }
    paths::find_prefixes()
        .into_iter()
        .next()
        .map(|p| p.path)
        .ok_or(LaunchError::NoPrefix)
}

fn game_args(
    settings: &LaunchSettings,
    mode: &Mode,
    windows_side: bool,
    prefix: Option<&Path>,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if let Mode::QuickTest { log_file, save_data } = mode {
        let game_path = |p: &Path| {
            if windows_side { to_wine_path(p, prefix) } else { p.to_string_lossy().into_owned() }
        };
        args.push("-quicktest".to_string());
        args.push("-logfile".to_string());
        args.push(game_path(log_file));
        if let Some(dir) = save_data {
            args.push(format!("-savedatafolder={}", game_path(dir)));
        }
    }
    args.extend(settings.extra_args.split_whitespace().map(str::to_string));
    args
}

/// Переводит путь хоста в путь, понятный процессу внутри префикса.
///
/// Всё, что лежит в `drive_c` префикса, отображается на диск `C:` — такие
/// файлы одинаково видны и игре, и нам. Остальное отображается на `Z:`
/// (wine вешает туда корень файловой системы), но полагаться на это
/// нельзя: umu запускает игру внутри контейнера pressure-vessel, где,
/// например, `/tmp` свой собственный. Лог, записанный в `Z:\tmp`, на хосте
/// просто не появляется — проверено.
pub fn to_wine_path(path: &Path, prefix: Option<&Path>) -> String {
    if let Some(prefix) = prefix {
        let drive_c = prefix.join("drive_c");
        if let Ok(rest) = path.strip_prefix(&drive_c) {
            return format!("C:\\{}", rest.to_string_lossy().replace('/', "\\"));
        }
    }
    let text = path.to_string_lossy();
    if text.starts_with('/') {
        format!("Z:{}", text.replace('/', "\\"))
    } else {
        text.into_owned()
    }
}

/// Есть ли программа в PATH.
fn has_program(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

fn require_program(name: &str) -> Result<(), LaunchError> {
    if has_program(name) {
        Ok(())
    } else {
        Err(LaunchError::MissingProgram(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Game(PathBuf);

    impl Game {
        /// Папка с Windows-сборкой (как у DRM-free копии).
        fn windows(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("rustrim_launch_{}_{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("RimWorldWin64_Data")).unwrap();
            std::fs::write(dir.join("RimWorldWin64.exe"), "MZ").unwrap();
            Self(dir)
        }

        fn native(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("rustrim_launch_{}_{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("RimWorldLinux_Data")).unwrap();
            std::fs::write(dir.join("RimWorldLinux"), "#!/bin/sh").unwrap();
            Self(dir)
        }
    }

    impl Drop for Game {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn settings(runner: Runner) -> LaunchSettings {
        LaunchSettings {
            runner,
            prefix: "/prefixes/rimworld".to_string(),
            ..Default::default()
        }
    }

    fn env_of(plan: &Plan, key: &str) -> Option<String> {
        plan.env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn native_build_runs_directly() {
        let g = Game::native("native");
        let plan = plan(&g.0, &settings(Runner::Auto), &Mode::Play).unwrap();

        assert_eq!(plan.program, g.0.join("RimWorldLinux").to_string_lossy());
        assert!(plan.env.is_empty(), "нативному запуску префикс не нужен");
        assert_eq!(plan.cwd, g.0);
    }

    #[test]
    fn windows_build_defaults_to_umu() {
        let g = Game::windows("auto");
        let Ok(plan) = plan(&g.0, &settings(Runner::Auto), &Mode::Play) else {
            // umu-run может быть не установлен в окружении сборки.
            return;
        };
        assert_eq!(plan.program, "umu-run");
        assert_eq!(plan.args[0], g.0.join("RimWorldWin64.exe").to_string_lossy());
        assert_eq!(env_of(&plan, "GAMEID").as_deref(), Some("umu-294100"));
        assert_eq!(env_of(&plan, "WINEPREFIX").as_deref(), Some("/prefixes/rimworld"));
        assert_eq!(env_of(&plan, "PROTONPATH").as_deref(), Some("GE-Proton"));
    }

    #[test]
    fn explicit_proton_version_wins() {
        let g = Game::windows("proton");
        let mut s = settings(Runner::Umu);
        s.proton = "/opt/GE-Proton10-34".to_string();
        let Ok(plan) = plan(&g.0, &s, &Mode::Play) else { return };
        assert_eq!(env_of(&plan, "PROTONPATH").as_deref(), Some("/opt/GE-Proton10-34"));
    }

    #[test]
    fn quicktest_adds_flags_and_log() {
        let g = Game::native("quicktest");
        let log = PathBuf::from("/tmp/rustrim/test.log");
        let plan = plan(&g.0, &settings(Runner::Native), &Mode::QuickTest { log_file: log.clone(), save_data: None })
            .unwrap();

        assert!(plan.args.contains(&"-quicktest".to_string()), "{:?}", plan.args);
        let at = plan.args.iter().position(|a| a == "-logfile").expect("нет -logfile");
        assert_eq!(plan.args[at + 1], "/tmp/rustrim/test.log");
    }

    #[test]
    fn log_path_for_windows_side_is_translated() {
        // Под Proton «/tmp/x.log» игра поняла бы как путь внутри drive_c.
        let g = Game::windows("logpath");
        let mut s = settings(Runner::Wine);
        s.prefix = "/prefixes/rimworld".into();
        let Ok(plan) = plan(&g.0, &s, &Mode::QuickTest { log_file: "/tmp/x.log".into(), save_data: None }) else {
            return;
        };
        let at = plan.args.iter().position(|a| a == "-logfile").unwrap();
        assert_eq!(plan.args[at + 1], "Z:\\tmp\\x.log");
    }

    #[test]
    fn quicktest_log_inside_prefix_uses_drive_c() {
        let g = Game::windows("logprefix");
        let mut s = settings(Runner::Wine);
        s.prefix = "/prefixes/rimworld".into();
        let log = PathBuf::from("/prefixes/rimworld/drive_c/users/x/rustrim.log");
        let Ok(plan) = plan(&g.0, &s, &Mode::QuickTest { log_file: log, save_data: None }) else { return };
        let at = plan.args.iter().position(|a| a == "-logfile").unwrap();
        assert_eq!(plan.args[at + 1], "C:\\users\\x\\rustrim.log");
    }

    #[test]
    fn quicktest_gets_its_own_save_data_folder() {
        let g = Game::native("savedata");
        let mode = Mode::QuickTest {
            log_file: "/data/RustRim/test/quicktest.log".into(),
            save_data: Some("/data/RustRim/test".into()),
        };
        let plan = plan(&g.0, &settings(Runner::Native), &mode).unwrap();
        // Одним аргументом через «=»: так его ищет GenCommandLine.
        assert!(plan.args.contains(&"-savedatafolder=/data/RustRim/test".to_string()), "{:?}", plan.args);
    }

    #[test]
    fn save_data_inside_the_prefix_uses_drive_c() {
        let g = Game::windows("savedata_win");
        let mut s = settings(Runner::Wine);
        s.prefix = "/prefixes/rimworld".into();
        let mode = Mode::QuickTest {
            log_file: "/prefixes/rimworld/drive_c/users/x/RustRim/test/quicktest.log".into(),
            save_data: Some("/prefixes/rimworld/drive_c/users/x/RustRim/test".into()),
        };
        let Ok(plan) = plan(&g.0, &s, &mode) else { return };
        assert!(
            plan.args.contains(&"-savedatafolder=C:\\users\\x\\RustRim\\test".to_string()),
            "{:?}",
            plan.args,
        );
    }

    #[test]
    fn save_data_path_with_equals_sign_is_rejected() {
        let g = Game::native("savedata_eq");
        let mode = Mode::QuickTest { log_file: "/tmp/x.log".into(), save_data: Some("/a=b/test".into()) };
        let err = plan(&g.0, &settings(Runner::Native), &mode);
        assert!(matches!(err, Err(LaunchError::BadSaveDataPath(_))), "{err:?}");
    }

    #[test]
    fn test_runs_do_not_go_online_for_proton() {
        let g = Game::windows("offline");
        let mode = Mode::QuickTest { log_file: "/tmp/x.log".into(), save_data: None };
        let Ok(test) = plan(&g.0, &settings(Runner::Umu), &mode) else { return };
        assert_eq!(env_of(&test, "UMU_RUNTIME_UPDATE").as_deref(), Some("0"));
        // Установленная версия — полным путём; «GE-Proton» остаётся, только
        // если ставить нечего и umu всё равно придётся скачать её самому.
        let proton = env_of(&test, "PROTONPATH").unwrap();
        assert!(proton == "GE-Proton" || proton.starts_with('/'), "{proton}");

        // Обычный запуск остаётся как был.
        let play = plan(&g.0, &settings(Runner::Umu), &Mode::Play).unwrap();
        assert_eq!(env_of(&play, "UMU_RUNTIME_UPDATE"), None);
        assert_eq!(env_of(&play, "PROTONPATH").as_deref(), Some("GE-Proton"));
    }

    #[test]
    fn background_tests_are_wrapped_in_cage() {
        let g = Game::windows("cage");
        let mode = Mode::QuickTest { log_file: "/tmp/x.log".into(), save_data: None };
        let Ok(plan) = plan(&g.0, &settings(Runner::Umu), &mode) else { return };
        if !background_available() {
            // Без cage — обычный запуск с окном, а не ошибка.
            assert_eq!(plan.program, "umu-run");
            return;
        }
        assert_eq!(plan.program, "cage");
        assert_eq!(plan.args[..2], ["--".to_string(), "umu-run".to_string()]);
        assert_eq!(env_of(&plan, "WLR_BACKENDS").as_deref(), Some("headless"));
        assert_eq!(env_of(&plan, "PROTON_ENABLE_WAYLAND").as_deref(), Some("1"));
        // Префикс и Proton на месте.
        assert!(env_of(&plan, "WINEPREFIX").is_some());

        // Обычный запуск игры — всегда с окном.
        let play = super::plan(&g.0, &settings(Runner::Umu), &Mode::Play).unwrap();
        assert_eq!(play.program, "umu-run");
        // И выключенная настройка — тоже.
        let mut s = settings(Runner::Umu);
        s.background_tests = false;
        assert_eq!(super::plan(&g.0, &s, &mode).unwrap().program, "umu-run");
    }

    #[test]
    fn explicit_proton_is_kept_for_test_runs_too() {
        let g = Game::windows("explicit_test");
        let mut s = settings(Runner::Umu);
        s.proton = "/opt/GE-Proton10-34".to_string();
        let mode = Mode::QuickTest { log_file: "/tmp/x.log".into(), save_data: None };
        let Ok(plan) = plan(&g.0, &s, &mode) else { return };
        assert_eq!(env_of(&plan, "PROTONPATH").as_deref(), Some("/opt/GE-Proton10-34"));
    }

    #[test]
    fn paths_inside_the_prefix_map_to_drive_c() {
        // Только такие файлы гарантированно видны и игре, и нам: под umu
        // игра работает в контейнере со своим /tmp.
        let prefix = Path::new("/prefixes/rimworld");
        assert_eq!(
            to_wine_path(Path::new("/prefixes/rimworld/drive_c/users/x/test.log"), Some(prefix)),
            "C:\\users\\x\\test.log",
        );
    }

    #[test]
    fn paths_outside_the_prefix_fall_back_to_z_drive() {
        let prefix = Path::new("/prefixes/rimworld");
        assert_eq!(to_wine_path(Path::new("/tmp/a b/c.log"), Some(prefix)), "Z:\\tmp\\a b\\c.log");
        assert_eq!(to_wine_path(Path::new("/tmp/x.log"), None), "Z:\\tmp\\x.log");
        assert_eq!(to_wine_path(Path::new("relative.log"), None), "relative.log");
    }

    #[test]
    fn extra_args_are_appended() {
        let g = Game::native("extra");
        let mut s = settings(Runner::Native);
        s.extra_args = "-popupwindow -force-glcore".to_string();
        let plan = plan(&g.0, &s, &Mode::Play).unwrap();
        assert_eq!(plan.args, ["-popupwindow", "-force-glcore"]);
    }

    #[test]
    fn steam_cannot_run_the_test() {
        // Клиент Steam отвязывает процесс игры — ни кода возврата,
        // ни возможности прибить её по таймауту.
        let g = Game::native("steam");
        let err = plan(
            &g.0,
            &settings(Runner::Steam),
            &Mode::QuickTest { log_file: "/tmp/x.log".into(), save_data: None },
        );
        assert!(matches!(err, Err(LaunchError::SteamCannotTest)), "{err:?}");
    }

    #[test]
    fn custom_command_is_split_into_program_and_args() {
        let g = Game::native("custom");
        let mut s = settings(Runner::Custom);
        s.custom_command = "gamescope -f -- umu-run".to_string();
        s.extra_args = "-popupwindow".to_string();
        let plan = plan(&g.0, &s, &Mode::Play).unwrap();

        assert_eq!(plan.program, "gamescope");
        assert_eq!(plan.args, ["-f", "--", "umu-run", "-popupwindow"]);
    }

    #[test]
    fn missing_game_is_reported() {
        let dir = std::env::temp_dir().join("rustrim_launch_missing");
        let _ = std::fs::create_dir_all(&dir);
        let err = plan(&dir, &settings(Runner::Auto), &Mode::Play);
        assert!(matches!(err, Err(LaunchError::GameNotFound(_))), "{err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
