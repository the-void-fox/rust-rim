use egui::{Align2, Button, Context, Frame, Margin, RichText, Stroke, Window};

use crate::game::{launch, Prefix, Runner};
use crate::job::Job;
use crate::settings::{AppSettings, SettingsTab};
use crate::ui::theme;
use crate::ui::{fit_width, fit_width_minus};
use crate::i18n::{tr, Lang};
use crate::trf;

/// Открывает нативный диалог выбора папки и возвращает путь как строку.
fn pick_folder(title: &str) -> Option<String> {
    rfd::FileDialog::new()
        .set_title(title)
        .pick_folder()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Возвращает путь к папке modlist в директории данных приложения,
/// создавая её при необходимости.
fn modlist_dir() -> Option<std::path::PathBuf> {
    let dir = directories::ProjectDirs::from("com", "rustrim", "RustRim")
        .map(|d| d.data_dir().join("modlist"))?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// Открывает нативный диалог выбора файла для чтения.
pub fn pick_open_file(title: &str) -> Option<std::path::PathBuf> {
    let mut dlg = rfd::FileDialog::new()
        .set_title(title)
        .add_filter(tr("Список модов RimWorld"), &["xml", "rml", "rws"])
        .add_filter(tr("Все файлы"), &["*"]);
    if let Some(dir) = modlist_dir() {
        dlg = dlg.set_directory(dir);
    }
    dlg.pick_file()
}

/// Открывает нативный диалог выбора файла для сохранения.
pub fn pick_save_file(title: &str) -> Option<std::path::PathBuf> {
    let mut dlg = rfd::FileDialog::new()
        .set_title(title)
        .add_filter(tr("Список модов (XML)"), &["xml"])
        .set_file_name("ModList.xml");
    if let Some(dir) = modlist_dir() {
        dlg = dlg.set_directory(dir);
    }
    dlg.save_file()
}

/// Диалог запроса путей при первом запуске.
/// Возвращает `true`, если пользователь подтвердил кнопкой «Открыть».
pub fn open_folder_dialog(ctx: &Context, open: &mut bool, settings: &mut AppSettings) -> bool {
    if !*open { return false; }

    let mut load_requested = false;

    Window::new(RichText::new(tr("▤  Настройка путей")).color(theme::TEXT_PRIMARY).size(13.0).strong())
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .min_width(480.0)
        .frame(Frame::window(&ctx.global_style())
            .fill(theme::BG_PANEL)
            .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)))
        .show(ctx, |ui| {
            ui.add_space(6.0);

            // ── Папка игры (обязательно) ────────────────────────────────
            required_label(ui, tr("Папка с игрой:"), settings.game_path.is_empty());
            ui.add_space(3.0);
            ui.label(RichText::new(tr("Корневая папка RimWorld (содержит RimWorldWin64.exe или RimWorldLinux)"))
                .color(theme::TEXT_MUTED).size(10.5).italics());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let changed = path_edit(ui, &mut settings.game_path,
                    "/path/to/RimWorld", "open_game_path");
                if ui.small_button("…").clicked() {
                    if let Some(p) = pick_folder(tr("Выберите папку игры")) {
                        settings.game_path = p;
                    }
                }
                let _ = changed;
            });

            ui.add_space(10.0);

            // ── Папка с модами (обязательно) ────────────────────────────
            required_label(ui, tr("Папка с локальными модами:"), settings.local_mods_path.is_empty());
            ui.add_space(3.0);
            ui.label(RichText::new(tr("Папка Mods/ внутри директории игры"))
                .color(theme::TEXT_MUTED).size(10.5).italics());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                path_edit(ui, &mut settings.local_mods_path,
                    "/path/to/RimWorld/Mods", "open_mods_path");
                if ui.small_button("…").clicked() {
                    if let Some(p) = pick_folder(tr("Выберите папку с модами")) {
                        settings.local_mods_path = p;
                    }
                }
            });

            ui.add_space(10.0);

            // ── Папка конфигурации (необязательно) ──────────────────────
            ui.label(RichText::new(tr("Папка с конфигурацией (необязательно):"))
                .color(theme::TEXT_PRIMARY).size(12.0).strong());
            ui.add_space(3.0);
            ui.label(RichText::new(tr("Содержит ModsConfig.xml — для загрузки и сохранения активных модов"))
                .color(theme::TEXT_MUTED).size(10.5).italics());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                path_edit(ui, &mut settings.config_path,
                    "~/.config/unity3d/.../Config", "open_config_path");
                if ui.small_button("…").clicked() {
                    if let Some(p) = pick_folder(tr("Выберите папку конфигурации")) {
                        settings.config_path = p;
                    }
                }
            });

            ui.add_space(12.0);

            ui.horizontal(|ui| {
                let can_open = !settings.game_path.is_empty() && !settings.local_mods_path.is_empty();
                let ok_color = if can_open { theme::TEXT_PRIMARY } else { theme::TEXT_MUTED };
                let ok_btn = Button::new(
                    RichText::new(tr("  Открыть  ")).color(ok_color).size(12.0)
                ).fill(theme::HEADER_LEFT).stroke(Stroke::new(1.0, theme::BORDER_ACCENT));

                let ok_resp = ui.add_enabled(can_open, ok_btn);
                if ok_resp.clicked() {
                    *open = false;
                    load_requested = true;
                }

                ui.add_space(8.0);

                let cancel_btn = Button::new(
                    RichText::new(tr("  Отмена  ")).color(theme::TEXT_MUTED).size(12.0)
                ).fill(theme::BG_ROW_EVEN).stroke(Stroke::new(1.0, theme::BORDER));

                if ui.add(cancel_btn).clicked() { *open = false; }
            });
            ui.add_space(4.0);
        });

    load_requested
}

