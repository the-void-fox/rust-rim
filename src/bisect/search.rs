//! Что проверять следующим: сам алгоритм сужения.
//!
//! Наивная бисекция пополам здесь не работает. Она предполагает, что виновник
//! один, а в RimWorld типичная поломка — это пара модов, которые по
//! отдельности безобидны. Делим пополам, обе половины чистые, и алгоритм
//! честно сообщает, что виновных нет.
//!
//! Поэтому используется дельта-отладка (ddmin Целлера): она перебирает не
//! только части, но и дополнения к ним, за счёт чего находит и одиночного
//! виновника, и пару. Цена — прогоны, а прогон стоит минуты.
//!
//! Экономия здесь и есть главная работа. На живой сборке в 898 модов
//! (`examples/bisect_smoke.rs`) одиночный виновник обходится в 9–15 прогонов,
//! а вот пара — в 61, это два с половиной часа. Поэтому подсказка из разбора
//! лога используется не только целиком: если она не воспроизводит проблему
//! сама по себе, подозреваемые всё равно остаются включёнными, пока ищется
//! их недостающий напарник. Для пары, где лог назвал одного, это разница
//! между 61 прогоном и 14.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::log_analysis::{LogIssue, Severity};
use crate::mod_data::{ModDb, ModId, Profile};
use crate::testing::Verdict;

/// Что считать воспроизведением проблемы.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Любой провал: прогон не закончился успехом.
    AnyFailure,
    /// Конкретная запись из лога исходного прогона.
    ///
    /// Точнее, чем «любой провал»: урезанная сборка может падать по своей
    /// причине (не хватило чего-то, что мы выключили), и такой провал за
    /// воспроизведение считать нельзя.
    Issue(String),
}

impl Target {
    /// Выбирает цель по разбору исходного прогона.
    ///
    /// Если в логе есть ошибки — ищем первую из них; она же самая частая,
    /// потому что `analyze` сортирует записи по убыванию частоты.
    pub fn from_issues(issues: &[LogIssue]) -> Self {
        issues
            .iter()
            .find(|i| i.severity == Severity::Error)
            .map(|i| Target::Issue(i.title.clone()))
            .unwrap_or(Target::AnyFailure)
    }

    /// Воспроизвелась ли проблема в очередном прогоне.
    pub fn matches(&self, verdict: &Verdict, issues: &[LogIssue]) -> bool {
        match self {
            Target::AnyFailure => !matches!(verdict, Verdict::Passed),
            Target::Issue(title) => issues.iter().any(|i| &i.title == title),
        }
    }
}

/// Зависимости внутри сборки: без кого какой мод нельзя включать.
///
/// Нужны потому, что произвольное подмножество — не рабочая сборка. Выключив
/// Harmony, мы получим падение всех модов сразу, и поиск сойдётся на нём
/// вместо настоящего виновника.
#[derive(Clone, Debug, Default)]
pub struct Deps(HashMap<ModId, Vec<ModId>>);

impl Deps {
    /// Собирает зависимости для активных модов сборки.
    ///
    /// Учитываются только те зависимости, которые в сборке есть: отсутствующие
    /// и так уже сломаны, и выключение их ничего не меняет.
    pub fn from_db(db: &ModDb, profile: &Profile) -> Self {
        let active: HashSet<&ModId> = profile.order().iter().collect();
        let mut map = HashMap::new();
        for id in profile.order() {
            let Some(entry) = db.get(id) else { continue };
            let needed: Vec<ModId> = entry
                .dependencies
                .iter()
                .filter(|dep| active.contains(dep))
                .cloned()
                .collect();
            if !needed.is_empty() {
                map.insert(id.clone(), needed);
            }
        }
        Self(map)
    }

    /// Дополняет набор всем, без чего он не соберётся.
    pub fn close(&self, subset: &[ModId]) -> Vec<ModId> {
        let mut seen: HashSet<ModId> = subset.iter().cloned().collect();
        let mut queue: Vec<ModId> = subset.to_vec();
        while let Some(id) = queue.pop() {
            let Some(deps) = self.0.get(&id) else { continue };
            for dep in deps {
                if seen.insert(dep.clone()) {
                    queue.push(dep.clone());
                }
            }
        }
        seen.into_iter().collect()
    }
}

