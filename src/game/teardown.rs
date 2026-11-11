//! Остановка всего, что осталось от запуска игры.
//!
//! Убить процесс-обёртку недостаточно. Цепочка запуска длинная — umu →
//! pressure-vessel → Proton → wine, — и wine переподчиняет свои процессы
//! себе, выходя из-под нашей группы. Проверено на живом прогоне: после
//! `kill(umu-run)` в системе оставались `RimWorldWin64.exe`, `wineserver`
//! и три `winedevice.exe`, и так после каждого запуска.
//!
//! Поэтому процессы ищутся по признакам принадлежности нашему запуску и
//! снимаются адресно: чужие игры в других префиксах трогать нельзя.
//!
//! Надёжнее всего — своя метка в окружении ([`TAG_VAR`]): её наследует вся
//! цепочка, вплоть до wineserver. По префиксу и пути к exe одного прогона
//! от другого не отличить, когда их два одновременно, а заодно под раздачу
//! попала бы и обычная игра игрока в том же префиксе.

use std::path::PathBuf;
use std::time::Duration;

/// Переменная окружения с меткой прогона.
pub const TAG_VAR: &str = "RUSTRIM_RUN";

/// По каким признакам узнавать процессы одного запуска.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Метка в окружении: `RUSTRIM_RUN=<метка>`. Её надо передать команде.
    Tag(String),
    /// Префикс и путь к exe — для запусков без метки.
    Install { prefix: PathBuf, exe: PathBuf },
}

impl Owner {
    fn matches(&self, environ: &[u8], cmdline: &[u8]) -> bool {
        match self {
            Owner::Tag(tag) => has_tag(environ, tag),
            Owner::Install { prefix, exe } => belongs_to(
                environ,
                cmdline,
                &prefix.to_string_lossy(),
                &exe.to_string_lossy(),
            ),
        }
    }
}

/// Есть ли в окружении процесса `RUSTRIM_RUN=<tag>` — целиком, а не как
/// начало другой метки.
pub fn has_tag(environ: &[u8], tag: &str) -> bool {
    if tag.is_empty() {
        return false;
    }
    let wanted = format!("{TAG_VAR}={tag}");
    environ.split(|&b| b == 0).any(|var| var == wanted.as_bytes())
}

/// Относится ли процесс к нашему запуску.
///
/// Звенья цепочки опознаются по-разному: обёртка — по пути к exe в
/// командной строке, Proton и wineserver — по префиксу в окружении.
pub fn belongs_to(environ: &[u8], cmdline: &[u8], prefix: &str, exe: &str) -> bool {
    if prefix.is_empty() && exe.is_empty() {
        return false;
    }
    let contains = |haystack: &[u8], needle: &str| {
        !needle.is_empty()
            && haystack
                .windows(needle.len())
                .any(|w| w == needle.as_bytes())
    };
    contains(environ, prefix) || contains(cmdline, exe) || contains(environ, exe)
}

/// Жив ли ещё хоть один процесс нашего запуска.
///
/// Нужно потому, что завершение процесса-обёртки ничего не означает:
/// проверено, что umu-run выходит с кодом 0, пока игра продолжает
/// стартовать (Prepatcher перезапускает её отдельным процессом).
pub fn any_running(owner: &Owner) -> bool {
    !find_targets(owner).is_empty()
}

/// Сколько процессорного времени набрали все процессы запуска.
///
/// По приросту видно, работает ли игра вообще: зависший на старте запуск
/// не тратит ни такта, а загрузка с молчащим логом — тратит.
pub fn cpu_time(owner: &Owner) -> Duration {
    let ticks: u64 = find_targets(owner).into_iter().map(process_ticks).sum();
    Duration::from_secs_f64(ticks as f64 / clock_ticks_per_second())
}

/// Снимает процессы запуска. Возвращает, скольких коснулись.
///
/// Сначала SIGTERM — чтобы wine успел закрыть префикс штатно, — затем
/// SIGKILL для тех, кто не ушёл.
pub fn stop(owner: &Owner) -> usize {
    let targets = find_targets(owner);
    if targets.is_empty() {
        return 0;
    }

    tracing::info!("Stopping {} leftover game process(es)", targets.len());
    signal_all(&targets, Signal::Term);
    std::thread::sleep(Duration::from_millis(1500));

    let survivors = find_targets(owner);
    if !survivors.is_empty() {
        signal_all(&survivors, Signal::Kill);
    }
    targets.len()
}

#[derive(Clone, Copy)]
enum Signal {
    Term,
    Kill,
}

#[cfg(unix)]
fn signal_all(pids: &[i32], signal: Signal) {
    let sig = match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    for &pid in pids {
        // SAFETY: kill(2) с валидным pid; ошибку (процесс уже ушёл) игнорируем.
        unsafe {
            libc::kill(pid, sig);
        }
    }
}

