//! Всё, что запускает игру: обычный запуск, прогон сборки и поиск виновника.
//!
//! У всех трёх одна и та же подготовка — найти wine-префикс, собрать план
//! запуска, прибрать за собой процессы. Раньше и подготовка, и состояние всех
//! трёх лежали в корне приложения: семь полей и девять методов вперемешку с
//! каталогом модов и раскладкой. Здесь они вместе, а корню остаётся вызвать
//! [`GameSession::show`] и разобрать ответ.

use egui::Context;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::bisect::Hunt;
use crate::game::teardown::Owner;
use crate::game::{launch, paths, workers, Prefix};
use crate::job::Job;
use crate::mod_data::{ModDb, ModId, Profile};
use crate::process::Run;
use crate::settings::AppSettings;
use crate::testing::{self, Phase, TestRun};
use crate::ui::bisect_panel::{self, BisectUi};
use crate::ui::launch_panel;
use crate::ui::test_panel::{self, TestUi};
use crate::i18n::tr;
use crate::trf;

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
    /// Сборка, с которой запущен этот прогон; забирается при успехе.
    tested: Option<Vec<ModId>>,

    /// Поиск виновника: цепочка прогонов с урезанными сборками.
    hunt: Option<Hunt>,
    hunt_ui: BisectUi,

    /// Копии префикса для параллельных прогонов — по одной на рабочее
    /// место, начиная со второго. Готовятся в фоне.
    worker_prefixes: Vec<PathBuf>,
    cloning: Job<Vec<PathBuf>>,
}

