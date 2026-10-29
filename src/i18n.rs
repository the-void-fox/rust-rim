//! Перевод интерфейса.
//!
//! Ключ перевода — сам русский текст из исходника: `tr("Сохранить")`. Так код
//! читается как раньше, а строка, для которой перевода нет, просто остаётся
//! русской. Переводы лежат в `locales/<язык>.json` и вшиты в бинарник.
//!
//! Строка с подстановками переводится целиком, вместе с `{именами}`:
//! порядок слов в языках разный, и склеивать фразу из кусков нельзя.
//!
//! ```ignore
//! ui.label(tr("Настройки"));
//! ui.label(trf!("Активных: {active}", active = n));
//! const LABEL: &str = k("Моды"); // ключ в константе; перевод — tr(LABEL)
//! ```
//!
//! Полноту переводов проверяет `tests/i18n_coverage.rs`: он находит в коде
//! каждый `tr`, `trf!` и `k` и требует перевод во всех языках.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Lang {
    Ru,
    En,
    Uk,
    De,
    Es,
    Pt,
    Zh,
}

impl Lang {
    pub const ALL: [Lang; 7] = [Lang::En, Lang::Ru, Lang::Uk, Lang::De, Lang::Es, Lang::Pt, Lang::Zh];

    /// Название языка на нём самом — так его узнают в списке.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Ru => "Русский",
            Lang::En => "English",
            Lang::Uk => "Українська",
            Lang::De => "Deutsch",
            Lang::Es => "Español",
            Lang::Pt => "Português (Brasil)",
            Lang::Zh => "简体中文",
        }
    }

    /// Имя файла в `locales/`.
    pub fn file(self) -> &'static str {
        match self {
            Lang::Ru => "",
            Lang::En => "en",
            Lang::Uk => "uk",
            Lang::De => "de",
            Lang::Es => "es",
            Lang::Pt => "pt-BR",
            Lang::Zh => "zh-CN",
        }
    }

    /// Язык системы; незнакомый — английский.
    pub fn detect() -> Lang {
        let locale = sys_locale::get_locale().unwrap_or_default().to_lowercase();
        let prefix = locale.split(['-', '_']).next().unwrap_or("");
        match prefix {
            "ru" | "be" | "kk" => Lang::Ru,
            "uk" => Lang::Uk,
            "de" => Lang::De,
            "es" => Lang::Es,
            "pt" => Lang::Pt,
            "zh" => Lang::Zh,
            _ => Lang::En,
        }
    }

    fn index(self) -> u8 {
        self as u8
    }

    fn from_index(i: u8) -> Lang {
        Lang::ALL.into_iter().find(|l| l.index() == i).unwrap_or(Lang::Ru)
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(Lang::Ru as u8);

pub fn set_lang(lang: Lang) {
    CURRENT.store(lang.index(), Ordering::Relaxed);
}

pub fn lang() -> Lang {
    Lang::from_index(CURRENT.load(Ordering::Relaxed))
}

/// Исходный текст файла переводов.
pub fn source(lang: Lang) -> &'static str {
    match lang {
        Lang::Ru => "{}",
        Lang::En => include_str!("../locales/en.json"),
        Lang::Uk => include_str!("../locales/uk.json"),
        Lang::De => include_str!("../locales/de.json"),
        Lang::Es => include_str!("../locales/es.json"),
        Lang::Pt => include_str!("../locales/pt-BR.json"),
        Lang::Zh => include_str!("../locales/zh-CN.json"),
    }
}

type Table = HashMap<&'static str, &'static str>;

static TABLES: LazyLock<Vec<Table>> = LazyLock::new(|| {
    let mut tables: Vec<Table> = (0..Lang::ALL.len()).map(|_| HashMap::new()).collect();
    for lang in Lang::ALL {
        let parsed: HashMap<String, String> = match serde_json::from_str(source(lang)) {
            Ok(map) => map,
            Err(e) => {
                tracing::error!("locales/{}.json is broken: {}", lang.file(), e);
                continue;
            }
        };
        // Таблицы живут до конца программы — утечка здесь и есть «static».
        tables[lang.index() as usize] = parsed
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (&*Box::leak(k.into_boxed_str()), &*Box::leak(v.into_boxed_str())))
            .collect();
    }
    tables
});

/// Перевод строки на текущий язык; нет перевода — исходный текст.
pub fn tr(key: &str) -> &str {
    lookup(lang(), key)
}

fn lookup(lang: Lang, key: &str) -> &str {
    if lang == Lang::Ru {
        return key;
    }
    TABLES[lang.index() as usize].get(key).copied().unwrap_or(key)
}

/// Помечает строку как ключ перевода там, где вызвать `tr` нельзя: в
/// константах. Переводится при показе, через `tr`.
pub const fn k(key: &'static str) -> &'static str {
    key
}

/// Перевод строки с подстановками `{имя}`.
///
/// Аргументы только именованные: в переводе слова могут стоять в другом
/// порядке, и позиционный `{}` указал бы не туда. `trf!("… {n} …", n)` —
/// короткая запись для `n = n`.
#[macro_export]
macro_rules! trf {
    (@v $name:ident) => { $name };
    (@v $name:ident, $val:expr) => { $val };
    ($key:literal $(, $name:ident $(= $val:expr)?)* $(,)?) => {{
        #[allow(unused_mut)]
        let mut s = $crate::i18n::tr($key).to_string();
        $( s = s.replace(
            concat!("{", stringify!($name), "}"),
            &$crate::trf!(@v $name $(, $val)?).to_string(),
        ); )*
        s
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    // Глобальный язык здесь не трогаем: тесты идут параллельно, и другие
    // ждут русских строк.

    #[test]
    fn переводит_и_оставляет_незнакомое_как_есть() {
        assert_eq!(lookup(Lang::En, "Сохранить список модов"), "Save mod list");
        assert_eq!(lookup(Lang::Zh, "Сохранить список модов"), "保存模组列表");
        assert_eq!(lookup(Lang::En, "строки нет в переводах"), "строки нет в переводах");
        assert_eq!(lookup(Lang::Ru, "Сохранить список модов"), "Сохранить список модов");
    }

    #[test]
    fn все_файлы_переводов_разбираются() {
        for lang in Lang::ALL {
            let parsed: Result<HashMap<String, String>, _> = serde_json::from_str(source(lang));
            assert!(parsed.is_ok(), "locales/{}.json", lang.file());
        }
    }
}
