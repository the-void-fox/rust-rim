//! Всё, что запускает игру: обычный запуск, прогон сборки и поиск виновника.
//!
//! У всех трёх одна и та же подготовка — найти wine-префикс, собрать план
//! запуска, прибрать за собой процессы. Раньше и подготовка, и состояние всех
//! трёх лежали в корне приложения: семь полей и девять методов вперемешку с
//! каталогом модов и раскладкой. Здесь они вместе, а корню остаётся вызвать
//! [`GameSession::show`] и разобрать ответ.

use egui::Context;

use crate::bisect::Hunt;
use crate::game::{launch, paths, Prefix};
use crate::mod_data::{ModDb, ModId, Profile};
use crate::process::Run;
use crate::settings::AppSettings;
use crate::testing::{self, Phase, TestRun};
use crate::ui::bisect_panel::{self, BisectUi};
use crate::ui::launch_panel;
use crate::ui::test_panel::{self, TestUi};

/// О чём сессия просит приложение.
pub enum Reply {
    None,
    /// Выделить мод в списке.
    Select(ModId),
    /// Выключить моды в сборке (результат поиска виновника).
    Deactivate(Vec<ModId>),
    /// Показать сообщение об ошибке.
    Failed(String),
}

#[derive(Default)]
pub struct GameSession {
    /// Найденные wine-префиксы. Поиск лезет в несколько папок, поэтому
    /// результат держится до следующего явного обновления.
    prefixes: Vec<Prefix>,

    /// Обычный запуск игры и её живой вывод.
    game: Option<Run>,
    game_window: bool,

    /// Прогон сборки через -quicktest.
    run: Option<TestRun>,
    test_ui: TestUi,

    /// Поиск виновника: цепочка прогонов с урезанными сборками.
    hunt: Option<Hunt>,
    hunt_ui: BisectUi,
}

impl GameSession {
    /// Перечитывает список префиксов — при открытии настроек.
    pub fn refresh_prefixes(&mut self) {
        self.prefixes = paths::find_prefixes();
    }

    pub fn prefixes(&self) -> &[Prefix] {
        &self.prefixes
    }

    /// Открывает окно прогона, даже если прогона ещё не было: там объяснено,
    /// что вообще делает кнопка.
    pub fn open_test_window(&mut self) {
        self.test_ui.open = true;
    }

    // ── Обычный запуск ───────────────────────────────────────────────────────

    pub fn launch(&mut self, settings: &AppSettings) -> Result<(), String> {
        let mut effective = settings.launch.clone();
        if effective.prefix.trim().is_empty() {
            // Настройки могли не открывать ни разу — ищем префикс сейчас.
            if let Some(prefix) = self.prefix(settings) {
                effective.prefix = prefix.to_string_lossy().into_owned();
            }
        }

        let game = std::path::Path::new(&settings.game_path);
        let plan = launch::plan(game, &effective, &launch::Mode::Play)
            .map_err(|e| e.to_string())?;

        tracing::info!("Launching game: {}", plan.display());
        let command = plan.display();
        match Run::spawn(plan.to_command(), command.clone()) {
            Ok(run) => {
                self.game = Some(run);
                self.game_window = true;
                Ok(())
            }
            Err(e) => Err(format!("Не удалось запустить: {e}\n\n{command}")),
        }
    }

    // ── Прогон сборки ────────────────────────────────────────────────────────

    pub fn start_test(&mut self, settings: &AppSettings, active: &[ModId]) -> Result<(), String> {
        let run = self.build_run(settings, active)?;
        self.run = Some(run);
        self.test_ui.reset();
        Ok(())
    }

    /// Запускает поиск виновника по разбору последнего прогона.
    pub fn start_hunt(&mut self, db: &ModDb, profile: &Profile) {
        let issues = self.test_ui.issues().to_vec();
        self.hunt = Some(Hunt::from_failed_run(db, profile, &issues));
        self.hunt_ui.open = true;
    }