/// Возвращает `true`, если пользователь подтвердил сохранение.
pub fn save_dialog(
    ctx: &Context,
    open: &mut bool,
    active_count: usize,
    total_count: usize,
    config_path: &str,
) -> bool {
    if !*open { return false; }

    let mut save_confirmed = false;

    Window::new(RichText::new(tr("⇩  Сохранить список модов")).color(theme::TEXT_PRIMARY).size(13.0).strong())
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .min_width(360.0)
        .frame(Frame::window(&ctx.global_style())
            .fill(theme::BG_PANEL)
            .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)))
        .show(ctx, |ui| {
            ui.add_space(6.0);

            Frame::NONE
                .fill(theme::BG_DARK)
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tr("Активных:")).color(theme::TEXT_MUTED).size(12.0));
                        ui.label(RichText::new(format!("{}", active_count))
                            .color(theme::ACTIVE_GREEN).size(12.0).strong());
                        ui.add_space(12.0);
                        ui.label(RichText::new(tr("Всего:")).color(theme::TEXT_MUTED).size(12.0));
                        ui.label(RichText::new(format!("{}", total_count))
                            .color(theme::TEXT_PRIMARY).size(12.0).strong());
                    });
                });

            ui.add_space(6.0);

            if config_path.is_empty() {
                Frame::NONE
                    .fill(theme::BG_DARK)
                    .inner_margin(Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.label(RichText::new(tr("⚠  Путь к конфигурации не задан.\nУкажите его в Настройки → Пути."))
                            .color(theme::WARNING_AMBER).size(11.0));
                    });
            } else {
                let target = format!("{}/ModsConfig.xml", config_path);
                ui.label(RichText::new(trf!("Запись в: {target}", target))
                    .color(theme::TEXT_MUTED).size(10.5).italics());
            }

            ui.add_space(8.0);

            ui.horizontal(|ui| {
                let can_save = !config_path.is_empty();
                let save_btn = Button::new(
                    RichText::new(tr("  ⇩ Сохранить  ")).color(theme::TEXT_PRIMARY).size(12.0)
                ).fill(theme::HEADER_RIGHT)
                 .stroke(Stroke::new(1.0, theme::ACTIVE_GREEN.gamma_multiply(0.5)));

                if ui.add_enabled(can_save, save_btn).clicked() {
                    *open = false;
                    save_confirmed = true;
                }

                ui.add_space(8.0);

                let cancel_btn = Button::new(
                    RichText::new(tr("  Отмена  ")).color(theme::TEXT_MUTED).size(12.0)
                ).fill(theme::BG_ROW_EVEN).stroke(Stroke::new(1.0, theme::BORDER));

                if ui.add(cancel_btn).clicked() { *open = false; }
            });
            ui.add_space(4.0);
        });

    save_confirmed
}

