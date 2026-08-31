//! Асинхронная загрузка превью-картинки выбранного мода.
//!
//! Декодирование PNG/JPEG идёт в отдельном потоке: баннеры модов бывают
//! в несколько мегапикселей, и синхронное чтение роняло бы кадр при каждом
//! перемещении выделения.

use std::path::{Path, PathBuf};

use crate::job::Job;

#[derive(Default)]
pub struct Preview {
    path: Option<PathBuf>,
    job: Job<egui::ColorImage>,
    texture: Option<egui::TextureHandle>,
}

impl Preview {
    pub fn new() -> Self {
        Self::default()
    }

    /// Сбрасывает состояние (например, после пересканирования модов).
    pub fn reset(&mut self) {
        self.path = None;
        self.job = Job::Idle;
        self.texture = None;
    }

    /// Возвращает текстуру для `path`, запуская загрузку при смене пути.
    /// Вызывается каждый кадр; сама решает, что делать.
    pub fn texture_for(
        &mut self,
        ctx: &egui::Context,
        path: Option<&Path>,
    ) -> Option<&egui::TextureHandle> {
        if self.path.as_deref() != path {
            self.path = path.map(Path::to_path_buf);
            self.texture = None;
            self.job = match path {
                Some(p) => {
                    let p = p.to_path_buf();
                    Job::spawn(move || decode(&p))
                }
                None => Job::Idle,
            };
        }

        if self.job.poll() {
            if let Some(image) = self.job.take_result() {
                self.texture =
                    Some(ctx.load_texture("mod_preview", image, egui::TextureOptions::LINEAR));
            }
        } else if self.job.is_running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }

        self.texture.as_ref()
    }
}

fn decode(path: &Path) -> Result<egui::ColorImage, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        rgba.as_raw(),
    ))
}
