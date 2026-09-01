//! Браузер Steam Workshop.
//!
//! Раньше это был один файл на 940 строк: две вкладки, кэш картинок, три
//! самодельных протокола «канал плюс enum состояния» и дословно скопированная
//! очередь. Теперь состояние вкладок разъехалось по своим модулям, запросы
//! ушли на общий [`crate::job::Job`], а общее — поиск, страницы, карточки и
//! очередь — существует в одном экземпляре.

mod browse;
mod collections;
mod image_cache;
mod mods;
mod queue;

use std::collections::HashSet;

use egui::{Frame, Margin, Stroke};

use crate::steam::workshop_api::{self, WorkshopItem};
use crate::ui::theme;

use browse::Browse;
use collections::Collections;
use queue::Queue;

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Mods,
    Collections,
}

pub struct WorkshopBrowser {
    tab: Tab,
    mods: Browse<WorkshopItem>,
    collections: Collections,
    /// Очередь общая: моды в неё попадают и поштучно, и целой сборкой.
    queue: Queue,
}

impl Default for WorkshopBrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkshopBrowser {
    pub fn new() -> Self {
        Self {
            tab: Tab::Mods,
            mods: Browse::new(workshop_api::fetch_workshop_page),
            collections: Collections::new(),
            queue: Queue::default(),
        }
    }

    /// Возвращает идентификаторы, которые пользователь отправил в SteamCMD.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        installed: &HashSet<u64>,
    ) -> Option<Vec<u64>> {
        self.mods.poll(ctx);
        self.collections.poll(ctx, &mut self.queue, installed);

        if self.mods.is_busy() || self.collections.is_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }

        let mut result = None;
        egui::Window::new("⚒  Steam Workshop — Браузер модов")
            .open(open)
            .collapsible(false)
            .resizable(true)
            .min_width(720.0)
            .min_height(520.0)
            .frame(
                Frame::window(&ctx.global_style())
                    .fill(theme::BG_PANEL)
                    .stroke(Stroke::new(1.0, theme::BORDER_ACCENT)),
            )
            .show(ctx, |ui| {
                result = self.content(ui, installed);
            });

        result
    }

    fn content(&mut self, ui: &mut egui::Ui, installed: &HashSet<u64>) -> Option<Vec<u64>> {
        Frame::NONE
            .fill(theme::BG_DARK)
            .inner_margin(Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if browse::tab_btn(ui, "▣  Моды", self.tab == Tab::Mods) {
                        self.tab = Tab::Mods;
                    }
                    if browse::tab_btn(ui, "▤  Сборки", self.tab == Tab::Collections) {
                        self.tab = Tab::Collections;
                    }
                });
            });
        ui.add_space(2.0);

        // Место под очередь нужно вычесть до отрисовки списка, иначе она
        // выедет за нижний край окна.
        let reserved = self.queue.height() + 4.0;
        match self.tab {
            Tab::Mods => mods::show(ui, &mut self.mods, &mut self.queue, installed, reserved),
            Tab::Collections => collections::show(ui, &mut self.collections, reserved),
        }

        queue::footer(ui, &mut self.queue, "wsbrowser_queue_tags")
    }
}