/// Возвращает `true`, если пользователь нажал «Применить».
pub fn settings_dialog(
    ctx: &Context,
    open: &mut bool,
    settings: &mut AppSettings,
    prefixes: &[Prefix],
) -> bool {
    if !*open { return false; }

    let mut applied = false;

    Window::new(tr("⚙ Настройки"))
        .collapsible(false)
        .resizable(true)
        .min_width(480.0)
        .min_height(300.0)
        .frame(Frame::window(&ctx.global_style())
            .fill(theme::BG_PANEL)
            .stroke(Stroke::new(1.0, theme::BORDER)))
        .show(ctx, |ui| {

            // ── Вкладки ──────────────────────────────────────────────────
            ui.horizontal(|ui| {
                tab_button(ui, tr("▤  Пути"),       settings.active_tab == SettingsTab::Paths,     || settings.active_tab = SettingsTab::Paths);
                tab_button(ui, tr("▶  Запуск"),      settings.active_tab == SettingsTab::Launch,     || settings.active_tab = SettingsTab::Launch);
                tab_button(ui, tr("❖  Интерфейс"),  settings.active_tab == SettingsTab::Interface,  || settings.active_tab = SettingsTab::Interface);
                tab_button(ui, tr("⚙  Поведение"),   settings.active_tab == SettingsTab::Behavior,   || settings.active_tab = SettingsTab::Behavior);
            });

            ui.separator();
            ui.add_space(6.0);

            match settings.active_tab {
                // ── Пути ─────────────────────────────────────────────────
                SettingsTab::Paths => {
                    path_row_required(ui, tr("Местоположение игры"),
                        tr("Корневая папка RimWorld (содержит RimWorldWin64.exe или RimWorldLinux)"),
                        &mut settings.game_path,
                        "game_path_edit",
                        true);

                    ui.add_space(10.0);

                    path_row_required(ui, tr("Местоположение локальных модов"),
                        tr("Папка Mods/ внутри директории игры или пользовательская папка"),
                        &mut settings.local_mods_path,
                        "mods_path_edit",
                        true);

                    ui.add_space(10.0);

                    path_row(ui, tr("Местоположение конфигурации"),
                        tr("Папка с ModsConfig.xml (обычно ~/AppData/LocalLow/.../Config)"),
                        &mut settings.config_path,
                        "config_path_edit");

                    ui.add_space(10.0);

                    path_row(ui, tr("Папка SteamCMD (необязательно)"),
                        tr("Базовая папка для SteamCMD; пусто — использовать папку данных приложения"),
                        &mut settings.steamcmd_path,
                        "steamcmd_path_edit");
                }

                // ── Запуск ───────────────────────────────────────────────
                SettingsTab::Launch => launch_tab(ui, settings, prefixes),

                // ── Интерфейс ────────────────────────────────────────────
                SettingsTab::Interface => {
                    section_header(ui, tr("ЯЗЫК"));
                    ui.add_space(4.0);
                    language_row(ui, settings);
                    ui.add_space(12.0);
                    section_header(ui, tr("ОПИСАНИЯ МОДОВ"));
                    ui.add_space(4.0);
                    checkbox_row(ui, &mut settings.load_remote_images,
                        tr("Загружать изображения из описаний"));
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(RichText::new(
                            tr("Картинки в описаниях лежат на сторонних хостах (CDN Steam, imgur\n\
                             и любые другие, указанные автором мода). При включении выбор мода\n\
                             в списке отправляет к ним запрос. Выключено — вместо картинки\n\
                             показывается ссылка.")
                        ).color(theme::TEXT_MUTED).size(10.5).italics());
                    });
                }

                // ── Поведение ────────────────────────────────────────────
                SettingsTab::Behavior => {
                    section_header(ui, tr("СОРТИРОВКА"));
                    ui.add_space(4.0);
                    checkbox_row(ui, &mut settings.use_community_rules,
                        tr("Использовать онлайн-базу правил сообщества"));
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(RichText::new(
                            tr("Загружает правила loadBefore/loadAfter с GitHub (RimSort Community Rules).\n\
                             Отключите при отсутствии интернета или для оффлайн-режима.")
                        ).color(theme::TEXT_MUTED).size(10.5).italics());
                    });

                    ui.add_space(12.0);
                    section_header(ui, tr("ПОИСК В МАСТЕРСКОЙ"));
                    ui.add_space(4.0);
                    text_row(ui, &mut settings.steam_api_key, "steam_api_key_edit",
                        tr("Ключ Steam Web API (необязательно)"));
                    ui.add_space(2.0);
                    api_key_check_row(ui, settings);
                    ui.add_space(2.0);
                    hint(ui,
                        tr("Без ключа поиск читает страницу мастерской, а её Steam время от времени\n\
                         переделывает. С ключом поиск идёт через официальный API и от вёрстки\n\
                         не зависит. Бесплатно: steamcommunity.com/dev/apikey (домен — любой)."));

                    ui.add_space(12.0);
                    section_header(ui, tr("ЗАГРУЗКА МОДОВ (STEAMCMD)"));
                    ui.add_space(4.0);

                    checkbox_row(ui, &mut settings.steamcmd_auto_move,
                        tr("Автоматически перемещать моды в папку локальных модов после загрузки"));
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(RichText::new(
                            tr("Моды сразу попадают в RimWorld/Mods без ручного нажатия кнопки.\n\
                             Отключите, если хотите проверить что скачалось перед добавлением.")
                        ).color(theme::TEXT_MUTED).size(10.5).italics());
                    });

                    ui.add_space(8.0);
                    checkbox_row(ui, &mut settings.steamcmd_multi_download,
                        tr("Параллельная загрузка (несколько процессов SteamCMD)"));
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(RichText::new(
                            tr("Запускает несколько процессов SteamCMD одновременно для ускорения загрузки.\n\
                             Активируется только когда число модов ≥ порогу.")
                        ).color(theme::TEXT_MUTED).size(10.5).italics());
                    });
                    ui.add_space(8.0);

                    let enabled = settings.steamcmd_multi_download;
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new(tr("Макс. параллельных процессов:"))
                                .color(theme::TEXT_PRIMARY).size(12.0));
                            ui.add_space(6.0);
                            ui.add(egui::Slider::new(&mut settings.steamcmd_max_processes, 2..=4)
                                .text(""));
                        });
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new(
                                tr("2 — стабильно для большинства систем; 3–4 только при быстром интернете.")
                            ).color(theme::TEXT_MUTED).size(10.5).italics());
                        });

                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new(tr("Порог активации (кол-во модов):"))
                                .color(theme::TEXT_PRIMARY).size(12.0));
                            ui.add_space(6.0);
                            ui.add(egui::Slider::new(&mut settings.steamcmd_multi_threshold, 2..=50)
                                .text(""));
                        });
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new(
                                tr("При меньшем числе модов используется один процесс (накладные расходы не оправданы).")
                            ).color(theme::TEXT_MUTED).size(10.5).italics());
                        });
                    });
                }
            }

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                let apply = Button::new(
                    RichText::new(tr("  Применить  ")).color(theme::TEXT_PRIMARY).size(12.0)
                ).fill(theme::HEADER_LEFT).stroke(Stroke::new(1.0, theme::BORDER_ACCENT));

                if ui.add(apply).clicked() {
                    *open = false;
                    applied = true;
                }

                ui.add_space(8.0);

                let cancel = Button::new(
                    RichText::new(tr("  Отмена  ")).color(theme::TEXT_MUTED).size(12.0)
                ).fill(theme::BG_ROW_EVEN).stroke(Stroke::new(1.0, theme::BORDER));

                if ui.add(cancel).clicked() { *open = false; }
            });
            ui.add_space(4.0);
        });

    applied
}