/// Откуда взялся очередной набор — для журнала в интерфейсе.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Подсказка из разбора лога целиком.
    Hint,
    /// Часть текущего набора.
    Part,
    /// Всё, кроме одной части.
    Rest,
    /// Проверка, нужна ли подсказка вообще.
    Recheck,
}

/// Один сделанный шаг.
#[derive(Clone, Debug)]
pub struct Step {
    pub kind: Kind,
    /// Сколько модов было в проверенном наборе.
    pub size: usize,
    pub reproduced: bool,
}

/// Набор, который надо прогнать.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub kind: Kind,
    /// Часть, которую сужаем на этом шаге.
    pub subset: Vec<ModId>,
    /// Что реально включать в сборку: подозреваемые плюс удерживаемые,
    /// ваниль и вытянутые зависимости, в исходном порядке загрузки.
    pub active: Vec<ModId>,
}

/// Чем сейчас занят поиск.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Проверяем подсказку из лога саму по себе.
    HintAlone,
    /// Подсказка сама не сработала: держим её включённой и ищем напарника.
    WithHint,
    /// Напарник найден — а нужна ли была подсказка?
    HintNeeded,
    /// Нужна: сужаем теперь саму подсказку.
    MinimizeHint,
    /// Обычное сужение без подсказки.
    Plain,
    Done,
}

/// Где мы внутри одного разбиения.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    /// Перебираем части.
    Parts,
    /// Перебираем дополнения к частям.
    Rest,
}

/// Состояние поиска. Двигается снаружи: [`Search::next`] выдаёт набор,
/// [`Search::record`] принимает ответ.
pub struct Search {
    /// Порядок загрузки исходной сборки — по нему выстраиваются наборы.
    order: Vec<ModId>,
    /// Ваниль: включена всегда и под подозрение не попадает.
    base: HashSet<ModId>,
    deps: Deps,
    /// Подозреваемые из разбора лога.
    hint: Vec<ModId>,

    /// Держим включённым сверх ванили, пока сужаем `pool`.
    pinned: Vec<ModId>,
    /// То, что сужаем прямо сейчас.
    pool: Vec<ModId>,
    /// Уже установленная часть ответа.
    found: Vec<ModId>,

    phase: Phase,
    /// На сколько частей режем `pool`.
    parts: usize,
    stage: Stage,
    /// Какую часть проверяем.
    index: usize,

    /// Состав набора → воспроизвелось ли. Прогон стоит минуты, повторять
    /// один и тот же состав недопустимо.
    cache: HashMap<u64, bool>,
    /// Что выдали последним и ждём ответа.
    pending: Option<Candidate>,
    log: Vec<Step>,
    runs: usize,
}

impl Search {
    /// `candidates` — подозреваемые в порядке загрузки, `base` — ваниль,
    /// `hint` — подозреваемые из разбора лога.
    pub fn new(candidates: Vec<ModId>, base: Vec<ModId>, deps: Deps, hint: Vec<ModId>) -> Self {
        let mut order = base.clone();
        order.extend(candidates.iter().cloned());

        let suspects: HashSet<&ModId> = candidates.iter().collect();
        // Подсказка полезна, только если она уже, чем весь набор, и не пуста.
        let hint: Vec<ModId> = hint.into_iter().filter(|id| suspects.contains(id)).collect();
        let use_hint = !hint.is_empty() && hint.len() < candidates.len();

        let phase = if candidates.is_empty() {
            Phase::Done
        } else if use_hint {
            Phase::HintAlone
        } else {
            Phase::Plain
        };

        Self {
            order,
            base: base.into_iter().collect(),
            deps,
            hint: if use_hint { hint } else { Vec::new() },
            pinned: Vec::new(),
            pool: candidates,
            found: Vec::new(),
            phase,
            parts: 2,
            stage: Stage::Parts,
            index: 0,
            cache: HashMap::new(),
            pending: None,
            log: Vec::new(),
            runs: 0,
        }
    }

