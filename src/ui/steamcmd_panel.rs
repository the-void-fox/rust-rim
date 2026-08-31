use egui::{Color32, Frame, Margin, RichText, ScrollArea, Stroke};
use std::sync::mpsc;

use crate::ui::theme;
use crate::steam::steamcmd;
use crate::ui::fit_width;

const LOG_MAX: usize = 300;

// ─── Состояние панели ─────────────────────────────────────────────────────────

enum State {
    Idle,
    Installing {
        rx: mpsc::Receiver<steamcmd::InstallEvent>,
    },
    Downloading {
        total: usize,
        completed: usize,
        failed: Vec<u64>,
        rx: mpsc::Receiver<steamcmd::DownloadEvent>,
    },
    Done {
        completed: usize,
        failed: Vec<u64>,
        /// true если авто-перенос уже был инициирован (чтобы не срабатывало дважды)
        rescan_triggered: bool,
    },
}

// ─── Панель ───────────────────────────────────────────────────────────────────

pub struct SteamCmdPanel {
    ids_input: String,
    validate: bool,
    log: Vec<String>,
    state: State,
}

impl SteamCmdPanel {
    pub fn new() -> Self {
        Self {
            ids_input: String::new(),
            validate: false,
            log: Vec::new(),
            state: State::Idle,
        }
    }

    /// Добавляет Workshop ID в поле ввода (вызывается из браузера Workshop).
    pub fn add_ids(&mut self, ids: &[u64]) {
        for id in ids {
            if !self.ids_input.is_empty() {
                self.ids_input.push('\n');
            }
            self.ids_input.push_str(&id.to_string());
        }
    }