// ─── Вкладка «Запуск» ────────────────────────────────────────────────────────

fn launch_tab(ui: &mut egui::Ui, settings: &mut AppSettings, prefixes: &[Prefix]) {
    section_header(ui, tr("СПОСОБ ЗАПУСКА"));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        egui::ComboBox::from_id_salt("runner_combo")
            .selected_text(settings.launch.runner.label())
            .width(220.0)
            .show_ui(ui, |ui| {
                for runner in [
                    Runner::Auto, Runner::Native, Runner::Umu,
                    Runner::Wine, Runner::Steam, Runner::Custom,
                ] {
                    ui.selectable_value(&mut settings.launch.runner, runner, runner.label());
                }
            });
    });
    ui.add_space(2.0);
    hint(ui, tr("«Автоматически» — нативная сборка запускается напрямую, Windows-сборка через umu-run."));

    if settings.launch.runner == Runner::Custom {
        ui.add_space(8.0);
        section_header(ui, tr("СВОЯ КОМАНДА"));
        ui.add_space(4.0);
        text_row(ui, &mut settings.launch.custom_command, "custom_command",
            tr("например: gamescope -f -- umu-run"));
        ui.add_space(2.0);
        hint(ui, tr("Путь к игре и аргументы дописываются в конец команды."));
    }

    ui.add_space(10.0);
    section_header(ui, tr("ПРЕФИКС WINE / PROTON"));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        if ui.button("…").on_hover_text(tr("Выбрать папку")).clicked() {
            if let Some(p) = pick_folder(tr("Выберите префикс")) {
                settings.launch.prefix = p;
            }
        }
        ui.add(
            egui::TextEdit::singleline(&mut settings.launch.prefix)
                .id(egui::Id::new("prefix_edit"))
                .desired_width(fit_width_minus(ui, 12.0))
                .hint_text(tr("не задан — будет выбран найденный ниже"))
                .text_color(theme::TEXT_PRIMARY),
        );
    });

    ui.add_space(6.0);
    if prefixes.is_empty() {
        hint(ui, tr("Префиксы с данными RimWorld не найдены."));
    } else {
        hint(ui, tr("Найденные префиксы (первым — тот, в чей конфиг писали последним):"));
        ui.add_space(2.0);
        for prefix in prefixes {
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                if ui
                    .button(tr("Выбрать"))
                    .on_hover_text(tr("Подставит и пути к ModsConfig.xml и Player.log"))
                    .clicked()
                {
                    settings.adopt_prefix(prefix);
                }
                ui.label(RichText::new(format!("[{}]", prefix.source))
                    .color(theme::TEXT_ACCENT).size(10.5));
                ui.add(
                    egui::Label::new(
                        RichText::new(prefix.path.to_string_lossy())
                            .color(theme::TEXT_MUTED).size(10.5),
                    )
                    .truncate(),
                )
                .on_hover_text(prefix.data_dir.to_string_lossy());
            });
        }
    }

    ui.add_space(10.0);
    section_header(ui, tr("ВЕРСИЯ PROTON"));
    ui.add_space(4.0);
    text_row(ui, &mut settings.launch.proton, "proton_edit", "GE-Proton");
    ui.add_space(2.0);
    hint(ui, tr("Имя (GE-Proton — последний установленный) или полный путь к версии."));

    ui.add_space(10.0);
    section_header(ui, tr("ДОПОЛНИТЕЛЬНЫЕ АРГУМЕНТЫ"));
    ui.add_space(4.0);
    text_row(ui, &mut settings.launch.extra_args, "extra_args_edit", "-popupwindow");

    ui.add_space(10.0);
    section_header(ui, tr("ИТОГОВАЯ КОМАНДА"));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.set_max_width(fit_width(ui));
            match preview(settings, prefixes) {
                Ok(text) => {
                    ui.label(RichText::new(text).color(theme::TEXT_PRIMARY).size(10.5).monospace());
                }
                Err(e) => {
                    ui.label(RichText::new(format!("⚠ {e}")).color(theme::WARNING_AMBER).size(11.0));
                }
            }
        });
    });
}

