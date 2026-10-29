use std::path::{Path, PathBuf};
use std::sync::mpsc;
use crate::i18n::tr;
use crate::trf;

pub const RIMWORLD_APP_ID: &str = "294100";

// ─── События ─────────────────────────────────────────────────────────────────

pub enum DownloadEvent {
    Log(String),
    ItemStarted(u64),
    ItemDone(u64),
    ItemFailed(u64),
    Finished { failed: Vec<u64> },
}

pub enum InstallEvent {
    Log(String),
    Done,
    Error(String),
}

// ─── Пути ────────────────────────────────────────────────────────────────────

pub fn steamcmd_dir(base: &Path) -> PathBuf {
    base.join("steamcmd")
}

pub fn steamcmd_executable(base: &Path) -> PathBuf {
    let dir = steamcmd_dir(base);
    if cfg!(target_os = "windows") {
        dir.join("steamcmd.exe")
    } else {
        dir.join("steamcmd.sh")
    }
}

/// Папка, куда SteamCMD скачивает моды Workshop:
/// `{base}/steam/steamapps/workshop/content/294100/`
pub fn steam_content_path(base: &Path) -> PathBuf {
    base.join("steam")
        .join("steamapps")
        .join("workshop")
        .join("content")
        .join(RIMWORLD_APP_ID)
}

pub fn is_installed(base: &Path) -> bool {
    if is_nixos() {
        find_system_steamcmd().is_some()
    } else {
        steamcmd_executable(base).exists()
    }
}

/// Возвращает `true` на NixOS (по наличию `/etc/NIXOS`).
pub fn is_nixos() -> bool {
    std::path::Path::new("/etc/NIXOS").exists()
}

/// Ищет системный бинарник `steamcmd` (nixpkgs) в PATH.
fn find_system_steamcmd() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("steamcmd");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

// ─── Установка ───────────────────────────────────────────────────────────────

pub fn install_async(base: PathBuf, tx: mpsc::Sender<InstallEvent>) {
    std::thread::spawn(move || {
        if let Err(e) = run_install(&base, &tx) {
            let _ = tx.send(InstallEvent::Error(e.to_string()));
        }
    });
}

fn run_install(base: &Path, tx: &mpsc::Sender<InstallEvent>) -> anyhow::Result<()> {
    let install_dir = steamcmd_dir(base);
    std::fs::create_dir_all(&install_dir)?;

    let url = if cfg!(target_os = "windows") {
        "https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip"
    } else if cfg!(target_os = "macos") {
        "https://steamcdn-a.akamaihd.net/client/installer/steamcmd_osx.tar.gz"
    } else {
        "https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz"
    };

    let _ = tx.send(InstallEvent::Log(trf!("Загрузка: {url}", url)));

    let bytes = download_bytes(url)?;

    let _ = tx.send(InstallEvent::Log(trf!("Распаковка ({mb} МБ)...", mb = bytes.len() / 1_048_576)));

    if cfg!(target_os = "windows") {
        extract_zip(&bytes, &install_dir)?;
    } else {
        extract_tar_gz(&bytes, &install_dir)?;
        set_executable(&steamcmd_executable(base));
    }

    if steamcmd_executable(base).exists() {
        let _ = tx.send(InstallEvent::Log(tr("SteamCMD успешно установлен.").into()));
        let _ = tx.send(InstallEvent::Done);
    } else {
        return Err(anyhow::anyhow!("{}", tr("Исполняемый файл не найден после распаковки")));
    }
    Ok(())
}

fn download_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|e| anyhow::anyhow!("{}", trf!("HTTP ошибка: {e}", e)))?;
    // Архив SteamCMD — несколько десятков МБ; лимит тела по умолчанию (10 МБ) мал.
    let buf = response
        .body_mut()
        .with_config()
        .limit(512 * 1024 * 1024)
        .read_to_vec()?;
    Ok(buf)
}

fn extract_tar_gz(bytes: &[u8], dest: &Path) -> anyhow::Result<()> {
    use flate2::read::GzDecoder;
    use tar::Archive;
    let gz = GzDecoder::new(std::io::Cursor::new(bytes));
    let mut archive = Archive::new(gz);
    archive.unpack(dest)?;
    Ok(())
}

fn extract_zip(bytes: &[u8], dest: &Path) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    archive.extract(dest)?;
    Ok(())
}

