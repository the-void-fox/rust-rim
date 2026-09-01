//! Набор шрифтов приложения.
//!
//! Живёт в библиотеке, а не в `main`, чтобы тест мог спросить у тех же самых
//! шрифтов, рисуется ли каждый символ, которым мы пользуемся в интерфейсе.
//!
//! Повод конкретный: в egui 0.35 не рисуется ни одно эмодзи. NotoEmoji в
//! наборе по умолчанию есть, но ни один его глиф не резолвится — проверено
//! напрямую через `has_glyph`. Работает только BMP-часть `emoji-icon-font`
//! (⚙ ⏱ ⟳ ⚒ ⚑ ☰ …) плюс то, что нашлось в NotoSansSC. Поэтому в интерфейсе
//! эмодзи нет вообще: вместо них геометрические символы, а `has_glyph` для
//! каждого проверяет `tests/glyph_coverage.rs`.

use egui::{FontData, FontDefinitions, FontFamily};

/// Основной шрифт: кириллица, латиница и CJK в одном файле.
const NOTO_SANS_SC: &[u8] = include_bytes!("../assets/NotoSansSC.ttf");

/// Шрифты приложения: встроенные в egui плюс наш основной первым в списке.
///
/// Порядок важен: egui берёт глиф из первого шрифта семейства, где он есть,
/// поэтому иконки из `emoji-icon-font` продолжают работать — их просто нет в
/// NotoSansSC, и поиск идёт дальше по списку.
pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "NotoSansSC".to_owned(),
        FontData::from_static(NOTO_SANS_SC).into(),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "NotoSansSC".to_owned());
    }
    fonts
}
