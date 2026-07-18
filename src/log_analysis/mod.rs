// Анализ логов RimWorld (Player.log): выделение ошибок/исключений
// и эвристическое определение модов-виновников.
//
// Атрибуция строится на четырёх сигналах (по убыванию веса):
//  1. путь к файлам мода в тексте ошибки  (…/Mods/<папка>/…)
//  2. неймспейс кадра стека == имя DLL из Assemblies/ мода
//  3. packageId мода упомянут в тексте
//  4. название мода упомянуто в тексте
// Кадры ванильных неймспейсов (Verse, RimWorld, UnityEngine, …) игнорируются.

mod parser;
mod attribution;

pub use parser::parse_log;
pub use attribution::ModIndex;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Error,
    Warning,
}

/// Подозреваемый мод с накопленным счётом и уликами.
#[derive(Clone, Debug)]
pub struct Suspect {
    pub package_id: String,
    pub name: String,
    pub is_active: bool,
    pub score: i32,
    pub evidence: Vec<String>,
}

/// Сгруппированная запись лога (одинаковые ошибки схлопнуты в count).
#[derive(Clone, Debug)]
pub struct LogIssue {
    pub severity: Severity,
    /// Первая строка записи (заголовок).
    pub title: String,
    /// Полный текст первого вхождения.
    pub full_text: String,
    /// Сколько раз запись встретилась в логе.
    pub count: usize,
    /// Кадры стека (без префикса "at ").
    pub frames: Vec<String>,
    pub suspects: Vec<Suspect>,
    /// Если виновник не найден, но виден след Harmony-патча —
    /// имя пропатченного метода.
    pub harmony_hint: Option<String>,
}

/// Полный конвейер: текст лога + индекс модов → готовые записи с подозреваемыми.
pub fn analyze(log_text: &str, index: &ModIndex) -> Vec<LogIssue> {
    let mut issues = parse_log(log_text);
    for issue in &mut issues {
        attribution::attribute(issue, index);
    }
    // Ошибки раньше предупреждений, дальше — по частоте
    issues.sort_by(|a, b| {
        let sev = |s: Severity| if s == Severity::Error { 0 } else { 1 };
        sev(a.severity).cmp(&sev(b.severity)).then(b.count.cmp(&a.count))
    });
    issues
}
