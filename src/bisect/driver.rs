//! Гоняет игру по наборам, которые выдаёт [`super::search`].
//!
//! Сам процессы не запускает: собрать команду умеет только приложение, где
//! лежат настройки и пути. Драйвер говорит, какой набор нужен, получает
//! готовый прогон и превращает его результат в ответ «воспроизвелось / нет».

use crate::log_analysis::LogIssue;
use crate::mod_data::{ModDb, ModId, Profile};
use crate::testing::{Phase, TestRun, Verdict};

use super::search::{Kind, Search, Target};

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

/// Идущий поиск виновника.
pub struct Hunt {
    search: Search,
    target: Target,
    /// Полная сборка — для проверочного повтора.
    full: Vec<ModId>,
    confirm: Confirm,
    run: Option<TestRun>,
    /// Сколько модов было в исходной сборке — для показа сужения.
    started_with: usize,
    /// Прогоны, которые провалились не по той причине, что мы ищем.
    ///
    /// Их приходится считать «не воспроизвелось», иначе поиск встанет, но
    /// это ослабляет результат: урезанная сборка могла упасть из-за того,
    /// что мы у неё что-то отняли. Поэтому число показывается пользователю.
    off_target: usize,
    attempts: Vec<Attempt>,
    /// Откуда взялся набор, который сейчас прогоняется.
    pending_kind: Option<Kind>,
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
            run: None,
            off_target: 0,
            attempts: Vec::new(),
            pending_kind: None,
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
    pub fn from_failed_run(db: &ModDb, profile: &Profile, issues: &[LogIssue]) -> Self {
        let target = Target::from_issues(issues);
        Self::new(Search::from_profile(db, profile, hint_for(&target, issues)), target)
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    pub fn search(&self) -> &Search {
        &self.search
    }

    pub fn run(&self) -> Option<&TestRun> {
        self.run.as_ref()
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
    pub fn runs(&self) -> usize {
        self.attempts.len()
    }

    pub fn is_done(&self) -> bool {
        self.cancelled
            || self.not_reproducible()
            || (self.run.is_none() && self.confirm != Confirm::Pending && self.search.is_done())
    }

    pub fn was_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Набор, который надо прогнать прямо сейчас, если прогона ещё нет.
    ///
    /// Повторный вызов до [`Hunt::attach`] отдаёт тот же набор: если запустить
    /// игру не удалось, очередь не должна съезжать.
    pub fn wants_run(&mut self) -> Option<Vec<ModId>> {
        if self.cancelled || self.run.is_some() || self.not_reproducible() {
            return None;
        }
        if self.confirm == Confirm::Pending {
            self.pending_kind = Some(Kind::Confirm);
            return Some(self.full.clone());
        }
        self.search.next().map(|c| {
            self.pending_kind = Some(c.kind);
            c.active
        })
    }

    /// Отдаёт запущенный прогон под управление.
    pub fn attach(&mut self, run: TestRun) {
        self.run = Some(run);
    }

    /// Двигает состояние. Возвращает `true`, если прогон только что закончился.
    pub fn poll(&mut self, db: &ModDb, profile: &Profile) -> bool {
        let Some(run) = &mut self.run else { return false };
        let Phase::Done(verdict) = run.poll().clone() else {
            return false;
        };

        // Остановку прогона пользователем нельзя считать ответом: мы не
        // знаем, воспроизвелось бы или нет.
        if verdict == Verdict::Cancelled {
            self.cancel();
            return true;
        }

        let issues = run.issues(db, profile);
        let size = run.active().len();
        let reproduced = self.target.matches(&verdict, &issues);
        let kind = self.pending_kind.take().unwrap_or(Kind::Part);
        self.attempts.push(Attempt { kind, verdict: verdict.clone(), size, reproduced });
        self.run = None;

        if kind == Kind::Confirm {
            self.confirm = if reproduced { Confirm::Reproduced } else { Confirm::NotReproduced };
            return true;
        }
        if !reproduced && !matches!(verdict, Verdict::Passed) {
            self.off_target += 1;
        }
        self.search.record(reproduced);
        true
    }

    /// Останавливает поиск, оставляя найденное приближение.
    pub fn cancel(&mut self) {
        if let Some(run) = &mut self.run {
            run.cancel();
        }
        self.run = None;
        self.search.stop();
        self.cancelled = true;
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

    fn ids(list: &[&str]) -> Vec<ModId> {
        list.iter().map(|s| ModId::new(s)).collect()
    }

    fn hunt(candidates: &[&str]) -> Hunt {
        Hunt::new(
            Search::new(ids(candidates), Vec::new(), Deps::default(), Vec::new()),
            Target::AnyFailure,
        )
    }

    #[test]
    fn the_same_set_is_offered_until_a_run_is_attached() {
        // Если игру запустить не удалось, очередь съезжать не должна.
        let mut h = hunt(&["a", "b", "c", "d"]);
        let first = h.wants_run().expect("есть что проверять");
        assert_eq!(h.wants_run().as_deref(), Some(first.as_slice()));
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
        assert_eq!(first, ids(&["a", "b", "c", "d"]), "сначала — полная сборка");
        assert!(!h.is_done());
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
