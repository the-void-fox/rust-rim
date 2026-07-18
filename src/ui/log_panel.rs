use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;

use egui::{Frame, Margin, RichText, Stroke};

use crate::app::theme;
use crate::log_analysis::{analyze, LogIssue, ModIndex, Severity};
use crate::mod_data::ModEntry;

/// Логи больше этого размера читаются с хвоста (старое обрезается).
const MAX_LOG_BYTES: usize = 32 * 1024 * 1024;

enum State {
    Idle,
    Working(mpsc::Receiver<Result<Vec<LogIssue>, String>>),
    Done(Vec<LogIssue>),
    Error(String),
}

/// Окно «Анализ логов»: разбирает Player.log и показывает ошибки
/// с предполагаемыми модами-виновниками.
pub struct LogPanel {
    path: Option<PathBuf>,
    state: State,
    only_with_suspects: bool,
    show_warnings: bool,
    expanded: HashSet<usize>,
    auto_started: bool,
}

impl LogPanel {
    pub fn new() -> Self {
        Self {
            path: None,
            state: State::Idle,
            only_with_suspects: false,
            show_warnings: true,
            expanded: HashSet::new(),
            auto_started: false,
        }
    }

    /// Возвращает package_id мода, если пользователь кликнул по подозреваемому.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        mods: &[ModEntry],
        saved_path: &mut String,
    ) -> Option<String> {
        if !*open {
            return None;
        }

        // Первое открытие: сохранённый путь или автопоиск
        if self.path.is_none() {
            if !saved_path.is_empty() && std::path::Path::new(saved_path.as_str()).exists() {
                self.path = Some(PathBuf::from(saved_path.clone()));
            } else {
                self.path = default_log_candidates().into_iter().find(|p| p.exists());
            }
        }
        if !self.auto_started {
            self.auto_started = true;
            if self.path.is_some() {
                self.start_analysis(mods);
            }
        }

        self.poll();
        if matches!(self.state, State::Working(_)) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let mut selected: Option<String> = None;
        egui::Window::new("📜  Анализ логов RimWorld")
            .open(open)
            .collapsible(false)
            .resizable(true)
            .min_width(760.0)
            .min_height(480.0)
            .frame(
                Frame::window(&ctx.global_style())
                    .fill(theme::BG_PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)),
            )
            .show(ctx, |ui| {
                selected = self.content(ui, mods, saved_path);
            });
        selected
    }

    fn poll(&mut self) {
        if let State::Working(rx) = &self.state {
            if let Ok(res) = rx.try_recv() {
                self.state = match res {
                    Ok(issues) => State::Done(issues),
                    Err(e) => State::Error(e),
                };
            }
        }
    }

    fn start_analysis(&mut self, mods: &[ModEntry]) {
        let Some(path) = self.path.clone() else { return };
        let mods_snapshot: Vec<ModEntry> = mods.to_vec();
        let (tx, rx) = mpsc::channel();
        self.state = State::Working(rx);
        self.expanded.clear();
        std::thread::spawn(move || {
            let res = (|| -> Result<Vec<LogIssue>, String> {
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("Не удалось прочитать {}: {e}", path.display()))?;
                let start = bytes.len().saturating_sub(MAX_LOG_BYTES);
                let text = String::from_utf8_lossy(&bytes[start..]);
                // Индекс строится здесь же: скан Assemblies/ — дисковый ввод-вывод
                let index = ModIndex::build(&mods_snapshot);
                Ok(analyze(&text, &index))
            })();
            let _ = tx.send(res);
        });
    }

    fn content(
        &mut self,
        ui: &mut egui::Ui,
        mods: &[ModEntry],
        saved_path: &mut String,
    ) -> Option<String> {
        let mut selected = None;

        // ── Панель управления ────────────────────────────────────────────
        ui.horizontal(|ui| {
            let path_label = self.path.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "лог не найден — выберите файл".to_string());
            ui.label(RichText::new("Файл:").color(theme::TEXT_MUTED).size(11.0));
            ui.add(
                egui::Label::new(RichText::new(&path_label).color(theme::TEXT_PRIMARY).size(11.0))
                    .truncate(),
            ).on_hover_text(&path_label);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let busy = matches!(self.state, State::Working(_));
                if busy {
                    ui.spinner();
                }
                if ui.add_enabled(!busy && self.path.is_some(), egui::Button::new("⟳ Обновить"))
                    .clicked()
                {
                    self.start_analysis(mods);
                }
                if ui.button("📂 Выбрать…").clicked() {
                    let mut dlg = rfd::FileDialog::new()
                        .add_filter("Логи", &["log", "txt"]);
                    if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
                        dlg = dlg.set_directory(dir);
                    }
                    if let Some(picked) = dlg.pick_file() {
                        *saved_path = picked.display().to_string();
                        self.path = Some(picked);
                        self.start_analysis(mods);
                    }
                }
            });
        });

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.only_with_suspects,
                RichText::new("Только с подозреваемыми").size(11.0));
            ui.checkbox(&mut self.show_warnings,
                RichText::new("Показывать предупреждения").size(11.0));

            if let State::Done(issues) = &self.state {
                let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
                let warns = issues.len() - errors;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("✕ {errors}")).color(theme::ERROR_RED).size(11.0));
                    ui.label(RichText::new(format!("⚠ {warns}")).color(theme::WARNING_AMBER).size(11.0));
                });
            }
        });

        ui.add_space(4.0);
        ui.separator();

        // ── Результаты ───────────────────────────────────────────────────
        match &self.state {
            State::Idle => {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Выберите файл лога (Player.log)")
                        .color(theme::TEXT_MUTED).italics());
                });
            }
            State::Working(_) => {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Анализ лога…").color(theme::TEXT_MUTED));
                });
            }
            State::Error(e) => {
                ui.add_space(20.0);
                ui.colored_label(theme::ERROR_RED, e);
            }
            State::Done(issues) => {
                let issues: Vec<(usize, &LogIssue)> = issues.iter().enumerate()
                    .filter(|(_, i)| self.show_warnings || i.severity == Severity::Error)
                    .filter(|(_, i)| !self.only_with_suspects || !i.suspects.is_empty())
                    .collect();

                if issues.is_empty() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Ошибок не найдено 🎉")
                            .color(theme::ACTIVE_GREEN));
                    });
                } else {
                    let mut toggle: Option<usize> = None;
                    egui::ScrollArea::vertical()
                        .id_salt("log_issues_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (idx, issue) in issues {
                                if let Some(pid) = draw_issue(
                                    ui, idx, issue,
                                    self.expanded.contains(&idx),
                                    &mut toggle,
                                ) {
                                    selected = Some(pid);
                                }
                                ui.add_space(4.0);
                            }
                        });
                    if let Some(idx) = toggle {
                        if !self.expanded.remove(&idx) {
                            self.expanded.insert(idx);
                        }
                    }
                }
            }
        }

        selected
    }
}