#[cfg(not(unix))]
fn signal_all(_pids: &[i32], _signal: Signal) {}

/// utime + stime процесса в тиках часов.
#[cfg(target_os = "linux")]
fn process_ticks(pid: i32) -> u64 {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { return 0 };
    // Имя процесса в скобках может содержать пробелы — считаем поля после него.
    let Some((_, rest)) = stat.rsplit_once(')') else { return 0 };
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // После «) » идут поля с третьего: utime — 14-е, stime — 15-е.
    let get = |n: usize| fields.get(n - 3).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    get(14) + get(15)
}

#[cfg(not(target_os = "linux"))]
fn process_ticks(_pid: i32) -> u64 {
    0
}

#[cfg(unix)]
fn clock_ticks_per_second() -> f64 {
    // SAFETY: sysconf без побочных эффектов.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if hz > 0 { hz as f64 } else { 100.0 }
}

#[cfg(not(unix))]
fn clock_ticks_per_second() -> f64 {
    100.0
}

/// Ищет в /proc процессы, относящиеся к нашему запуску.
#[cfg(target_os = "linux")]
fn find_targets(owner: &Owner) -> Vec<i32> {
    let own = std::process::id() as i32;
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        let Ok(pid) = name.parse::<i32>() else { continue };
        if pid == own {
            continue;
        }
        let environ = std::fs::read(entry.path().join("environ")).unwrap_or_default();
        let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
        if owner.matches(&environ, &cmdline) {
            out.push(pid);
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn find_targets(_owner: &Owner) -> Vec<i32> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "/home/user/prefixes/rimworld";
    const EXE: &str = "/games/RimWorld/RimWorldWin64.exe";

    fn env(vars: &[&str]) -> Vec<u8> {
        vars.join("\0").into_bytes()
    }

    #[test]
    fn matches_wine_process_by_prefix_in_environment() {
        // wineserver виден только по WINEPREFIX — в командной строке
        // ни игры, ни префикса нет.
        let environ = env(&["WINEPREFIX=/home/user/prefixes/rimworld", "HOME=/home/user"]);
        assert!(belongs_to(&environ, b"wineserver\0", PREFIX, EXE));
    }

    #[test]
    fn matches_wrapper_by_exe_in_command_line() {
        let cmdline = b"umu-run\0/games/RimWorld/RimWorldWin64.exe\0".as_slice();
        assert!(belongs_to(b"", cmdline, PREFIX, EXE));
    }

    #[test]
    fn does_not_match_another_prefix() {
        // Чужая игра в соседнем префиксе не должна пострадать.
        let environ = env(&["WINEPREFIX=/home/user/prefixes/otherGame"]);
        assert!(!belongs_to(&environ, b"wineserver\0", PREFIX, EXE));
    }

    #[test]
    fn does_not_match_unrelated_process() {
        let environ = env(&["HOME=/home/user", "PATH=/usr/bin"]);
        assert!(!belongs_to(&environ, b"firefox\0", PREFIX, EXE));
    }

    #[test]
    fn empty_targets_match_nothing() {
        // Иначе пустая строка совпала бы с любым процессом.
        let environ = env(&["WINEPREFIX=/whatever"]);
        assert!(!belongs_to(&environ, b"anything\0", "", ""));
    }

    #[test]
    fn a_tag_matches_only_its_own_run() {
        // Два прогона одновременно: путь к игре у них общий, метка — нет.
        let first = env(&["HOME=/home/user", "RUSTRIM_RUN=42-1", "WINEPREFIX=/p"]);
        let second = env(&["RUSTRIM_RUN=42-12"]);
        assert!(has_tag(&first, "42-1"));
        assert!(!has_tag(&second, "42-1"), "метка не должна совпадать по началу");
        assert!(!has_tag(&first, ""));

        let owner = Owner::Tag("42-1".into());
        let cmdline = b"umu-run\0/games/RimWorld/RimWorldWin64.exe\0".as_slice();
        assert!(owner.matches(&first, cmdline));
        assert!(!owner.matches(&second, cmdline));
    }

    #[test]
    fn own_process_ticks_are_readable() {
        // Хоть сколько-то CPU тест-процесс уже потратил.
        if cfg!(target_os = "linux") {
            let mut x = 0u64;
            for i in 0..5_000_000u64 { x = x.wrapping_add(i * i); }
            assert!(x > 0);
            assert!(process_ticks(std::process::id() as i32) > 0);
        }
    }

    #[test]
    fn prefix_as_part_of_a_longer_path_still_matches() {
        // Proton кладёт свой pfx внутрь нашего префикса.
        let environ = env(&["STEAM_COMPAT_DATA_PATH=/home/user/prefixes/rimworld/pfx"]);
        assert!(belongs_to(&environ, b"proton\0", PREFIX, EXE));
    }
}
