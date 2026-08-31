//! Запущенный процесс с живым выводом.
//!
//! [`crate::job::Job`] годится для «посчитать и вернуть результат», а здесь
//! нужен поток строк по ходу дела: пока Proton разворачивает префикс и
//! поднимает игру, пользователю важно видеть, что что-то происходит.
//! Тот же механизм понадобится прогону `-quicktest`, где решение принимается
//! по строкам лога.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Сколько последних строк держим. Proton при первом запуске выдаёт тысячи
/// строк, и весь этот вывод в памяти держать незачем.
const MAX_LINES: usize = 2000;

#[derive(Default)]
struct Buffer {
    lines: VecDeque<String>,
    /// Сколько строк пришло всего, включая вытесненные.
    total: usize,
}

impl Buffer {
    fn push(&mut self, line: String) {
        self.total += 1;
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }
}

/// Процесс, чей вывод собирается в фоне.
pub struct Run {
    child: Option<Child>,
    buffer: Arc<Mutex<Buffer>>,
    started: Instant,
    status: Option<ExitStatus>,
    /// Что запускали — для показа в заголовке.
    pub command: String,
}

impl Run {
    /// Запускает команду, перехватывая stdout и stderr.
    pub fn spawn(mut cmd: Command, command: String) -> std::io::Result<Self> {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
        let mut child = cmd.spawn()?;

        let buffer = Arc::new(Mutex::new(Buffer::default()));
        if let Some(out) = child.stdout.take() {
            pump(out, Arc::clone(&buffer));
        }
        if let Some(err) = child.stderr.take() {
            pump(err, Arc::clone(&buffer));
        }

        Ok(Self {
            child: Some(child),
            buffer,
            started: Instant::now(),
            status: None,
            command,
        })
    }

    /// Проверяет, не завершился ли процесс. Возвращает `true`, если статус
    /// только что изменился.
    pub fn poll(&mut self) -> bool {
        let Some(child) = &mut self.child else { return false };
        match child.try_wait() {
            Ok(Some(status)) => {
                self.status = Some(status);
                self.child = None;
                true
            }
            Ok(None) => false,
            Err(e) => {
                self.buffer.lock().unwrap().push(format!("[ошибка ожидания: {e}]"));
                self.child = None;
                true
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn status(&self) -> Option<ExitStatus> {
        self.status
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Последние строки вывода.
    pub fn lines(&self) -> Vec<String> {
        self.buffer.lock().unwrap().lines.iter().cloned().collect()
    }

    /// Сколько строк пришло всего, включая вытесненные из буфера.
    pub fn total_lines(&self) -> usize {
        self.buffer.lock().unwrap().total
    }

    /// Ждёт появления строки, удовлетворяющей условию (для тестов).
    #[cfg(test)]
    fn wait_for(&mut self, timeout: Duration, mut pred: impl FnMut(&[String]) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.poll();
            if pred(&self.lines()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Снимает процесс.
    pub fn kill(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        // Иначе процесс остался бы зомби до выхода из приложения.
        if let Some(child) = &mut self.child {
            let _ = child.try_wait();
        }
    }
}

fn pump(stream: impl Read + Send + 'static, buffer: Arc<Mutex<Buffer>>) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            match line {
                Ok(text) => buffer.lock().unwrap().push(text),
                // Нечитаемый байт в выводе — не повод ронять поток.
                Err(_) => continue,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Run {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        Run::spawn(cmd, format!("sh -c {script}")).expect("не удалось запустить sh")
    }

    #[test]
    fn collects_stdout_and_stderr() {
        let mut run = sh("echo из-stdout; echo из-stderr >&2");
        assert!(
            run.wait_for(Duration::from_secs(5), |l| {
                l.iter().any(|x| x.contains("из-stdout")) && l.iter().any(|x| x.contains("из-stderr"))
            }),
            "вывод не собрался: {:?}",
            run.lines(),
        );
    }

    #[test]
    fn reports_exit_status() {
        let mut run = sh("exit 3");
        for _ in 0..500 {
            if run.poll() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!run.is_running());
        assert_eq!(run.status().and_then(|s| s.code()), Some(3));
    }

    #[test]
    fn keeps_only_the_tail_of_long_output() {
        let mut run = sh(&format!("seq 1 {}", MAX_LINES + 500));
        run.wait_for(Duration::from_secs(20), |l| l.len() >= MAX_LINES);
        for _ in 0..1000 {
            if run.poll() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let lines = run.lines();
        assert_eq!(lines.len(), MAX_LINES, "буфер должен быть ограничен");
        assert!(run.total_lines() > MAX_LINES, "счётчик считает все строки");
        // Хвост, а не начало: последние строки важнее.
        assert_eq!(lines.last().map(String::as_str), Some((MAX_LINES + 500).to_string().as_str()));
    }

    #[test]
    fn kill_stops_a_long_running_process() {
        let mut run = sh("sleep 30");
        assert!(run.is_running());
        run.kill();
        assert!(!run.is_running());
    }
}
