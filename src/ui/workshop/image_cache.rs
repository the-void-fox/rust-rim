//! Асинхронная загрузка превью из мастерской.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

/// Максимум текстур, загружаемых в GPU за один кадр: защита от фриза,
/// когда приходит сразу страница из 30 картинок.
const MAX_TEXTURE_UPLOADS_PER_FRAME: usize = 3;

pub struct ImageCache {
    textures: HashMap<String, egui::TextureHandle>,
    pending: HashSet<String>,
    // Декод выполняется в фоновом потоке; по каналу приходит готовый ColorImage.
    tx: mpsc::Sender<(String, egui::ColorImage)>,
    rx: mpsc::Receiver<(String, egui::ColorImage)>,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageCache {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { textures: HashMap::new(), pending: HashSet::new(), tx, rx }
    }

    pub fn request(&mut self, url: &str) {
        if url.is_empty() || self.textures.contains_key(url) || self.pending.contains(url) {
            return;
        }
        self.pending.insert(url.to_string());
        let url_owned = url.to_string();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            // Скачивание И декод — в этом потоке; UI-поток только грузит текстуру.
            let result: anyhow::Result<egui::ColorImage> = (|| {
                let buf = ureq::get(&url_owned)
                    .header("User-Agent", "Mozilla/5.0")
                    .call()?
                    .body_mut()
                    .read_to_vec()?;
                let img = image::load_from_memory(&buf)?;
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                Ok(egui::ColorImage::from_rgba_unmultiplied(size, &rgba.into_raw()))
            })();
            if let Ok(ci) = result {
                let _ = tx.send((url_owned, ci));
            }
        });
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        for _ in 0..MAX_TEXTURE_UPLOADS_PER_FRAME {
            let Ok((url, ci)) = self.rx.try_recv() else { break };
            self.pending.remove(&url);
            let tex = ctx.load_texture(&url, ci, egui::TextureOptions::LINEAR);
            self.textures.insert(url, tex);
        }
    }

    pub fn get(&self, url: &str) -> Option<&egui::TextureHandle> {
        self.textures.get(url)
    }

    pub fn is_busy(&self) -> bool {
        !self.pending.is_empty()
    }
}