#[allow(unused_variables)]
fn set_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.exists() {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
        }
    }
}

// ─── Чтение вывода процесса ───────────────────────────────────────────────────

/// SteamCMD пишет прогресс через `\r` без `\n`. Стандартный `BufReader::lines()`
/// ждёт `\n` и не отдаёт данные — pipe-буфер переполняется, процесс виснет.
/// Эта функция читает поток и разбивает по обоим разделителям (`\n` и `\r`).
fn drain_output<R: std::io::Read>(reader: &mut std::io::BufReader<R>, mut on_line: impl FnMut(String)) {
    use std::io::BufRead;
    let mut pending: Vec<u8> = Vec::with_capacity(256);
    loop {
        let n = {
            let buf = match reader.fill_buf() {
                Ok(b) => b,
                Err(_) => break,
            };
            if buf.is_empty() {
                break; // EOF
            }
            let n = buf.len();
            for &byte in buf {
                if byte == b'\n' || byte == b'\r' {
                    if !pending.is_empty() {
                        on_line(String::from_utf8_lossy(&pending).into_owned());
                        pending.clear();
                    }
                } else {
                    pending.push(byte);
                }
            }
            n
        };
        reader.consume(n);
    }
    // Последняя строка без завершающего разделителя
    if !pending.is_empty() {
        on_line(String::from_utf8_lossy(&pending).into_owned());
    }
}

// ─── Скачивание модов ─────────────────────────────────────────────────────────

/// Разбивает список ID на `n` примерно равных частей.
fn split_chunks(ids: Vec<u64>, n: usize) -> Vec<Vec<u64>> {
    let mut chunks: Vec<Vec<u64>> = (0..n).map(|_| Vec::new()).collect();
    for (i, id) in ids.into_iter().enumerate() {
        chunks[i % n].push(id);
    }
    chunks.into_iter().filter(|c| !c.is_empty()).collect()
}

/// Запускает несколько параллельных процессов SteamCMD.
/// Каждый воркер получает изолированную папку `steam_worker_{i}` во избежание
/// конфликтов lock-файлов SteamCMD. После завершения агрегатор переносит
/// скачанный контент в канонический путь `{base}/steam/`.
/// Итоговый `Finished` отправляется только когда завершатся все процессы.
pub fn download_mods_multi_async(
    base: PathBuf,
    ids: Vec<u64>,
    validate: bool,
    max_processes: usize,
    tx: mpsc::Sender<DownloadEvent>,
) {
    let n = max_processes.clamp(1, ids.len());
    let chunks = split_chunks(ids, n);
    let worker_count = chunks.len();

    // Папки воркеров — изолированы, чтобы SteamCMD не конфликтовали по lock-файлам
    let worker_bases: Vec<PathBuf> = (0..worker_count)
        .map(|i| base.join(format!("steam_worker_{i}")))
        .collect();

    let (inner_tx, inner_rx) = mpsc::channel::<DownloadEvent>();

    for (chunk, worker_base) in chunks.into_iter().zip(worker_bases.iter().cloned()) {
        let worker_tx = inner_tx.clone();
        let base_clone = base.clone();
        std::thread::spawn(move || {
            let failed = download_with_retry(&worker_base, &chunk, validate, &worker_tx);
            // Переносим скачанный контент в канонический путь {base}/steam/
            let src = steam_content_path(&worker_base);
            let dst = steam_content_path(&base_clone);
            if src.is_dir() {
                if let Err(e) = move_dir_contents(&src, &dst) {
                    let _ = worker_tx.send(DownloadEvent::Log(
                        trf!("⚠ Не удалось переместить контент воркера: {e}", e)
                    ));
                }
                let _ = std::fs::remove_dir_all(worker_base);
            }
            let _ = worker_tx.send(DownloadEvent::Finished { failed });
        });
    }
    drop(inner_tx); // канал закроется когда все воркеры завершатся

    std::thread::spawn(move || {
        let mut finished = 0;
        let mut all_failed: Vec<u64> = Vec::new();
        for ev in inner_rx {
            match ev {
                DownloadEvent::Finished { failed } => {
                    finished += 1;
                    all_failed.extend(failed);
                    if finished == worker_count {
                        let _ = tx.send(DownloadEvent::Finished { failed: all_failed });
                        return;
                    }
                    // Промежуточные Finished не пересылаем — только финальный
                }
                other => {
                    let _ = tx.send(other);
                }
            }
        }
        // Канал закрылся раньше времени — всё равно отправляем Finished
        if finished < worker_count {
            let _ = tx.send(DownloadEvent::Finished { failed: all_failed });
        }
    });
}

