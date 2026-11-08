// Анализ логов RimWorld (Player.log): выделение ошибок/исключений
// и эвристическое определение модов-виновников.
//
// Атрибуция строится на сигналах (по убыванию веса):
//  1. Harmony-патч на кадре, где упало исключение   (- TRANSPILER owner: …)
//  2. верхний кадр стека из кода мода               (неймспейс == DLL мода)
//  3. путь к файлам мода в тексте ошибки            (…/Mods/<папка>/…)
//  4. «[Имя мода] Patch operation … failed»
//  5. тип из DLL мода, названный в сообщении
//  6. packageId или название мода в тексте
// Кадры ванильных неймспейсов (Verse, RimWorld, UnityEngine, …) сами по себе
// не улика — но патчи модов на них улика, и сильная.
//
// Стек одного и того же исключения мод Harmony печатает только при первом
// появлении (`[Ref ABC]`), дальше — `[Ref ABC] Duplicate stacktrace`. Поэтому
// записи группируются по Ref, а дубликатам достаётся стек оригинала.

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
    pub package_id: crate::mod_data::ModId,
    pub name: String,
    pub is_active: bool,
    pub score: i32,
    pub evidence: Vec<String>,
}

/// Harmony-патч, которым мод Harmony подписывает кадр стека:
/// `    - TRANSPILER PerformanceOptimizer.Main: IEnumerable`1 Ns.Class:Method(…)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    /// PREFIX, POSTFIX, TRANSPILER, FINALIZER или ILMANIPULATOR.
    pub kind: String,
    /// Harmony-id владельца (`net.pardeike.rimworld.mod.camera+`).
    pub owner: String,
    /// Класс с патчем (`CameraPlus.Root_OnGUI_Patch`) — по нему видна DLL.
    pub class: String,
}

/// Кадр стека.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// `Ns.Class.Method` без аргументов и смещений.
    pub method: String,
    /// Метод подменён Harmony: у такого кадра нет IL-смещения (`<0x… + 0x…>`).
    pub patched: bool,
    pub patches: Vec<Patch>,
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
    /// Кадры стека, сверху вниз. У дубликата `[Ref …]` — кадры оригинала.
    pub frames: Vec<Frame>,
    /// Тип исключения (`System.NullReferenceException`), если запись о нём.
    pub exception: Option<String>,
    /// Причина ошибки в виде, который переживает перезапуск игры: без имён
    /// пешек, координат и адресов. По ней поиск виновника узнаёт «ту же
    /// ошибку» в следующих прогонах (см. `parser::cause_key`).
    pub cause: String,
    pub suspects: Vec<Suspect>,
    /// Если виновник не найден, но виден след Harmony-патча —
    /// имя пропатченного метода.
    pub harmony_hint: Option<String>,
}

/// Ванильные и системные неймспейсы — их код сам по себе не улика.
pub(crate) const VANILLA: &[&str] = &[
    "system", "verse", "rimworld", "unityengine", "unity", "harmonylib",
    "mono", "microsoft", "tmpro", "steamworks", "ludeontk", "mscorlib",
    "0harmony", "ionic", "newtonsoft", "runtimeaudioclip", "object",
    "harmonymod",
];

/// Принадлежит ли метод игре или среде, а не моду.
pub(crate) fn is_vanilla_method(path: &str) -> bool {
    let root = path.split(['.', ':', '+']).next().unwrap_or("").to_lowercase();
    root.is_empty() || VANILLA.contains(&root.as_str())
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
