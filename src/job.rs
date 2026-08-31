//! Одноразовая фоновая задача: посчитать в отдельном потоке, забрать результат
//! в кадре UI не блокируя его.
//!
//! До этого каждая панель заводила свой `mpsc` и свой enum состояний
//! (превью, анализ логов), и каждая по-своему обрабатывала обрыв канала.
//!
//! Задачи, которые не заканчиваются одним результатом, а **потоково** шлют
//! события (процессы SteamCMD, а в будущем — запуск игры и прогон тестов),
//! под этот тип не подходят: им нужен поток событий и отмена. Это отдельная
//! абстракция, и она появится вместе со своим первым потребителем.

use std::sync::mpsc::{self, Receiver, TryRecvError};

pub enum Job<T> {
    Idle,
    Running(Receiver<Result<T, String>>),
    Done(T),
    Failed(String),
}

impl<T> Default for Job<T> {
    fn default() -> Self {
        Job::Idle
    }
}

impl<T: Send + 'static> Job<T> {
    /// Запускает работу в фоновом потоке.
    pub fn spawn(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        Job::Running(rx)
    }

    /// Забирает результат, если он готов. Возвращает `true`, если состояние
    /// изменилось — по этому признаку UI решает, нужна ли перерисовка.
    pub fn poll(&mut self) -> bool {
        let Job::Running(rx) = self else { return false };
        match rx.try_recv() {
            Ok(Ok(value)) => {
                *self = Job::Done(value);
                true
            }
            Ok(Err(err)) => {
                *self = Job::Failed(err);
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                *self = Job::Failed("фоновая задача прервалась".to_string());
                true
            }
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, Job::Running(_))
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Job::Idle)
    }

    pub fn result(&self) -> Option<&T> {
        match self {
            Job::Done(v) => Some(v),
            _ => None,
        }
    }

    /// Извлекает результат, переводя задачу в состояние «пусто».
    pub fn take_result(&mut self) -> Option<T> {
        if !matches!(self, Job::Done(_)) {
            return None;
        }
        match std::mem::replace(self, Job::Idle) {
            Job::Done(v) => Some(v),
            _ => unreachable!(),
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Job::Failed(e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ждёт завершения задачи, опрашивая её как это делал бы UI.
    fn settle<T: Send + 'static>(job: &mut Job<T>) {
        for _ in 0..1000 {
            if job.poll() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("задача не завершилась за отведённое время");
    }

    #[test]
    fn delivers_result() {
        let mut job = Job::spawn(|| Ok(21 * 2));
        settle(&mut job);
        assert_eq!(job.result(), Some(&42));
    }

    #[test]
    fn delivers_error() {
        let mut job: Job<i32> = Job::spawn(|| Err("не вышло".to_string()));
        settle(&mut job);
        assert_eq!(job.error(), Some("не вышло"));
        assert_eq!(job.result(), None);
    }

    #[test]
    fn panicking_work_is_reported_not_hung() {
        // Паника в потоке рвёт канал; задача обязана перейти в Failed,
        // иначе UI навсегда останется в состоянии «загрузка».
        let mut job: Job<i32> = Job::spawn(|| panic!("бум"));
        settle(&mut job);
        assert!(job.error().is_some());
    }

    #[test]
    fn take_result_empties_the_job() {
        let mut job = Job::spawn(|| Ok("готово".to_string()));
        settle(&mut job);
        assert_eq!(job.take_result().as_deref(), Some("готово"));
        assert!(job.is_idle());
        assert_eq!(job.take_result(), None);
    }

    #[test]
    fn poll_on_idle_is_noop() {
        let mut job: Job<i32> = Job::Idle;
        assert!(!job.poll());
        assert!(job.is_idle());
    }
}