/// Рисует одну запись; возвращает package_id при клике на подозреваемого.
fn draw_issue(
    ui: &mut egui::Ui,
    idx: usize,
    issue: &LogIssue,
    expanded: bool,
    toggle: &mut Option<usize>,
) -> Option<String> {
    let mut selected = None;

    let (mark, mark_color) = match issue.severity {
        Severity::Error => ("✕", theme::ERROR_RED),
        Severity::Warning => ("⚠", theme::WARNING_AMBER),
    };

    Frame::new()
        .fill(theme::BG_ROW_EVEN)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            ui.horizontal(|ui| {
                ui.label(RichText::new(mark).color(mark_color).size(12.0));
                if issue.count > 1 {
                    ui.label(RichText::new(format!("×{}", issue.count))
                        .color(theme::TEXT_ACCENT).size(11.0).strong());
                }
                let expand_label = if expanded { "▲" } else { "▼" };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(expand_label).clicked() {
                        *toggle = Some(idx);
                    }
                    if expanded && ui.small_button("📋").on_hover_text("Скопировать текст").clicked() {
                        ui.ctx().copy_text(issue.full_text.clone());
                    }
                });
            });

            ui.add(
                egui::Label::new(RichText::new(&issue.title)
                    .color(theme::TEXT_PRIMARY).size(11.5))
                    .truncate(),
            ).on_hover_text(&issue.title);

            // Подозреваемые
            if !issue.suspects.is_empty() {
                ui.add_space(3.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Подозреваемые:")
                        .color(theme::TEXT_MUTED).size(10.5));
                    for s in &issue.suspects {
                        let color = if s.score >= 5 {
                            theme::ERROR_RED
                        } else if s.score >= 3 {
                            theme::WARNING_AMBER
                        } else {
                            theme::TEXT_MUTED
                        };
                        let label = if s.is_active {
                            s.name.clone()
                        } else {
                            format!("{} (неактивен)", s.name)
                        };
                        let btn = egui::Button::new(
                            RichText::new(label).color(color).size(10.5))
                            .fill(theme::BG_DARK)
                            .stroke(Stroke::new(1.0, color.gamma_multiply(0.4)));
                        let tooltip = format!(
                            "{}\nсчёт: {}\n{}",
                            s.package_id, s.score, s.evidence.join("\n"),
                        );
                        if ui.add(btn).on_hover_text(tooltip).clicked() {
                            selected = Some(s.package_id.clone());
                        }
                    }
                });
            } else if let Some(hint) = &issue.harmony_hint {
                ui.add_space(3.0);
                ui.label(RichText::new(format!(
                    "След Harmony-патча: виновник среди модов, патчащих {hint}"))
                    .color(theme::TEXT_MUTED).size(10.5).italics());
            }

            if expanded {
                ui.add_space(4.0);
                Frame::new()
                    .fill(theme::BG_DARK)
                    .inner_margin(Margin::symmetric(6, 5))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt(("issue_text", idx))
                            .max_height(240.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&issue.full_text)
                                            .color(theme::TEXT_PRIMARY)
                                            .size(10.5)
                                            .monospace(),
                                    )
                                    .wrap(),
                                );
                            });
                    });
            }
        });

    selected
}

/// Стандартные расположения Player.log по платформам.
fn default_log_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let unity = ".config/unity3d/Ludeon Studios/RimWorld by Ludeon Studios";
        v.push(home.join(unity).join("Player.log"));
        v.push(home.join(unity).join("Player-prev.log"));
        // Steam Proton
        let proton = "steamapps/compatdata/294100/pfx/drive_c/users/steamuser/AppData/LocalLow/Ludeon Studios/RimWorld by Ludeon Studios/Player.log";
        v.push(home.join(".steam/steam").join(proton));
        v.push(home.join(".local/share/Steam").join(proton));
        // macOS
        v.push(home.join("Library/Logs/Ludeon Studios/RimWorld by Ludeon Studios/Player.log"));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        v.push(profile.join("AppData/LocalLow/Ludeon Studios/RimWorld by Ludeon Studios/Player.log"));
    }
    v
}