/// Перемещает содержимое папки `src` в `dst` (по модам верхнего уровня).
///
/// Уже лежащая в `dst` версия заменяется: раньше она оставалась, а только
/// что скачанное обновление удалялось вместе с папкой воркера.
fn move_dir_contents(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        crate::fs_util::replace_dir(&entry.path(), &dst.join(entry.file_name()))?;
    }
    Ok(())
}

pub fn download_mods_async(
    base: PathBuf,
    ids: Vec<u64>,
    validate: bool,
    tx: mpsc::Sender<DownloadEvent>,
) {
    std::thread::spawn(move || {
        let failed = download_with_retry(&base, &ids, validate, &tx);
        let _ = tx.send(DownloadEvent::Finished { failed });
    });
}

/// Скачивает моды и один раз повторяет неудавшиеся.
///
/// SteamCMD временами отказывается качать мод, а со второй попытки берёт его
/// без вопросов — главная жалоба пользователей RimSort. Повтор идёт с
/// `validate`: он же чинит недокачанные в первый раз файлы.
/// Возвращает моды, которые не скачались и после повтора.
fn download_with_retry(
    base: &Path,
    ids: &[u64],
    validate: bool,
    tx: &mpsc::Sender<DownloadEvent>,
) -> Vec<u64> {
    let failed = match run_download(base, ids, validate, tx) {
        Ok(failed) => failed,
        Err(e) => {
            // SteamCMD не запустился — повтор ничего не изменит.
            let _ = tx.send(DownloadEvent::Log(trf!("Критическая ошибка: {e}", e)));
            return ids.to_vec();
        }
    };
    if failed.is_empty() {
        return failed;
    }
    let _ = tx.send(DownloadEvent::Log(trf!("⟳ Повтор для {count} мод(ов) с проверкой файлов…", count = failed.len())));
    match run_download(base, &failed, true, tx) {
        Ok(still) => still,
        Err(e) => {
            let _ = tx.send(DownloadEvent::Log(trf!("Критическая ошибка: {e}", e)));
            failed
        }
    }
}

