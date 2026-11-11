//! Гоняет игру по наборам, которые выдаёт [`super::search`].
//!
//! Сам процессы не запускает: собрать команду умеет только приложение, где
//! лежат настройки и пути. Драйвер говорит, какой набор нужен и в каком
//! рабочем месте, получает готовый прогон и превращает его результат в ответ
//! «воспроизвелось / нет».
//!
//! Рабочих мест может быть несколько — тогда наборы гоняются параллельно:
//! пока идёт тот, что нужен поиску прямо сейчас, рядом идёт следующий по
//! плану (см. [`Search::plan`]). Проверено на живой сборке: два экземпляра
//! игры укладываются в 74 с против 136 подряд.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::log_analysis::{parse_log, LogIssue};
use crate::mod_data::{ModDb, ModId, Profile};
use crate::testing::{Phase, TestRun, Verdict};

use super::search::{Kind, Search, Target};

/// Как часто заглядывать в лог идущего прогона за искомой ошибкой.
const PEEK_INTERVAL: Duration = Duration::from_secs(2);

/// Чем закончился очередной прогон.
#[derive(Clone, Debug)]
pub struct Attempt {
    /// Откуда взялся набор — часть сборки, дополнение или подсказка из лога.
    pub kind: Kind,
    pub verdict: Verdict,
    /// Сколько модов было в сборке.
    pub size: usize,
    pub reproduced: bool,
}

/// Проверка, что проблема воспроизводится на полной сборке.
///
/// Один прогон — это выборка, а не факт: на живой сборке шторм ошибок
/// Performance Optimizer был в 2 прогонах из 5. Сужать по такой проблеме —
/// значит получать «чисто» от везения и искать не там. Поэтому перед
/// сужением полная сборка прогоняется ещё раз.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confirm {
    /// Повтор ещё не сделан.
    Pending,
    /// Проблема повторилась — можно сужать.
    Reproduced,
    /// Не повторилась: она плавающая, и сужение не начинается.
    NotReproduced,
    /// Сужать нечего — и проверять тоже.
    Skipped,
}

/// Набор, который надо прогнать в рабочем месте `worker`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub worker: usize,
    pub active: Vec<ModId>,
}

/// Прогон, занимающий рабочее место.
struct Slot {
    worker: usize,
    kind: Kind,
    active: Vec<ModId>,
    run: TestRun,
    /// Когда последний раз смотрели в лог за искомой ошибкой.
    peeked: Option<Instant>,
}

/// Идущий поиск виновника.
pub struct Hunt {
    search: Search,
    target: Target,
    /// Полная сборка — для проверочного повтора.
    full: Vec<ModId>,
    confirm: Confirm,
    slots: Vec<Slot>,
    /// Сколько прогонов можно держать одновременно.
    workers: usize,
    /// Выданный, но ещё не запущенный набор: до [`Hunt::attach`] отдаётся
    /// снова, чтобы очередь не съезжала, если игру запустить не удалось.
    requested: Option<(Request, Kind)>,
    /// Сколько модов было в исходной сборке — для показа сужения.
    started_with: usize,
    /// Прогоны, которые провалились не по той причине, что мы ищем.
    ///
    /// Их приходится считать «не воспроизвелось», иначе поиск встанет, но
    /// это ослабляет результат: урезанная сборка могла упасть из-за того,
    /// что мы у неё что-то отняли. Поэтому число показывается пользователю.
    off_target: usize,
    attempts: Vec<Attempt>,
    cancelled: bool,
}

impl Hunt {
    pub fn new(search: Search, target: Target) -> Self {
        let confirm = if search.is_done() { Confirm::Skipped } else { Confirm::Pending };
        Self {
            started_with: search.result().len(),
            full: search.full_build(),
            confirm,
            search,
            target,
            slots: Vec::new(),
            workers: 1,
            requested: None,
            off_target: 0,
            attempts: Vec::new(),
            cancelled: false,
        }
    }

