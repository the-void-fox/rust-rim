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

use std::path::Path;
use std::time::Duration;

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
pub fn any_running(prefix: &Path, exe: &Path) -> bool {
    let prefix = prefix.to_string_lossy().into_owned();
    let exe = exe.to_string_lossy().into_owned();
    !find_targets(&prefix, &exe).is_empty()
}

/// Снимает процессы запуска. Возвращает, скольких коснулись.
///
/// Сначала SIGTERM — чтобы wine успел закрыть префикс штатно, — затем
/// SIGKILL для тех, кто не ушёл.
pub fn stop(prefix: &Path, exe: &Path) -> usize {
    let prefix = prefix.to_string_lossy().into_owned();
    let exe = exe.to_string_lossy().into_owned();
    let targets = find_targets(&prefix, &exe);
    if targets.is_empty() {
        return 0;
    }

    tracing::info!("Stopping {} leftover game process(es)", targets.len());
    signal_all(&targets, Signal::Term);
    std::thread::sleep(Duration::from_millis(1500));

    let survivors = find_targets(&prefix, &exe);
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

/// Ищет в /proc процессы, относящиеся к нашему запуску.
#[cfg(target_os = "linux")]
fn find_targets(prefix: &str, exe: &str) -> Vec<i32> {
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
        if belongs_to(&environ, &cmdline, prefix, exe) {
            out.push(pid);
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn find_targets(_prefix: &str, _exe: &str) -> Vec<i32> {
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
    fn prefix_as_part_of_a_longer_path_still_matches() {
        // Proton кладёт свой pfx внутрь нашего префикса.
        let environ = env(&["STEAM_COMPAT_DATA_PATH=/home/user/prefixes/rimworld/pfx"]);
        assert!(belongs_to(&environ, b"proton\0", PREFIX, EXE));
    }
}