/// Метки прогонов: по ним снимаются процессы ровно одного прогона.
static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_tag() -> String {
    format!("{}-{}", std::process::id(), RUN_COUNTER.fetch_add(1, Ordering::Relaxed))
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
            Err(e) => Err(trf!("Не удалось запустить: {e}\n\n{command}", e, command)),
        }
    }

    // ── Прогон сборки ────────────────────────────────────────────────────────

    pub fn start_test(&mut self, settings: &AppSettings, active: &[ModId]) -> Result<(), String> {
        // Поиск занимает ту же папку прогона и тот же префикс.
        if self.hunt.as_ref().is_some_and(|h| !h.is_done()) {
            return Err(tr("Идёт поиск виновника — дождитесь его конца или остановите.").to_string());
        }
        // Недоигранный прошлый тест снимается: его игра ещё может работать.
        if let Some(previous) = &mut self.run {
            previous.cancel();
        }
        let run = self.build_run(settings, active, 0)?;
        self.run = Some(run);
        self.tested = Some(active.to_vec());
        self.test_ui.reset();
        Ok(())
    }

    /// Сборка, на которой тест только что прошёл. Отдаётся один раз: по ней
    /// приложение запоминает рабочую сборку.
    pub fn take_passed(&mut self) -> Option<Vec<ModId>> {
        use crate::testing::{Phase, Verdict};
        let passed = self
            .run
            .as_ref()
            .is_some_and(|run| matches!(run.phase(), Phase::Done(Verdict::Passed)));
        if passed { self.tested.take() } else { None }
    }

    /// Провалился ли последний тест — только тогда есть что сужать.
    pub fn can_hunt(&self) -> bool {
        use crate::testing::{Phase, Verdict};
        self.hunt.as_ref().is_none_or(|h| h.is_done())
            && self.run.as_ref().is_some_and(|run| {
                matches!(run.phase(), Phase::Done(v) if !matches!(v, Verdict::Passed | Verdict::Cancelled))
            })
    }

    /// Запускает поиск виновника по разбору последнего прогона.
    pub fn start_hunt(&mut self, db: &ModDb, profile: &Profile, settings: &AppSettings) {
        let issues = self.test_ui.issues().to_vec();
        let verdict = match self.run.as_ref().map(TestRun::phase) {
            Some(Phase::Done(v)) => v.clone(),
            _ => crate::testing::Verdict::LoadedWithErrors,
        };
        self.hunt = Some(Hunt::from_failed_run(db, profile, &verdict, &issues));
        self.hunt_ui.open = true;
        self.prepare_workers(settings);
    }

    /// Нужны ли рабочим местам свои префиксы: только Windows-сборке под
    /// Proton/wine — нативная игра префикса не имеет.
    fn needs_prefix(settings: &AppSettings) -> bool {
        let game = Path::new(&settings.game_path);
        paths::find_executable(game).is_some_and(|exe| exe.needs_compat_layer())
            && settings.launch.runner != launch::Runner::Native
    }

    /// Начинает готовить копии префикса для параллельных прогонов, если
    /// их ещё нет. Поиск тем временем идёт в одном рабочем месте.
    fn prepare_workers(&mut self, settings: &AppSettings) {
        let wanted = settings.launch.parallel();
        if wanted < 2 || !Self::needs_prefix(settings) || matches!(self.cloning, Job::Running(_)) {
            return;
        }
        let Some(main) = self.prefix(settings) else { return };
        let dirs: Vec<PathBuf> =
            (1..wanted).filter_map(|w| workers::clone_dir(&main, w)).collect();
        if dirs.len() == self.worker_prefixes.len()
            && dirs.iter().all(|d| workers::is_ready(&main, d))
        {
            return;
        }
        self.worker_prefixes.clear();
        self.cloning = Job::spawn(move || {
            for dir in &dirs {
                workers::ensure_clone(&main, dir)
                    .map_err(|e| trf!("Не удалось подготовить префикс {dir}: {e}", dir = dir.display(), e))?;
            }
            Ok(dirs)
        });
    }

    /// Сколько прогонов поиска можно держать одновременно прямо сейчас.
    fn available_workers(&mut self, settings: &AppSettings) -> usize {
        if self.cloning.poll() {
            match std::mem::take(&mut self.cloning) {
                Job::Done(dirs) => self.worker_prefixes = dirs,
                Job::Failed(e) => tracing::warn!("Prefix copy failed: {e}"),
                other => self.cloning = other,
            }
        }
        let wanted = settings.launch.parallel();
        if Self::needs_prefix(settings) {
            wanted.min(1 + self.worker_prefixes.len())
        } else {
            wanted
        }
    }

    /// Готовит прогон произвольного набора модов в рабочем месте `worker`.
    ///
    /// Общее для обычного теста и для поиска виновника: настройки, план
    /// запуска и уборка процессов у них одни и те же, разный только состав.
    /// У каждого рабочего места свой префикс и своя папка данных.
    fn build_run(&mut self, settings: &AppSettings, active: &[ModId], worker: usize) -> Result<TestRun, String> {
        if settings.config_path.is_empty() {
            return Err(
                tr("Не задан путь к конфигу игры — прогону некуда положить сборку.").to_string()
            );
        }
        let windows_side = Self::needs_prefix(settings);
        let prefix = match worker {
            0 => self.prefix(settings),
            n => self.worker_prefixes.get(n - 1).cloned(),
        };
        if windows_side && prefix.is_none() {
            return Err(tr("Не найден wine-префикс: укажите его в Настройки → Запуск.").to_string());
        }

        let game = Path::new(&settings.game_path);
        let config_dir = PathBuf::from(&settings.config_path);
        // Своя папка данных прогона — рядом с Player.log, в папке данных
        // игры: под umu игра работает в контейнере со своим /tmp, и папку
        // на произвольном пути хост не увидит. Лог — туда же. Второе и
        // следующие рабочие места живут в своих копиях префикса — их папка
        // лежит там же, на диске C: копии.
        let data_dir = config_dir.parent().unwrap_or(&config_dir);
        let save_data = match (worker, &prefix) {
            (0, _) => data_dir.join("RustRim").join("test"),
            (_, Some(clone)) if windows_side => clone.join("drive_c").join("rustrim").join("test"),
            (n, _) => data_dir.join("RustRim").join(format!("test-{}", n + 1)),
        };
        let log_file = save_data.join("quicktest.log");

        let mut launch_settings = settings.launch.clone();
        if let Some(prefix) = &prefix {
            launch_settings.prefix = prefix.to_string_lossy().into_owned();
        }
        let mode = launch::Mode::QuickTest {
            log_file: log_file.clone(),
            save_data: Some(save_data.clone()),
        };
        let plan = launch::plan(game, &launch_settings, &mode).map_err(|e| e.to_string())?;

        let mut config = testing::Config::new(config_dir, save_data, log_file);
        // Своя метка у каждого прогона: путь к игре у параллельных прогонов
        // общий, и по нему один снял бы и другой, и игру самого игрока.
        config.cleanup = Some(Owner::Tag(next_tag()));

        TestRun::start(plan.to_command(), plan.display(), config, active)
            .map_err(|e| trf!("Не удалось запустить прогон: {e}", e))
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
            Reply::None => self.show_hunt(ctx, db, settings),
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
                self.start_hunt(db, profile, settings);
                Reply::None
            }
            test_panel::Reply::Select(id) => Reply::Select(id),
        }
    }

    fn show_hunt(&mut self, ctx: &Context, db: &ModDb, settings: &AppSettings) -> Reply {
        let mut failure = None;
        // Поиск на время работы вынимается: чтобы запустить очередной прогон,
        // ему нужен build_run, а тот забирает сессию целиком.
        if let Some(mut hunt) = self.hunt.take() {
            if !hunt.is_done() {
                hunt.set_workers(self.available_workers(settings));
            }
            hunt.poll();
            while let Some(request) = hunt.wants_run() {
                match self.build_run(settings, &request.active, request.worker) {
                    Ok(run) => hunt.attach(run),
                    Err(e) => {
                        failure = Some(e);
                        hunt.cancel();
                        break;
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

        let preparing = matches!(self.cloning, Job::Running(_));
        match bisect_panel::show(ctx, &mut self.hunt_ui, self.hunt.as_ref(), db, preparing) {
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