    /// Отрисовывает окно панели.
    /// Возвращает `true`, когда скачивание завершено и нужно пересканировать моды.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        steamcmd_base: &str,
        auto_move: bool,
        multi_enabled: bool,
        max_processes: usize,
        multi_threshold: usize,
    ) -> bool {
        self.poll(ctx, steamcmd_base);

        let was_open = *open;
        let mut rescan = false;

        egui::Window::new("⬇  Загрузка модов (SteamCMD)")
            .open(open)
            .collapsible(false)
            .resizable(true)
            .min_width(520.0)
            .min_height(420.0)
            .frame(
                Frame::window(&ctx.global_style())
                    .fill(theme::BG_PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)),
            )
            .show(ctx, |ui| {
                rescan = self.content(ui, steamcmd_base, auto_move, multi_enabled, max_processes, multi_threshold);
            });

        if was_open && !*open {
            self.ids_input.clear();
            self.state = State::Idle;
        }

        rescan
    }

    // ── Опрос каналов ─────────────────────────────────────────────────────────

    fn poll(&mut self, ctx: &egui::Context, _steamcmd_base: &str) {
        // Сначала собираем события и новое состояние без заимствования self.log
        let mut pending_logs: Vec<String> = Vec::new();
        let mut next_state: Option<State> = None;
        let mut got_event = false;

        match &mut self.state {
            State::Installing { rx } => {
                while let Ok(ev) = rx.try_recv() {
                    got_event = true;
                    match ev {
                        steamcmd::InstallEvent::Log(msg) => pending_logs.push(msg),
                        steamcmd::InstallEvent::Done => {
                            pending_logs.push("✓ SteamCMD установлен.".into());
                            next_state = Some(State::Idle);
                            break;
                        }
                        steamcmd::InstallEvent::Error(e) => {
                            pending_logs.push(format!("✕ Ошибка установки: {e}"));
                            next_state = Some(State::Idle);
                            break;
                        }
                    }
                }
            }
            State::Downloading { completed, failed, rx, .. } => {
                while let Ok(ev) = rx.try_recv() {
                    got_event = true;
                    match ev {
                        steamcmd::DownloadEvent::Log(msg) => pending_logs.push(msg),
                        steamcmd::DownloadEvent::ItemStarted(id) => {
                            pending_logs.push(format!("→ Скачиваем {id}…"));
                        }
                        steamcmd::DownloadEvent::ItemDone(id) => {
                            *completed += 1;
                            pending_logs.push(format!("✓ Скачан: {id}"));
                        }
                        steamcmd::DownloadEvent::ItemFailed(id) => {
                            failed.push(id);
                            pending_logs.push(format!("✕ Ошибка: {id}"));
                        }
                        steamcmd::DownloadEvent::Finished { failed: f } => {
                            let c = *completed;
                            let fv = f.clone();
                            let msg = if f.is_empty() {
                                "✓ Все моды успешно скачаны!".into()
                            } else {
                                format!(
                                    "⚠ Завершено. Не удалось скачать: {}",
                                    f.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(", ")
                                )
                            };
                            pending_logs.push(msg);
                            next_state = Some(State::Done { completed: c, failed: fv, rescan_triggered: false });
                            break;
                        }
                    }
                }
            }
            _ => {}
        }

        // Теперь применяем накопленные данные (borrow на self.state уже закончен)
        for msg in pending_logs {
            self.push_log(msg);
        }
        if let Some(s) = next_state {
            self.state = s;
        }

        // Держим UI обновлённым пока идёт процесс
        if got_event
            || matches!(&self.state, State::Installing { .. } | State::Downloading { .. })
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    }

    fn push_log(&mut self, msg: String) {
        self.log.push(msg);
        if self.log.len() > LOG_MAX {
            self.log.drain(..self.log.len() - LOG_MAX);
        }
    }

    // ── Содержимое окна ───────────────────────────────────────────────────────

    fn content(&mut self, ui: &mut egui::Ui, steamcmd_base: &str, auto_move: bool, multi_enabled: bool, max_processes: usize, multi_threshold: usize) -> bool {
        let base = std::path::Path::new(steamcmd_base);
        let nixos = steamcmd::is_nixos();
        let installed = if nixos {
            steamcmd::is_installed(base) // проверяет системный steamcmd
        } else {
            !steamcmd_base.is_empty() && steamcmd::is_installed(base)
        };
        let is_busy = matches!(
            &self.state,
            State::Installing { .. } | State::Downloading { .. }
        );
        let mut rescan = false;

        // ── Статус SteamCMD ──────────────────────────────────────────────────
        Frame::NONE
            .fill(theme::BG_HEADER)
            .inner_margin(Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if installed {
                        ui.label(
                            RichText::new("● SteamCMD установлен")
                                .color(theme::ACTIVE_GREEN)
                                .size(11.0),
                        );
                        if nixos {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("(системный, NixOS)")
                                    .color(theme::TEXT_MUTED)
                                    .size(10.0)
                                    .italics(),
                            );
                        } else {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(format!(
                                    "({})",
                                    steamcmd::steamcmd_executable(base).display()
                                ))
                                .color(theme::TEXT_MUTED)
                                .size(10.0)
                                .italics(),
                            );
                        }
                    } else {
                        ui.label(
                            RichText::new("○ SteamCMD не установлен")
                                .color(theme::WARNING_AMBER)
                                .size(11.0),
                        );
                        ui.add_space(8.0);
                        if nixos {
                            ui.label(
                                RichText::new("— NixOS: установите через  nix-shell -p steamcmd  или добавьте steamcmd в systemPackages")
                                    .color(theme::TEXT_MUTED)
                                    .size(10.5)
                                    .italics(),
                            );
                        } else if steamcmd_base.is_empty() {
                            ui.label(
                                RichText::new("— укажите папку в Настройки → Пути")
                                    .color(theme::TEXT_MUTED)
                                    .size(10.5)
                                    .italics(),
                            );
                        } else if !is_busy {
                            let btn = egui::Button::new(
                                RichText::new("  Установить  ")
                                    .color(theme::TEXT_PRIMARY)
                                    .size(11.0),
                            )
                            .fill(theme::HEADER_LEFT)
                            .stroke(Stroke::new(1.0, theme::BORDER_ACCENT));
                            if ui.add(btn).clicked() {
                                let (tx, rx) = mpsc::channel();
                                steamcmd::install_async(base.to_path_buf(), tx);
                                self.log.clear();
                                self.state = State::Installing { rx };
                            }
                        }
                    }
                    if matches!(&self.state, State::Installing { .. }) {
                        ui.add_space(8.0);
                        ui.spinner();
                        ui.label(
                            RichText::new("Установка…")
                                .color(theme::TEXT_MUTED)
                                .size(11.0),
                        );
                    }
                });
            });

        ui.add_space(10.0);

        // ── Ввод Workshop ID ─────────────────────────────────────────────────
        ui.label(
            RichText::new("Workshop ID (по одному на строку, через запятую или пробел):")
                .color(theme::TEXT_MUTED)
                .size(11.0),
        );
        ui.add_space(4.0);

        let edit_enabled = installed && !is_busy;
        ScrollArea::vertical()
            .max_height(120.0)               // ← лимит высоты в пикселях (подберите под свой UI)
            .id_salt("workshop_ids_scroll")  // ← уникальный ID для сохранения позиции (опционально)
            .show(ui, |ui| {
                ui.add_enabled(
                    edit_enabled,
                    egui::TextEdit::multiline(&mut self.ids_input)
                        .desired_width(f32::INFINITY)
                        .desired_rows(4)     // всё ещё желаемая высота, но ScrollArea её обрежет
                        .font(egui::TextStyle::Monospace)
                        .hint_text("2009463077\n1507748539\n…"),
                );
            });

        ui.add_space(6.0);
        ui.checkbox(
            &mut self.validate,
            RichText::new("Проверять целостность (validate)")
                .color(theme::TEXT_PRIMARY)
                .size(11.0),
        );
        ui.add_space(8.0);

        // ── Кнопки и прогресс ────────────────────────────────────────────────
        ui.horizontal(|ui| {
            let ids = parse_ids(&self.ids_input);
            let can_download = installed && !is_busy && !ids.is_empty();

            let dl_btn = egui::Button::new(
                RichText::new("⬇  Скачать").color(theme::TEXT_PRIMARY).size(12.0),
            )
            .fill(theme::HEADER_LEFT)
            .stroke(Stroke::new(1.0, theme::BORDER_ACCENT));

            if ui.add_enabled(can_download, dl_btn).clicked() {
                let (tx, rx) = mpsc::channel();
                let total = ids.len();
                let use_multi = multi_enabled && max_processes > 1 && total >= multi_threshold;
                if use_multi {
                    let procs = max_processes.min(total);
                    steamcmd::download_mods_multi_async(base.to_path_buf(), ids, self.validate, procs, tx);
                } else {
                    steamcmd::download_mods_async(base.to_path_buf(), ids, self.validate, tx);
                }
                self.log.clear();
                if use_multi {
                    self.push_log(format!(
                        "Мульти-загрузка: {} параллельных процессов для {} мод(ов)",
                        max_processes.min(total), total
                    ));
                }
                self.state = State::Downloading {
                    total,
                    completed: 0,
                    failed: Vec::new(),
                    rx,
                };
            }

            // Прогресс во время скачивания
            if let State::Downloading { total, completed, .. } = &self.state {
                ui.add_space(8.0);
                ui.spinner();
                ui.label(
                    RichText::new(format!("{completed} / {total}"))
                        .color(theme::TEXT_MUTED)
                        .size(11.0),
                );
            }

            // Итог после завершения
            if let State::Done { completed, failed, .. } = &self.state {
                ui.add_space(8.0);
                let (text, color) = if failed.is_empty() {
                    (format!("✓ Скачано: {completed}"), theme::ACTIVE_GREEN)
                } else {
                    (
                        format!(
                            "⚠ Скачано: {completed}, ошибок: {}",
                            failed.len()
                        ),
                        theme::WARNING_AMBER,
                    )
                };
                ui.label(RichText::new(text).color(color).size(11.0));
            }
        });

        // Авто-перенос: при включённой настройке срабатывает сразу после завершения
        let trigger_auto = if let State::Done { completed, rescan_triggered, .. } = &mut self.state {
            if *completed > 0 && auto_move && !*rescan_triggered {
                *rescan_triggered = true;
                true
            } else {
                false
            }
        } else {
            false
        };
        if trigger_auto {
            self.push_log("→ Перемещаем моды в папку локальных модов…".into());
            rescan = true;
        }

        // Ручная кнопка — только когда авто-перенос выключен
        if !auto_move {
            if let State::Done { completed, .. } = &self.state {
                if *completed > 0 {
                    ui.add_space(4.0);
                    let add_btn = egui::Button::new(
                        RichText::new("↺  Перенести в Mods и обновить список")
                            .color(theme::TEXT_PRIMARY)
                            .size(11.0),
                    )
                    .fill(theme::BG_ROW_EVEN)
                    .stroke(Stroke::new(1.0, theme::BORDER_ACCENT));

                    if ui
                        .add(add_btn)
                        .on_hover_text("Переместить скачанные моды в папку RimWorld/Mods и пересканировать список")
                        .clicked()
                    {
                        rescan = true;
                        self.ids_input.clear();
                        self.state = State::Idle;
                    }
                }
            }
        }

        ui.add_space(8.0);
        ui.separator();

        // ── Лог ─────────────────────────────────────────────────────────────
        ui.add_space(4.0);
        ui.label(
            RichText::new("Вывод SteamCMD:")
                .color(theme::TEXT_MUTED)
                .size(10.5)
                .strong(),
        );
        ui.add_space(4.0);

        let avail_h = (ui.available_height() - 8.0).max(80.0);
        egui::ScrollArea::vertical()
            .id_salt("steamcmd_log_scroll")
            .max_height(avail_h)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_width(fit_width(ui));
                Frame::NONE
                    .fill(theme::BG_DARK)
                    .inner_margin(Margin::same(6))
                    .show(ui, |ui| {
                        if self.log.is_empty() {
                            ui.label(
                                RichText::new("(пусто)")
                                    .color(theme::TEXT_MUTED)
                                    .size(10.5)
                                    .italics(),
                            );
                        }
                        for line in &self.log {
                            let color = log_line_color(line);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(line)
                                        .color(color)
                                        .size(10.5)
                                        .family(egui::FontFamily::Monospace),
                                )
                                .wrap(),
                            );
                        }
                    });
            });

        rescan
    }
}

// ─── Вспомогательные функции ──────────────────────────────────────────────────

/// Разбирает строку ввода в список Workshop ID.
/// Поддерживает разделители: перенос строки, запятая, пробел, точка с запятой.
pub fn parse_ids(input: &str) -> Vec<u64> {
    input
        .split(|c: char| c == '\n' || c == ',' || c == ';' || c == ' ')
        .filter_map(|s| s.trim().parse::<u64>().ok())
        .collect()
}

fn log_line_color(line: &str) -> Color32 {
    if line.starts_with("✓") {
        theme::ACTIVE_GREEN
    } else if line.starts_with("✕") || line.contains("ERROR") || line.contains("FAILED") {
        theme::ERROR_RED
    } else if line.starts_with("⚠") || line.starts_with("→") || line.contains("Downloading") {
        theme::WARNING_AMBER
    } else {
        theme::TEXT_MUTED
    }
}