    /// Собирает поиск по сборке и подозреваемым из лога.
    ///
    /// Ванильный контент под подозрение не попадает: без Core игра не
    /// стартует, а DLC ломаются редко и выключать их бессмысленно.
    pub fn from_profile(db: &ModDb, profile: &Profile, hint: Vec<ModId>) -> Self {
        let mut base = Vec::new();
        let mut candidates = Vec::new();
        for id in profile.order() {
            match db.get(id) {
                Some(entry) if entry.is_vanilla() => base.push(id.clone()),
                _ => candidates.push(id.clone()),
            }
        }
        Self::new(candidates, base, Deps::from_db(db, profile), hint)
    }

    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }

    /// Найденный набор. Пока поиск идёт — текущее приближение.
    pub fn result(&self) -> Vec<ModId> {
        let keep: HashSet<&ModId> = self
            .pinned
            .iter()
            .chain(&self.pool)
            .chain(&self.found)
            .collect();
        self.order
            .iter()
            .filter(|id| keep.contains(*id))
            .cloned()
            .collect()
    }

    pub fn steps(&self) -> &[Step] {
        &self.log
    }

    /// Сколько прогонов уже сделано (наборы, узнанные по кешу, не в счёт).
    pub fn runs(&self) -> usize {
        self.runs
    }

    /// Следующий набор для прогона. `None` — сужать больше нечего.
    ///
    /// Наборы, состав которых уже проверялся, пропускаются без прогона.
    pub fn next(&mut self) -> Option<Candidate> {
        if let Some(pending) = &self.pending {
            return Some(pending.clone());
        }
        loop {
            let candidate = self.step()?;
            let key = hash_ids(&candidate.active);
            match self.cache.get(&key).copied() {
                Some(reproduced) => self.advance(&candidate, reproduced),
                None => {
                    self.pending = Some(candidate.clone());
                    return Some(candidate);
                }
            }
        }
    }

    /// Принимает результат прогона выданного набора.
    pub fn record(&mut self, reproduced: bool) {
        let Some(candidate) = self.pending.take() else { return };
        self.cache.insert(hash_ids(&candidate.active), reproduced);
        self.runs += 1;
        self.log.push(Step {
            kind: candidate.kind,
            size: candidate.subset.len(),
            reproduced,
        });
        self.advance(&candidate, reproduced);
    }

    /// Прекращает поиск, оставляя текущее приближение как результат.
    pub fn stop(&mut self) {
        self.pending = None;
        self.phase = Phase::Done;
    }

    /// Что проверять дальше, без учёта кеша.
    fn step(&mut self) -> Option<Candidate> {
        loop {
            match self.phase {
                Phase::Done => return None,
                Phase::HintAlone => return Some(self.candidate(Kind::Hint, self.hint.clone())),
                // Пустой subset: в сборке остаётся только ваниль и найденное.
                Phase::HintNeeded => {
                    return Some(self.candidate(Kind::Recheck, self.found.clone()))
                }
                _ => {}
            }

            if self.pool.len() < 2 {
                self.finish_phase();
                continue;
            }

            let picked: Option<(Kind, Vec<ModId>)> = {
                let parts = split(&self.pool, self.parts);
                match self.stage {
                    Stage::Parts if self.index < parts.len() => {
                        Some((Kind::Part, parts[self.index].to_vec()))
                    }
                    Stage::Rest if self.index < parts.len() => {
                        let skip: HashSet<&ModId> = parts[self.index].iter().collect();
                        let rest = self
                            .pool
                            .iter()
                            .filter(|id| !skip.contains(id))
                            .cloned()
                            .collect();
                        Some((Kind::Rest, rest))
                    }
                    _ => None,
                }
            };
            if let Some((kind, subset)) = picked {
                return Some(self.candidate(kind, subset));
            }

            // Текущее разбиение перебрано целиком.
            match self.stage {
                Stage::Parts => {
                    self.stage = Stage::Rest;
                    self.index = 0;
                }
                Stage::Rest if self.parts >= self.pool.len() => self.finish_phase(),
                Stage::Rest => {
                    self.parts = (self.parts * 2).min(self.pool.len());
                    self.stage = Stage::Parts;
                    self.index = 0;
                }
            }
        }
    }

    /// Двигает состояние по ответу на конкретный набор.
    fn advance(&mut self, candidate: &Candidate, reproduced: bool) {
        match (candidate.kind, reproduced) {
            // Подсказка воспроизвела проблему сама: остальная сборка ни при
            // чём, дальше сужаем только её.
            (Kind::Hint, true) => {
                self.pool = candidate.subset.clone();
                self.phase = Phase::Plain;
                self.restart();
            }
            // Не воспроизвела — но это не значит, что подозреваемые невиновны.
            // Держим их включёнными и ищем, кого им не хватает.
            (Kind::Hint, false) => {
                let hint: HashSet<&ModId> = self.hint.iter().collect();
                self.pool = self
                    .pool
                    .iter()
                    .filter(|id| !hint.contains(id))
                    .cloned()
                    .collect();
                self.pinned = self.hint.clone();
                self.phase = Phase::WithHint;
                self.restart();
            }
            // Без подсказки тоже воспроизводится — она была ни при чём.
            (Kind::Recheck, true) => {
                self.pool = std::mem::take(&mut self.found);
                self.phase = Phase::Plain;
                self.restart();
            }
            // Подсказка всё-таки нужна: теперь сужаем её саму.
            (Kind::Recheck, false) => {
                self.pinned = std::mem::take(&mut self.found);
                self.pool = self.hint.clone();
                self.phase = Phase::MinimizeHint;
                self.restart();
            }
            (Kind::Part, true) => {
                self.pool = candidate.subset.clone();
                self.restart();
            }
            (Kind::Rest, true) => {
                self.pool = candidate.subset.clone();
                // Часть набора уже исключена, поэтому дробим на единицу крупнее.
                self.parts = self.parts.saturating_sub(1).max(2);
                self.stage = Stage::Parts;
                self.index = 0;
            }
            (Kind::Part | Kind::Rest, false) => self.index += 1,
        }
    }

    /// Сужать в этой фазе больше нечего — что дальше.
    fn finish_phase(&mut self) {
        match self.phase {
            // Напарник найден. Проверим, была ли подсказка вообще нужна:
            // разбор лога мог ошибиться, и тогда она только засоряет ответ.
            Phase::WithHint => {
                self.found = std::mem::take(&mut self.pool);
                self.pinned.clear();
                self.phase = Phase::HintNeeded;
            }
            _ => self.phase = Phase::Done,
        }
    }

    fn restart(&mut self) {
        self.parts = 2;
        self.stage = Stage::Parts;
        self.index = 0;
    }

    /// Достраивает проверяемую часть до рабочей сборки.
    fn candidate(&self, kind: Kind, subset: Vec<ModId>) -> Candidate {
        let mut keep: HashSet<ModId> = self.deps.close(&subset).into_iter().collect();
        keep.extend(self.deps.close(&self.pinned));
        keep.extend(self.base.iter().cloned());
        let active = self
            .order
            .iter()
            .filter(|id| keep.contains(*id))
            .cloned()
            .collect();
        Candidate { kind, subset, active }
    }
}