/// Один запуск SteamCMD. Возвращает моды, скачивание которых SteamCMD
/// не подтвердил.
fn run_download(
    base: &Path,
    ids: &[u64],
    validate: bool,
    tx: &mpsc::Sender<DownloadEvent>,
) -> anyhow::Result<Vec<u64>> {
    // На NixOS используем системный steamcmd из nixpkgs (уже пропатчен).
    let (exe, is_system) = if is_nixos() {
        match find_system_steamcmd() {
            Some(p) => (p, true),
            None => return Err(anyhow::anyhow!("{}", tr("NixOS: steamcmd не найден в PATH.\n\
                 Установите через nixpkgs: nix-shell -p steamcmd"))),
        }
    } else {
        (steamcmd_executable(base), false)
    };
    let steam_path = base.join("steam");
    std::fs::create_dir_all(&steam_path)?;

    let _ = tx.send(DownloadEvent::Log(format!(
        "SteamCMD: {} {}",
        exe.display(),
        if is_system { tr("(системный)") } else { "" }
    )));
    let _ = tx.send(DownloadEvent::Log(trf!("Скачиваем {count} мод(ов)...", count = ids.len())));

    // ── Запуск SteamCMD (аргументы вместо скрипта — совместимо с NixOS FHS) ──
    let steam_path_str = steam_path.to_string_lossy().replace('\\', "/");
    let (mut cmd, wrapped) = if is_system {
        (std::process::Command::new(&exe), false)
    } else {
        steamcmd_command(&exe)
    };
    if wrapped {
        let _ = tx.send(DownloadEvent::Log(tr("(обёрнут в steam-run для FHS-совместимости)").into()));
    }
    cmd.arg("+force_install_dir").arg(&steam_path_str);
    cmd.arg("+login").arg("anonymous");
    for &id in ids {
        if validate {
            cmd.arg("+workshop_download_item")
                .arg(RIMWORLD_APP_ID)
                .arg(id.to_string())
                .arg("validate");
        } else {
            cmd.arg("+workshop_download_item")
                .arg(RIMWORLD_APP_ID)
                .arg(id.to_string());
        }
    }
    cmd.arg("+quit");

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("{}", trf!("Не удалось запустить SteamCMD: {e}", e)))?;

    // Отдельный поток для чтения stderr (предотвращает deadlock).
    // Используем drain_output вместо lines() — SteamCMD может выводить \r без \n.
    let tx_err = tx.clone();
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stderr);
            drain_output(&mut reader, |line| {
                for part in split_ansi_lines(&line) {
                    if !part.is_empty() {
                        let _ = tx_err.send(DownloadEvent::Log(part));
                    }
                }
            });
        });
    }

    // ── Чтение stdout и разбор прогресса ────────────────────────────────────
    // drain_output разбивает по \n И \r, исключая pipe-deadlock от \r-прогресса SteamCMD.
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("{}", tr("stdout не настроен")))?;
    let mut reader = std::io::BufReader::new(stdout);

    let mut succeeded: std::collections::HashSet<u64> = std::collections::HashSet::new();

    drain_output(&mut reader, |raw_line| {
        // SteamCMD иногда объединяет несколько событий через ANSI-коды — разбиваем дополнительно.
        for part in split_ansi_lines(&raw_line) {
            let _ = tx.send(DownloadEvent::Log(part.clone()));

            // Не используем else-if: одна часть может содержать и Success, и Downloading
            if let Some(id) = parse_downloading_id(&part) {
                let _ = tx.send(DownloadEvent::ItemStarted(id));
            }
            if let Some(id) = parse_success_id(&part) {
                succeeded.insert(id);
                let _ = tx.send(DownloadEvent::ItemDone(id));
            }
            if let Some(id) = parse_error_id(&part) {
                let _ = tx.send(DownloadEvent::ItemFailed(id));
            }
        }
    });

    let status = child.wait()?;
    if !status.success() {
        let _ = tx.send(DownloadEvent::Log(trf!("× SteamCMD завершился с ошибкой (код {code})", code = status.code().unwrap_or(-1))));
    }

    // Неудача — всё, что SteamCMD не подтвердил, а не только явные ERROR!:
    // если процесс умер посреди пачки, остаток иначе молча пропадал.
    let failed: Vec<u64> = ids.iter().copied().filter(|id| !succeeded.contains(id)).collect();
    Ok(failed)
}

// ─── NixOS / steam-run ────────────────────────────────────────────────────────

/// На NixOS бинарники ELF требуют FHS-окружения.
/// `steam-run` (пакет `steam` в nixpkgs) создаёт его.
/// Ищет `steam-run` в PATH без запуска подпроцессов.
#[cfg(target_os = "linux")]
fn find_steam_run() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("steam-run");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn find_steam_run() -> Option<PathBuf> { None }

/// Создаёт `Command` для запуска `exe` (steamcmd).
/// На Linux при наличии `steam-run` оборачивает в него (нужно для NixOS).
fn steamcmd_command(exe: &Path) -> (std::process::Command, bool) {
    if let Some(steam_run) = find_steam_run() {
        let mut cmd = std::process::Command::new(steam_run);
        cmd.arg(exe);
        (cmd, true)
    } else {
        (std::process::Command::new(exe), false)
    }
}

// ─── Очистка ANSI и разбиение строк ──────────────────────────────────────────

/// Убирает ANSI escape-последовательности вида ESC [ ... <letter>.
fn strip_ansi_codes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() { break; }
                }
            }
            // bare ESC без '[' — просто пропускаем
        } else {
            out.push(c);
        }
    }
    out
}

/// Зачищает ANSI, делит по '\r', возвращает непустые обрезанные части.
fn split_ansi_lines(raw: &str) -> Vec<String> {
    let clean = strip_ansi_codes(raw);
    clean.split('\r')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// ─── Вспомогательные парсеры вывода SteamCMD ─────────────────────────────────

fn parse_id_after(line: &str, prefix: &str) -> Option<u64> {
    let start = line.find(prefix)? + prefix.len();
    let rest = &line[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    rest[..end].parse().ok()
}

fn parse_downloading_id(line: &str) -> Option<u64> {
    parse_id_after(line, "Downloading item ")
}

fn parse_success_id(line: &str) -> Option<u64> {
    parse_id_after(line, "Success. Downloaded item ")
}

fn parse_error_id(line: &str) -> Option<u64> {
    parse_id_after(line, "ERROR! Download item ")
}