/// Команда, которая выполнится при нажатии «Запустить».
fn preview(settings: &AppSettings, prefixes: &[Prefix]) -> Result<String, launch::LaunchError> {
    let mut effective = settings.launch.clone();
    if effective.prefix.trim().is_empty() {
        if let Some(p) = prefixes.first() {
            effective.prefix = p.path.to_string_lossy().into_owned();
        }
    }
    let game = std::path::Path::new(&settings.game_path);
    launch::plan(game, &effective, &launch::Mode::Play).map(|p| p.display())
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(RichText::new(text).color(theme::TEXT_MUTED).size(10.5).italics());
    });
}

/// Выбор языка интерфейса. Применяется сразу — без перезапуска.
fn language_row(ui: &mut egui::Ui, settings: &mut AppSettings) {
    let system = tr("Как в системе");
    let current = settings.language.map_or(system, Lang::native_name);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        egui::ComboBox::from_id_salt("ui_language")
            .selected_text(current)
            .show_ui(ui, |ui| {
                let mut changed = false;
                if ui.selectable_label(settings.language.is_none(), system).clicked() {
                    settings.language = None;
                    changed = true;
                }
                for lang in Lang::ALL {
                    if ui.selectable_label(settings.language == Some(lang), lang.native_name()).clicked() {
                        settings.language = Some(lang);
                        changed = true;
                    }
                }
                if changed {
                    settings.apply_language();
                }
            });
    });
}