/// Режет набор на `n` частей примерно поровну.
fn split(items: &[ModId], n: usize) -> Vec<&[ModId]> {
    let n = n.clamp(1, items.len().max(1));
    let mut parts = Vec::with_capacity(n);
    let mut start = 0;
    for i in 0..n {
        let end = items.len() * (i + 1) / n;
        parts.push(&items[start..end]);
        start = end;
    }
    parts.retain(|p| !p.is_empty());
    parts
}

/// Ключ кеша: состав набора, независимо от порядка перечисления.
fn hash_ids(ids: &[ModId]) -> u64 {
    let mut sorted: Vec<&str> = ids.iter().map(ModId::as_str).collect();
    sorted.sort_unstable();
    let mut hasher = DefaultHasher::new();
    sorted.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<ModId> {
        list.iter().map(|s| ModId::new(s)).collect()
    }

    /// Гоняет поиск против заранее известной причины поломки.
    ///
    /// `cause` — набор, при полном наличии которого проблема воспроизводится.
    /// Так моделируются и одиночный виновник, и конфликт пары.
    fn run(search: &mut Search, cause: &[&str]) -> usize {
        let cause = ids(cause);
        let mut runs = 0;
        while let Some(candidate) = search.next() {
            let present: HashSet<&ModId> = candidate.active.iter().collect();
            let reproduced = cause.iter().all(|c| present.contains(c));
            search.record(reproduced);
            runs += 1;
            assert!(runs < 500, "поиск не сходится");
        }
        runs
    }

    fn search(candidates: &[&str]) -> Search {
        Search::new(ids(candidates), Vec::new(), Deps::default(), Vec::new())
    }

    fn names(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("mod{i:02}")).collect()
    }

    #[test]
    fn finds_a_single_culprit() {
        let mut s = search(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        run(&mut s, &["f"]);
        assert_eq!(s.result(), ids(&["f"]));
    }

    #[test]
    fn finds_a_conflicting_pair() {
        // Ради чего всё и затевалось: по отдельности «b» и «g» безобидны,
        // и деление пополам объявило бы, что виновных нет.
        let mut s = search(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        run(&mut s, &["b", "g"]);
        assert_eq!(s.result(), ids(&["b", "g"]));
    }

    #[test]
    fn a_single_culprit_costs_far_fewer_runs_than_brute_force() {
        let names = names(64);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut s = Search::new(ids(&refs), Vec::new(), Deps::default(), Vec::new());

        let runs = run(&mut s, &["mod40"]);
        assert_eq!(s.result(), ids(&["mod40"]));
        assert!(runs < 32, "прогонов {runs} — дороже перебора половины сборки");
    }

    #[test]
    fn a_matching_hint_saves_most_of_the_work() {
        let names = names(64);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();

        let mut without = Search::new(ids(&refs), Vec::new(), Deps::default(), Vec::new());
        let plain = run(&mut without, &["mod40"]);

        let mut with = Search::new(
            ids(&refs),
            Vec::new(),
            Deps::default(),
            ids(&["mod39", "mod40", "mod41"]),
        );
        let hinted = run(&mut with, &["mod40"]);

        assert_eq!(with.result(), ids(&["mod40"]));
        assert!(hinted < plain, "подсказка не сэкономила: {hinted} против {plain}");
    }

    #[test]
    fn a_hint_naming_half_of_a_pair_still_saves_work() {
        // Самый частый случай на практике: исключение прилетело из кода одного
        // мода, а ломается он в паре со вторым, которого в логе нет. Раньше
        // такая подсказка не давала ничего — сборка искалась с нуля.
        let names = names(64);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();

        let mut without = Search::new(ids(&refs), Vec::new(), Deps::default(), Vec::new());
        let plain = run(&mut without, &["mod05", "mod50"]);

        let mut with =
            Search::new(ids(&refs), Vec::new(), Deps::default(), ids(&["mod05"]));
        let hinted = run(&mut with, &["mod05", "mod50"]);

        assert_eq!(with.result(), ids(&["mod05", "mod50"]));
        assert!(
            hinted * 2 < plain,
            "подсказка про половину пары не помогла: {hinted} против {plain}",
        );
    }

    #[test]
    fn a_wrong_hint_does_not_lose_the_culprit() {
        // Разбор лога ошибся: виновника среди подозреваемых нет. Подсказка
        // не должна ни потерять виновника, ни просочиться в ответ.
        let mut s = Search::new(
            ids(&["a", "b", "c", "d", "e", "f", "g", "h"]),
            Vec::new(),
            Deps::default(),
            ids(&["a", "b"]),
        );
        run(&mut s, &["g"]);
        assert_eq!(s.result(), ids(&["g"]));
    }

    #[test]
    fn a_partly_wrong_hint_is_trimmed_down() {
        // Лог назвал троих, виновен один из них — и то в паре. Лишние двое
        // не должны остаться в ответе.
        let mut s = Search::new(
            ids(&["a", "b", "c", "d", "e", "f", "g", "h"]),
            Vec::new(),
            Deps::default(),
            ids(&["a", "b", "c"]),
        );
        run(&mut s, &["b", "h"]);
        assert_eq!(s.result(), ids(&["b", "h"]));
    }

    #[test]
    fn dependencies_are_always_included() {
        // «b» требует «a». Набор без «a» — не рабочая сборка, и прогонять
        // его бессмысленно: упадёт не из-за конфликта.
        let mut deps = Deps::default();
        deps.0.insert(ModId::new("b"), ids(&["a"]));

        let mut s = Search::new(ids(&["a", "b", "c", "d"]), Vec::new(), deps, Vec::new());
        while let Some(candidate) = s.next() {
            if candidate.active.contains(&ModId::new("b")) {
                assert!(
                    candidate.active.contains(&ModId::new("a")),
                    "зависимость не подтянулась: {:?}",
                    candidate.active,
                );
            }
            s.record(candidate.active.contains(&ModId::new("c")));
        }
        assert_eq!(s.result(), ids(&["c"]));
    }

    #[test]
    fn base_mods_are_in_every_run() {
        let mut s = Search::new(
            ids(&["a", "b", "c"]),
            ids(&["core"]),
            Deps::default(),
            Vec::new(),
        );
        while let Some(candidate) = s.next() {
            assert!(candidate.active.contains(&ModId::new("core")));
            s.record(candidate.active.contains(&ModId::new("b")));
        }
    }

    #[test]
    fn the_same_set_is_never_run_twice() {
        let mut s = search(&["a", "b", "c", "d", "e", "f"]);
        let mut seen: HashSet<u64> = HashSet::new();
        while let Some(candidate) = s.next() {
            let key = hash_ids(&candidate.active);
            assert!(seen.insert(key), "состав повторился: {:?}", candidate.active);
            s.record(candidate.active.contains(&ModId::new("e")));
        }
    }

    #[test]
    fn runs_keep_the_original_load_order() {
        let mut s = Search::new(
            ids(&["a", "b", "c", "d"]),
            ids(&["core"]),
            Deps::default(),
            Vec::new(),
        );
        while let Some(candidate) = s.next() {
            let mut sorted = candidate.active.clone();
            sorted.sort_by_key(|id| {
                s.order.iter().position(|o| o == id).expect("мод из сборки")
            });
            assert_eq!(candidate.active, sorted, "порядок загрузки нарушен");
            s.record(false);
        }
    }

    #[test]
    fn an_empty_build_finishes_immediately() {
        let mut s = search(&[]);
        assert!(s.next().is_none());
        assert!(s.is_done());
    }

    #[test]
    fn a_single_mod_needs_no_runs() {
        let mut s = search(&["only"]);
        assert!(s.next().is_none());
        assert_eq!(s.result(), ids(&["only"]));
    }

    #[test]
    fn stopping_keeps_the_current_approximation() {
        let mut s = search(&["a", "b", "c", "d"]);
        s.next();
        s.record(true);
        let narrowed = s.result();
        s.stop();
        assert!(s.is_done());
        assert_eq!(s.result(), narrowed);
        assert!(s.next().is_none());
    }

    #[test]
    fn dependency_closure_is_transitive() {
        let mut deps = Deps::default();
        deps.0.insert(ModId::new("c"), ids(&["b"]));
        deps.0.insert(ModId::new("b"), ids(&["a"]));

        let mut closed = deps.close(&ids(&["c"]));
        closed.sort();
        assert_eq!(closed, ids(&["a", "b", "c"]));
    }

    #[test]
    fn target_matches_the_original_issue_only() {
        let issue = |title: &str| LogIssue {
            severity: Severity::Error,
            title: title.to_string(),
            full_text: title.to_string(),
            count: 1,
            frames: Vec::new(),
            suspects: Vec::new(),
            harmony_hint: None,
        };

        let target = Target::from_issues(&[issue("боль")]);
        assert_eq!(target, Target::Issue("боль".into()));

        assert!(target.matches(&Verdict::LoadedWithErrors, &[issue("боль")]));
        // Урезанная сборка упала по своей причине — это не воспроизведение.
        assert!(!target.matches(&Verdict::Crashed { code: Some(1) }, &[issue("другая")]));
    }

    #[test]
    fn without_issues_any_failure_counts() {
        let target = Target::from_issues(&[]);
        assert_eq!(target, Target::AnyFailure);
        assert!(target.matches(&Verdict::DiedWhileLoading, &[]));
        assert!(!target.matches(&Verdict::Passed, &[]));
    }
}