    /// Готовит прогон произвольного набора модов.
    ///
    /// Общее для обычного теста и для поиска виновника: настройки, план
    /// запуска и уборка процессов у них одни и те же, разный только состав.
    fn build_run(&mut self, settings: &AppSettings, active: &[ModId]) -> Result<TestRun, String> {
        let Some(prefix) = self.prefix(settings) else {
            return Err("Не найден wine-префикс: укажите его в Настройки → Запуск.".to_string());
        };
        if settings.config_path.is_empty() {
            return Err(
                "Не задан путь к конфигу игры — прогону некуда положить сборку.".to_string()
            );
        }

        let game = std::path::Path::new(&settings.game_path);
        let config_dir = std::path::PathBuf::from(&settings.config_path);
        // Лог кладём рядом с Player.log: под umu игра работает в контейнере
        // со своим /tmp, и файл на произвольном пути хосту не виден.
        let log_file = config_dir
            .parent()
            .unwrap_or(&config_dir)
            .join("rustrim-quicktest.log");

        let mut launch_settings = settings.launch.clone();
        launch_settings.prefix = prefix.to_string_lossy().into_owned();
        let mode = launch::Mode::QuickTest { log_file: log_file.clone() };
        let plan = launch::plan(game, &launch_settings, &mode).map_err(|e| e.to_string())?;

        let mut config = testing::Config::new(config_dir, log_file);
        if let Some(exe) = paths::find_executable(game) {
            config.cleanup = Some((prefix, exe.path().to_path_buf()));
        }

        TestRun::start(plan.to_command(), plan.display(), config, active)
            .map_err(|e| format!("Не удалось запустить прогон: {e}"))
    }

    /// Префикс из настроек, иначе первый найденный.
    fn prefix(&mut self, settings: &AppSettings) -> Option<std::path::PathBuf> {
        let configured = settings.launch.prefix.trim();
        if !configured.is_empty() {
            return Some(std::path::PathBuf::from(configured));
        }
        if self.prefixes.is_empty() {
            self.prefixes = paths::find_prefixes();
        }
        self.prefixes.first().map(|p| p.path.clone())
    }

    // ── Отрисовка ────────────────────────────────────────────────────────────

    /// Двигает все три окна за один кадр.
    pub fn show(
        &mut self,
        ctx: &Context,
        db: &ModDb,
        profile: &Profile,
        settings: &AppSettings,
    ) -> Reply {
        self.show_launch(ctx);
        let reply = self.show_test(ctx, db, profile, settings);
        match reply {
            Reply::None => self.show_hunt(ctx, db, profile, settings),
            other => other,
        }
    }

    /// Живой вывод запуска: без него Proton молчит десятки секунд, и
    /// непонятно, запускается игра или нет.
    fn show_launch(&mut self, ctx: &Context) {
        let Some(run) = &mut self.game else { return };
        let finished_now = run.poll();
        if run.is_running() || finished_now {
            // Пока идёт вывод, окно обновляется само.
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        if launch_panel::show(ctx, &mut self.game_window, run) == launch_panel::Reply::Kill {
            run.kill();
        }
    }

    fn show_test(
        &mut self,
        ctx: &Context,
        db: &ModDb,
        profile: &Profile,
        settings: &AppSettings,
    ) -> Reply {
        if let Some(run) = &mut self.run {
            if matches!(run.poll(), Phase::Done(_)) {
                // Разбор лога делается один раз, а не каждый кадр.
                if !self.test_ui.issues_ready() {
                    let issues = run.issues(db, profile);
                    self.test_ui.set_issues(issues);
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
        }

        match test_panel::show(ctx, &mut self.test_ui, self.run.as_ref()) {
            test_panel::Reply::None => Reply::None,
            test_panel::Reply::Cancel => {
                if let Some(run) = &mut self.run {
                    run.cancel();
                }
                Reply::None
            }
            test_panel::Reply::Restart => {
                let active = profile.order().to_vec();
                match self.start_test(settings, &active) {
                    Ok(()) => Reply::None,
                    Err(e) => Reply::Failed(e),
                }
            }
            test_panel::Reply::Bisect => {
                self.start_hunt(db, profile);
                Reply::None
            }
            test_panel::Reply::Select(id) => Reply::Select(id),
        }
    }

    fn show_hunt(
        &mut self,
        ctx: &Context,
        db: &ModDb,
        profile: &Profile,
        settings: &AppSettings,
    ) -> Reply {
        let mut failure = None;
        // Поиск на время работы вынимается: чтобы запустить очередной прогон,
        // ему нужен build_run, а тот забирает сессию целиком.
        if let Some(mut hunt) = self.hunt.take() {
            hunt.poll(db, profile);
            if let Some(active) = hunt.wants_run() {
                match self.build_run(settings, &active) {
                    Ok(run) => hunt.attach(run),
                    Err(e) => {
                        failure = Some(e);
                        hunt.cancel();
                    }
                }
            }
            if !hunt.is_done() {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
            self.hunt = Some(hunt);
        }
        if let Some(e) = failure {
            return Reply::Failed(e);
        }

        match bisect_panel::show(ctx, &mut self.hunt_ui, self.hunt.as_ref(), db) {
            bisect_panel::Reply::None => Reply::None,
            bisect_panel::Reply::Cancel => {
                if let Some(hunt) = &mut self.hunt {
                    hunt.cancel();
                }
                Reply::None
            }
            bisect_panel::Reply::Select(id) => Reply::Select(id),
            bisect_panel::Reply::Deactivate(ids) => Reply::Deactivate(ids),
        }
    }
}
