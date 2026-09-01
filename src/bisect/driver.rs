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

/// Идущий поиск виновника.
pub struct Hunt {
    search: Search,
    target: Target,
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
        Self {
            started_with: search.result().len(),
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
    pub fn from_failed_run(db: &ModDb, profile: &Profile, issues: &[LogIssue]) -> Self {
        let hint: Vec<ModId> = issues
            .iter()
            .flat_map(|issue| issue.suspects.iter())
            .filter(|s| s.is_active)
            .map(|s| s.package_id.clone())
            .collect();
        Self::new(
            Search::from_profile(db, profile, hint),
            Target::from_issues(issues),
        )
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

    pub fn is_done(&self) -> bool {
        self.cancelled || (self.run.is_none() && self.search.is_done())
    }

    pub fn was_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Набор, который надо прогнать прямо сейчас, если прогона ещё нет.
    ///
    /// Повторный вызов до [`Hunt::attach`] отдаёт тот же набор: если запустить
    /// игру не удалось, очередь не должна съезжать.
    pub fn wants_run(&mut self) -> Option<Vec<ModId>> {
        if self.cancelled || self.run.is_some() {
            return None;
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
        if !reproduced && !matches!(verdict, Verdict::Passed) {
            self.off_target += 1;
        }

        self.attempts.push(Attempt {
            kind: self.pending_kind.take().unwrap_or(Kind::Part),
            verdict,
            size,
            reproduced,
        });
        self.search.record(reproduced);
        self.run = None;
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
    fn an_empty_build_is_done_at_once() {
        let mut h = hunt(&[]);
        assert!(h.wants_run().is_none());
        assert!(h.is_done());
    }
}