    /// Собирает поиск по результатам провалившегося прогона.
    ///
    /// Подозреваемые из разбора лога становятся подсказкой: если виновник
    /// среди них, сборка сузится с сотен модов до горстки за один прогон.
    /// Берутся подозреваемые только искомой проблемы и только с весомыми
    /// уликами: подсказка из всех ошибок лога разрослась бы до половины
    /// сборки и перестала бы экономить прогоны.
    pub fn from_failed_run(
        db: &ModDb,
        profile: &Profile,
        verdict: &Verdict,
        issues: &[LogIssue],
    ) -> Self {
        let target = Target::from_run(verdict, issues);
        Self::new(Search::from_profile(db, profile, hint_for(&target, issues)), target)
    }

    /// Сколько прогонов держать одновременно. Меняется на ходу: второе
    /// рабочее место появляется, когда для него готов свой префикс.
    pub fn set_workers(&mut self, workers: usize) {
        self.workers = workers.max(1);
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    pub fn search(&self) -> &Search {
        &self.search
    }

    /// Идущие прогоны: откуда набор и сам прогон.
    pub fn active_runs(&self) -> impl Iterator<Item = (Kind, &TestRun)> {
        self.slots.iter().map(|s| (s.kind, &s.run))
    }

    pub fn attempts(&self) -> &[Attempt] {
        &self.attempts
    }

    pub fn off_target(&self) -> usize {
        self.off_target
    }

    pub fn started_with(&self) -> usize {
        self.started_with
    }

    pub fn confirm(&self) -> Confirm {
        self.confirm
    }

    /// Проблема не повторилась на полной сборке — сужение не начиналось.
    pub fn not_reproducible(&self) -> bool {
        self.confirm == Confirm::NotReproduced
    }

    /// Сколько прогонов уже сделано, включая проверочный.
    pub fn run_count(&self) -> usize {
        self.attempts.len()
    }

    pub fn is_done(&self) -> bool {
        self.cancelled
            || self.not_reproducible()
            || (self.slots.is_empty()
                && self.requested.is_none()
                && self.confirm != Confirm::Pending
                && self.search.is_done())
    }

    pub fn was_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Следующий набор для свободного рабочего места, если есть что гонять.
    ///
    /// Повторный вызов до [`Hunt::attach`] отдаёт тот же набор: если запустить
    /// игру не удалось, очередь не должна съезжать.
    pub fn wants_run(&mut self) -> Option<Request> {
        if self.cancelled || self.not_reproducible() {
            return None;
        }
        if let Some((request, _)) = &self.requested {
            return Some(request.clone());
        }
        let worker = (0..self.workers).find(|w| !self.slots.iter().any(|s| s.worker == *w))?;
        let (kind, active) = self
            .wanted()
            .into_iter()
            .find(|(_, set)| !self.slots.iter().any(|s| same_set(&s.active, set)))?;
        let request = Request { worker, active };
        self.requested = Some((request.clone(), kind));
        Some(request)
    }

    /// Отдаёт запущенный прогон под управление.
    pub fn attach(&mut self, run: TestRun) {
        let Some((request, kind)) = self.requested.take() else { return };
        self.slots.push(Slot {
            worker: request.worker,
            kind,
            active: request.active,
            run,
            peeked: None,
        });
    }

    /// Двигает состояние. Возвращает `true`, если какой-то прогон только что
    /// закончился.
    pub fn poll(&mut self) -> bool {
        let target = &self.target;
        for slot in &mut self.slots {
            if matches!(slot.run.poll(), Phase::Done(_)) {
                continue;
            }
            // Искомая ошибка уже в логе — ответ есть, доигрывать незачем.
            if slot.peeked.is_none_or(|t| t.elapsed() >= PEEK_INTERVAL) {
                slot.peeked = Some(Instant::now());
                if target.found_in(&parse_log(&slot.run.log_text())) {
                    slot.run.conclude(Verdict::StoppedOnTarget);
                }
            }
        }

        let mut finished = false;
        let mut i = 0;
        while i < self.slots.len() {
            let Phase::Done(verdict) = self.slots[i].run.phase().clone() else {
                i += 1;
                continue;
            };
            let slot = self.slots.remove(i);
            finished = true;
            // Остановку прогона пользователем нельзя считать ответом: мы не
            // знаем, воспроизвелось бы или нет.
            if verdict == Verdict::Cancelled {
                self.cancel();
                return true;
            }
            self.finish(slot, verdict);
        }
        if finished {
            self.prune();
        }
        finished
    }

    /// Останавливает поиск, оставляя найденное приближение.
    pub fn cancel(&mut self) {
        for slot in &mut self.slots {
            slot.run.cancel();
        }
        self.slots.clear();
        self.requested = None;
        self.search.stop();
        self.cancelled = true;
    }

    /// Наборы, которые нужны прямо сейчас, по порядку важности.
    fn wanted(&mut self) -> Vec<(Kind, Vec<ModId>)> {
        let mut out = Vec::new();
        if self.confirm == Confirm::Pending {
            out.push((Kind::Confirm, self.full.clone()));
        }
        if self.confirm != Confirm::NotReproduced {
            // Выдать набор поиску: ответ на него двигает поиск, а не кеш.
            self.search.next();
            out.extend(self.search.plan(self.workers).into_iter().map(|c| (c.kind, c.active)));
        }
        out.truncate(self.workers);
        out
    }

    fn finish(&mut self, slot: Slot, verdict: Verdict) {
        // Причине не нужны подозреваемые: хватает разбора без индекса модов.
        let issues = parse_log(&slot.run.log_text());
        let reproduced = self.target.matches(&verdict, &issues);
        self.attempts.push(Attempt {
            kind: slot.kind,
            verdict: verdict.clone(),
            size: slot.active.len(),
            reproduced,
        });

        if slot.kind == Kind::Confirm {
            self.confirm = if reproduced { Confirm::Reproduced } else { Confirm::NotReproduced };
            return;
        }
        if !reproduced && !matches!(verdict, Verdict::Passed) {
            self.off_target += 1;
        }
        self.search.remember(&slot.active, reproduced);
    }

    /// Снимает прогоны, которые после нового ответа стали не нужны: поиск
    /// свернул в другую сторону, или искать больше нечего.
    fn prune(&mut self) {
        let wanted = if self.not_reproducible() { Vec::new() } else { self.wanted() };
        self.slots.retain_mut(|slot| {
            let keep = wanted.iter().any(|(_, set)| same_set(set, &slot.active));
            if !keep {
                slot.run.cancel();
            }
            keep
        });
    }
}

/// Один и тот же ли состав, независимо от порядка.
fn same_set(a: &[ModId], b: &[ModId]) -> bool {
    a.len() == b.len() && {
        let left: HashSet<&ModId> = a.iter().collect();
        b.iter().all(|id| left.contains(id))
    }
}

/// Минимальный вес улики, с которым подозреваемый идёт в подсказку:
/// упоминание названия в тексте (1) — слишком слабо.
const HINT_MIN_SCORE: i32 = 3;
/// Больше — и подсказка уже не экономит прогоны.
const HINT_MAX: usize = 5;

/// Подозреваемые искомой проблемы — подсказка для поиска.
fn hint_for(target: &Target, issues: &[LogIssue]) -> Vec<ModId> {
    let mut scored: Vec<(&ModId, i32)> = Vec::new();
    for issue in target.issues(issues) {
        for s in issue.suspects.iter().filter(|s| s.is_active && s.score >= HINT_MIN_SCORE) {
            match scored.iter_mut().find(|(id, _)| *id == &s.package_id) {
                Some((_, score)) => *score = (*score).max(s.score),
                None => scored.push((&s.package_id, s.score)),
            }
        }
    }
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().take(HINT_MAX).map(|(id, _)| id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bisect::search::Deps;
    use crate::testing::Config;

    fn ids(list: &[&str]) -> Vec<ModId> {
        list.iter().map(|s| ModId::new(s)).collect()
    }

    fn hunt(candidates: &[&str]) -> Hunt {
        Hunt::new(
            Search::new(ids(candidates), Vec::new(), Deps::default(), Vec::new()),
            Target::AnyFailure,
        )
    }

    /// Прогон-заглушка: процесс, который ничего не пишет.
    fn idle_run(tag: &str, active: &[ModId]) -> (TestRun, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("rustrim_driver_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = Config::new(dir.join("Config"), dir.join("run"), dir.join("game.log"));
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg("sleep 30");
        (TestRun::start(cmd, "fake".into(), cfg, active).unwrap(), dir)
    }

    #[test]
    fn the_same_set_is_offered_until_a_run_is_attached() {
        // Если игру запустить не удалось, очередь съезжать не должна.
        let mut h = hunt(&["a", "b", "c", "d"]);
        let first = h.wants_run().expect("есть что проверять");
        assert_eq!(h.wants_run(), Some(first));
    }

    #[test]
    fn cancelling_stops_asking_for_runs() {
        let mut h = hunt(&["a", "b", "c", "d"]);
        h.wants_run();
        h.cancel();
        assert!(h.wants_run().is_none());
        assert!(h.is_done());
        assert!(h.was_cancelled());
    }

    #[test]
    fn the_full_build_is_rechecked_first() {
        let mut h = hunt(&["a", "b", "c", "d"]);
        assert_eq!(h.confirm(), Confirm::Pending);
        let first = h.wants_run().expect("есть что проверять");
        assert_eq!(first.active, ids(&["a", "b", "c", "d"]), "сначала — полная сборка");
        assert_eq!(first.worker, 0);
        assert!(!h.is_done());
    }

    #[test]
    fn a_second_worker_runs_the_next_set_alongside() {
        let mut h = hunt(&["a", "b", "c", "d"]);
        h.set_workers(2);

        let first = h.wants_run().unwrap();
        let (run, dir1) = idle_run("w0", &first.active);
        h.attach(run);

        let second = h.wants_run().expect("второе место свободно");
        assert_eq!(second.worker, 1);
        assert_ne!(second.active, first.active, "параллельно — другой набор");
        let (run, dir2) = idle_run("w1", &second.active);
        h.attach(run);

        assert!(h.wants_run().is_none(), "оба места заняты");
        assert_eq!(h.active_runs().count(), 2);
        h.cancel();
        let _ = std::fs::remove_dir_all(dir1);
        let _ = std::fs::remove_dir_all(dir2);
    }

    #[test]
    fn hint_comes_from_the_target_issue_only() {
        use crate::log_analysis::{LogIssue, Severity, Suspect};
        let suspect = |id: &str, score: i32| Suspect {
            package_id: ModId::new(id),
            name: id.to_string(),
            is_active: true,
            score,
            evidence: Vec::new(),
        };
        let issue = |cause: &str, count: usize, suspects: Vec<Suspect>| LogIssue {
            severity: Severity::Error,
            title: cause.to_string(),
            full_text: String::new(),
            count,
            frames: Vec::new(),
            exception: None,
            cause: cause.to_string(),
            suspects,
            harmony_hint: None,
        };
        let issues = [
            issue("main", 100, vec![suspect("culprit", 8), suspect("mentioned", 1)]),
            issue("other", 1, vec![suspect("unrelated", 6)]),
        ];
        let target = Target::from_issues(&issues);
        // Ни слабое упоминание, ни подозреваемый чужой ошибки в подсказку не идут.
        assert_eq!(hint_for(&target, &issues), ids(&["culprit"]));
    }

    #[test]
    fn an_empty_build_is_done_at_once() {
        let mut h = hunt(&[]);
        assert!(h.wants_run().is_none());
        assert!(h.is_done());
    }
}