/// Кнопка «Проверить ключ» и её итог. Запрос идёт в фоне: окно настроек
/// не должно замирать на сетевом таймауте.
fn api_key_check_row(ui: &mut egui::Ui, settings: &mut AppSettings) {
    settings.api_key_check.poll();
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let running = settings.api_key_check.is_running();
        let can_check = !running && !settings.steam_api_key.trim().is_empty();
        if ui.add_enabled(can_check, Button::new(tr("Проверить ключ"))).clicked() {
            let key = settings.steam_api_key.clone();
            settings.api_key_check = Job::spawn(move || {
                crate::steam::workshop_api::check_api_key(&key).map_err(|e| e.to_string())
            });
        }
        match &settings.api_key_check {
            Job::Running(_) => {
                ui.spinner();
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            Job::Done(total) => {
                ui.label(RichText::new(trf!("✓ Ключ работает, модов в мастерской: {total}", total))
                    .color(theme::ACTIVE_GREEN).size(11.0));
            }
            Job::Failed(err) => {
                ui.label(RichText::new(format!("× {err}")).color(theme::WARNING_AMBER).size(11.0));
            }
            Job::Idle => {}
        }
    });
}

fn text_row(ui: &mut egui::Ui, value: &mut String, id: &str, hint_text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.add(
            egui::TextEdit::singleline(value)
                .id(egui::Id::new(id))
                .desired_width(fit_width_minus(ui, 12.0))
                .hint_text(hint_text)
                .text_color(theme::TEXT_PRIMARY),
        );
    });
}

// ─── Вспомогательные виджеты ─────────────────────────────────────────────────

fn tab_button(ui: &mut egui::Ui, label: &str, active: bool, on_click: impl FnOnce()) {
    let fill   = if active { theme::BG_HEADER } else { theme::BG_DARK };
    let color  = if active { theme::TEXT_ACCENT } else { theme::TEXT_MUTED };
    let border = if active { theme::BORDER_ACCENT } else { theme::BORDER };

    let btn = Button::new(RichText::new(label).color(color).size(12.0))
        .fill(fill)
        .stroke(Stroke::new(1.0, border));

    if ui.add(btn).clicked() { on_click(); }
}

fn path_row(ui: &mut egui::Ui, label: &str, hint: &str, value: &mut String, id: &str) {
    path_row_required(ui, label, hint, value, id, false);
}

fn path_row_required(ui: &mut egui::Ui, label: &str, hint: &str, value: &mut String, id: &str, required: bool) {
    required_label(ui, label, required && value.is_empty());
    ui.add_space(2.0);
    ui.label(RichText::new(hint).color(theme::TEXT_MUTED).size(10.5).italics());
    ui.add_space(4.0);

    let spacing = ui.spacing().item_spacing.x;

    ui.horizontal(|ui| {
        if ui.button("…").on_hover_text(tr("Выбрать папку")).clicked() {
            if let Some(p) = pick_folder(label) {
                *value = p;
            }
        }
        ui.add_space(spacing);
        ui.add(
            egui::TextEdit::singleline(value)
                .id(egui::Id::new(id))
                .desired_width(fit_width(ui))
                .text_color(theme::TEXT_PRIMARY)
                .hint_text(tr("Не задан")),
        );
    });
}

fn checkbox_row(ui: &mut egui::Ui, value: &mut bool, label: &str) {
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.checkbox(value, RichText::new(label).color(theme::TEXT_PRIMARY).size(12.0));
    });
}

fn section_header(ui: &mut egui::Ui, title: &str) {
    Frame::NONE
        .fill(theme::BG_HEADER)
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(title).color(theme::TEXT_MUTED).size(10.0).strong());
        });
}

/// Заголовок поля с опциональным красным маркером обязательности.
fn required_label(ui: &mut egui::Ui, label: &str, is_empty: bool) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(theme::TEXT_PRIMARY).size(12.0).strong());
        if is_empty {
            ui.label(RichText::new("*").color(theme::ERROR_RED).size(12.0).strong())
                .on_hover_text(tr("Обязательное поле"));
        }
    });
}

/// Однострочное поле ввода пути, растянутое на всю доступную ширину (без кнопки).
fn path_edit(ui: &mut egui::Ui, value: &mut String, hint: &str, id: &str) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .id(egui::Id::new(id))
            .desired_width(fit_width_minus(ui, 36.0))
            .text_color(theme::TEXT_PRIMARY)
            .hint_text(hint),
    )
}
