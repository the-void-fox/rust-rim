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
            Self::Auto => "Автоматически",
            Self::Native => "Нативный запуск",
            Self::Umu => "Proton (umu-run)",
            Self::Wine => "Wine",
            Self::Steam => "Через Steam",
            Self::Custom => "Своя команда",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
}

/// Зачем запускаем.
#[derive(Clone, Debug)]
pub enum Mode {
    /// Обычный запуск.
    Play,
    /// Прогон для автотеста сборки: игра стартует, генерирует карту
    /// и пишет лог в известное место.
    QuickTest { log_file: PathBuf },
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

    let windows_side = exe.needs_compat_layer() && !matches!(runner, Runner::Native);
    // Префикс нужен не только как переменная окружения: путь к логу
    // приходится переводить относительно него.
    let prefix = if windows_side { require_prefix(settings).ok() } else { None };
    let game_args = game_args(settings, mode, windows_side, prefix.as_deref());
    let cwd = game_path.to_path_buf();

    match runner {
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
            let proton = if settings.proton.trim().is_empty() {
                "GE-Proton".to_string()
            } else {
                settings.proton.trim().to_string()
            };
            env.push(("PROTONPATH".to_string(), proton));

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
    }
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
    if let Mode::QuickTest { log_file } = mode {
        args.push("-quicktest".to_string());
        args.push("-logfile".to_string());
        args.push(if windows_side {
            to_wine_path(log_file, prefix)
        } else {
            log_file.to_string_lossy().into_owned()
        });
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
fn require_program(name: &str) -> Result<(), LaunchError> {
    let found = std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(name).is_file())
        })
        .unwrap_or(false);
    if found {
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
        let plan = plan(&g.0, &settings(Runner::Native), &Mode::QuickTest { log_file: log.clone() })
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
        let Ok(plan) = plan(&g.0, &s, &Mode::QuickTest { log_file: "/tmp/x.log".into() }) else {
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
        let Ok(plan) = plan(&g.0, &s, &Mode::QuickTest { log_file: log }) else { return };
        let at = plan.args.iter().position(|a| a == "-logfile").unwrap();
        assert_eq!(plan.args[at + 1], "C:\\users\\x\\rustrim.log");
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
            &Mode::QuickTest { log_file: "/tmp/x.log".into() },
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
