use std::path::Path;
use anyhow::{Context, Result};
use quick_xml::Reader;
use quick_xml::events::{BytesRef, Event};

// ─── About.xml ───────────────────────────────────────────────────────────────

pub struct AboutData {
    pub name: String,
    pub package_id: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub supported_versions: Vec<String>,
    pub dependencies: Vec<String>,
    /// Откуда качать зависимость, если её нет: (packageId, Workshop ID).
    ///
    /// RimWorld требует от модов указывать `<steamWorkshopUrl>` рядом с каждой
    /// зависимостью и ругается в лог, если его нет. На живой установке ссылку
    /// дают 602 мода из 902 — этого хватает, чтобы предложить доустановку.
    pub dependency_sources: Vec<(String, u64)>,
    pub load_after: Vec<String>,
    pub load_before: Vec<String>,
    pub incompatible_with: Vec<String>,
}

/// Достаёт номер предмета мастерской из ссылки.
///
/// Форм в дикой природе несколько, и не все ведут в мастерскую. Замеры на
/// 902 модах: `steam://url/CommunityFilePage/<id>` — 483 раза,
/// `steamcommunity.com/sharedfiles/filedetails/?id=<id>` — 178,
/// `.../workshop/filedetails/?id=<id>` — 132, плюс 4 ссылки с опечаткой
/// `https:/` в одну косую и одна с заглушкой `xxxxxxxxxx` вместо номера.
///
/// Отдельно важны 7 ссылок на `store.steampowered.com/app/<id>`: это страницы
/// DLC, и число там — идентификатор приложения, а не предмета мастерской.
/// Скачать его как мод нельзя, поэтому такие ссылки отбрасываются.
pub fn workshop_id_from_url(url: &str) -> Option<u64> {
    let url = url.trim();
    if url.contains("store.steampowered.com") {
        return None;
    }
    let tail = if let Some(rest) = url.split("CommunityFilePage/").nth(1) {
        rest
    } else if url.contains("steamcommunity.com") {
        url.split("id=").nth(1)?
    } else {
        return None;
    };
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|id| *id > 0)
}

// quick-xml ≥0.38 отдаёт `&amp;` и прочие ссылки отдельным событием GeneralRef,
// а текст вокруг них — отдельными Text. Поэтому текст элемента накапливается
// в буфере и обрабатывается на закрывающем теге, иначе "Cats &amp; Dogs"
// распался бы на три куска.

/// Разворачивает ссылку на сущность (`&amp;`, `&#1080;`, ...) в текст.
fn resolve_ref(e: &BytesRef) -> String {
    if let Ok(Some(ch)) = e.resolve_char_ref() {
        return ch.to_string();
    }
    match e.decode().as_deref() {
        Ok("amp")  => "&",
        Ok("lt")   => "<",
        Ok("gt")   => ">",
        Ok("quot") => "\"",
        Ok("apos") => "'",
        _          => "", // неизвестная сущность — пропускаем
    }
    .to_string()
}

pub fn parse_about_xml(xml_path: &Path) -> Result<AboutData> {
    let content = std::fs::read_to_string(xml_path)
        .with_context(|| format!("cannot read {:?}", xml_path))?;

    let mut reader = Reader::from_str(&content);

    let mut stack: Vec<String> = Vec::new();
    let mut buf = String::new();
    // packageId и ссылка лежат в соседних тегах одного <li>, поэтому
    // накапливаются до его закрытия.
    let mut dep_id: Option<String> = None;
    let mut dep_workshop: Option<u64> = None;
    let mut data = AboutData {
        name: String::new(),
        package_id: String::new(),
        version: String::new(),
        author: String::new(),
        description: String::new(),
        supported_versions: Vec::new(),
        dependencies: Vec::new(),
        dependency_sources: Vec::new(),
        load_after: Vec::new(),
        load_before: Vec::new(),
        incompatible_with: Vec::new(),
    };

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                stack.push(tag);
                buf.clear();
            }
            Ok(Event::Text(e)) => {
                if let Ok(t) = e.xml10_content() {
                    buf.push_str(&t);
                }
            }
            Ok(Event::GeneralRef(e)) => buf.push_str(&resolve_ref(&e)),
            Ok(Event::CData(e)) => {
                if let Ok(t) = e.decode() {
                    buf.push_str(&t);
                }
            }
            Ok(Event::End(_)) => {
                let text = buf.trim().to_string();
                buf.clear();

                if !text.is_empty() {
                    let cur  = stack.last().map(String::as_str).unwrap_or("");
                    let par  = stack.iter().rev().nth(1).map(String::as_str).unwrap_or("");
                    let gpar = stack.iter().rev().nth(2).map(String::as_str).unwrap_or("");

                    match (cur, par, gpar) {
                        ("name",        "ModMetaData", _)      => data.name        = text,
                        ("packageId",   "ModMetaData", _)      => data.package_id  = text,
                        ("version",     "ModMetaData", _)      => data.version     = text,
                        ("modVersion",  "ModMetaData", _)      => { if data.version.is_empty() { data.version = text; } }
                        ("author",      "ModMetaData", _)      => data.author      = text,
                        ("description", "ModMetaData", _)      => data.description = text,
                        ("li", "supportedVersions",    _)      => data.supported_versions.push(text),
                        // loadAfter и forceLoadAfter имеют одинаковую семантику
                        ("li", "loadAfter",            _)      => data.load_after.push(text),
                        ("li", "forceLoadAfter",       _)      => data.load_after.push(text),
                        // loadBefore и forceLoadBefore имеют одинаковую семантику
                        ("li", "loadBefore",           _)      => data.load_before.push(text),
                        ("li", "forceLoadBefore",      _)      => data.load_before.push(text),
                        ("li", "incompatibleWith",     _)      => data.incompatible_with.push(text),
                        ("packageId", "li", "modDependencies") => {
                            dep_id = Some(text.clone());
                            data.dependencies.push(text);
                        }
                        ("steamWorkshopUrl", "li", "modDependencies") => {
                            dep_workshop = workshop_id_from_url(&text);
                        }
                        // Множественные авторы: <authors><li>Name</li></authors>
                        ("li", "authors", "ModMetaData")       => {
                            if data.author.is_empty() {
                                data.author = text;
                            } else {
                                data.author.push_str(", ");
                                data.author.push_str(&text);
                            }
                        }
                        _ => {}
                    }
                }

                // Закрылся <li> зависимости — складываем накопленное.
                // Проверяется здесь, а не в match выше: у самого <li> текста
                // нет, и до match дело не доходит.
                let closing_dep_li = stack.last().is_some_and(|t| t == "li")
                    && stack.iter().rev().nth(1).is_some_and(|t| t == "modDependencies");
                if closing_dep_li {
                    if let (Some(id), Some(workshop)) = (dep_id.take(), dep_workshop.take()) {
                        data.dependency_sources.push((id, workshop));
                    }
                    dep_id = None;
                    dep_workshop = None;
                }

                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(anyhow::anyhow!("XML error in {:?}: {}", xml_path, e));
            }
            _ => {}
        }
    }

    Ok(data)
}

// ─── ModsConfig.xml ──────────────────────────────────────────────────────────

/// Читает список активных модов (package IDs в порядке загрузки) из ModsConfig.xml.
pub fn parse_mods_config(xml_path: &Path) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(xml_path)
        .with_context(|| format!("cannot read {:?}", xml_path))?;

    let mut reader = Reader::from_str(&content);

    let mut stack: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut active_mods: Vec<String> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                stack.push(tag);
                buf.clear();
            }
            Ok(Event::Text(e)) => {
                if let Ok(t) = e.xml10_content() {
                    buf.push_str(&t);
                }
            }
            Ok(Event::GeneralRef(e)) => buf.push_str(&resolve_ref(&e)),
            Ok(Event::End(_)) => {
                let text = buf.trim().to_string();
                buf.clear();
                if !text.is_empty() {
                    let cur = stack.last().map(String::as_str).unwrap_or("");
                    let par = stack.iter().rev().nth(1).map(String::as_str).unwrap_or("");
                    if cur == "li" && par == "activeMods" {
                        active_mods.push(text);
                    }
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("XML error in {:?}: {}", xml_path, e)),
            _ => {}
        }
    }

    Ok(active_mods)
}

/// Записывает список модов в отдельный файл (.xml) в формате ModsConfigData,
/// совместимом с RimSort и RimWorld.
/// Не сохраняет knownExpansions — только activeMods.
pub fn write_mod_list(path: &Path, active_package_ids: &[String]) -> Result<()> {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str("<ModsConfigData>\n");
    out.push_str("\t<version>1.0</version>\n");
    out.push_str("\t<activeMods>\n");
    for id in active_package_ids {
        out.push_str(&format!("\t\t<li>{}</li>\n", id));
    }
    out.push_str("\t</activeMods>\n");
    out.push_str("</ModsConfigData>\n");
    std::fs::write(path, out)
        .with_context(|| format!("cannot write mod list {:?}", path))
}

/// Записывает активные моды в ModsConfig.xml.
/// `version` и `knownExpansions` читаются из существующего файла и сохраняются без изменений.
pub fn write_mods_config(xml_path: &Path, active_package_ids: &[String]) -> Result<()> {
    let (version, known_expansions) = read_mods_config_extras(xml_path);

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str("<ModsConfigData>\n");
    out.push_str(&format!("\t<version>{}</version>\n", version));
    out.push_str("\t<activeMods>\n");
    for id in active_package_ids {
        out.push_str(&format!("\t\t<li>{}</li>\n", id));
    }
    out.push_str("\t</activeMods>\n");
    if !known_expansions.is_empty() {
        out.push_str("\t<knownExpansions>\n");
        for id in known_expansions {
            out.push_str(&format!("\t\t<li>{}</li>\n", id));
        }
        out.push_str("\t</knownExpansions>\n");
    }
    out.push_str("</ModsConfigData>\n");

    std::fs::write(xml_path, out)
        .with_context(|| format!("cannot write {:?}", xml_path))
}

/// Считывает `version` и `knownExpansions` из существующего ModsConfig.xml,
/// возвращает дефолты если файл не читается.
fn read_mods_config_extras(xml_path: &Path) -> (String, Vec<String>) {
    let content = match std::fs::read_to_string(xml_path) {
        Ok(c) => c,
        Err(_) => return ("1.0.0".to_string(), Vec::new()),
    };

    let mut reader = Reader::from_str(&content);

    let mut stack: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut version = String::new();
    let mut known: Vec<String> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let tag = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                stack.push(tag);
                buf.clear();
            }
            Ok(Event::Text(e)) => {
                if let Ok(t) = e.xml10_content() {
                    buf.push_str(&t);
                }
            }
            Ok(Event::GeneralRef(e)) => buf.push_str(&resolve_ref(&e)),
            Ok(Event::End(_)) => {
                let text = buf.trim().to_string();
                buf.clear();
                if !text.is_empty() {
                    let cur = stack.last().map(String::as_str).unwrap_or("");
                    let par = stack.iter().rev().nth(1).map(String::as_str).unwrap_or("");
                    match (cur, par) {
                        ("version", "ModsConfigData") => version = text,
                        ("li", "knownExpansions") => known.push(text),
                        _ => {}
                    }
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
    }

    (
        if version.is_empty() { "1.0.0".to_string() } else { version },
        known,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_workshop_url_form_seen_in_the_wild() {
        // Доли на живой установке из 902 модов: 483 / 178 / 132.
        assert_eq!(
            workshop_id_from_url("steam://url/CommunityFilePage/2009463077"),
            Some(2009463077),
        );
        assert_eq!(
            workshop_id_from_url("https://steamcommunity.com/sharedfiles/filedetails/?id=2009463077"),
            Some(2009463077),
        );
        assert_eq!(
            workshop_id_from_url("https://steamcommunity.com/workshop/filedetails/?id=2009463077"),
            Some(2009463077),
        );
        // Опечатка в одну косую — встречается у четырёх модов.
        assert_eq!(
            workshop_id_from_url("https:/steamcommunity.com/sharedfiles/filedetails/?id=2009463077"),
            Some(2009463077),
        );
    }

    #[test]
    fn store_pages_are_not_workshop_items() {
        // Семь модов дают в steamWorkshopUrl ссылку на страницу DLC. Число там
        // — идентификатор приложения; скачать его как мод нельзя, и попытка
        // отправила бы SteamCMD за несуществующим предметом.
        assert_eq!(
            workshop_id_from_url("https://store.steampowered.com/app/1149640"),
            None,
        );
        assert_eq!(
            workshop_id_from_url("https://store.steampowered.com/app/1826140/RimWorld__Biotech/"),
            None,
        );
    }

    #[test]
    fn junk_urls_yield_nothing() {
        assert_eq!(workshop_id_from_url("steam://url/CommunityFilePage/xxxxxxxxxx"), None);
        assert_eq!(workshop_id_from_url(""), None);
        assert_eq!(workshop_id_from_url("https://github.com/author/mod"), None);
        assert_eq!(workshop_id_from_url("https://steamcommunity.com/sharedfiles/"), None);
        // Ноль — не предмет мастерской.
        assert_eq!(workshop_id_from_url("steam://url/CommunityFilePage/0"), None);
    }

    #[test]
    fn dependency_and_its_source_are_read_together() {
        let dir = std::env::temp_dir()
            .join(format!("rustrim_parser_{}_dep", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("About.xml");
        std::fs::write(
            &path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<ModMetaData>
  <name>Test</name>
  <packageId>author.test</packageId>
  <modDependencies>
    <li>
      <packageId>brrainz.harmony</packageId>
      <displayName>Harmony</displayName>
      <steamWorkshopUrl>steam://url/CommunityFilePage/2009463077</steamWorkshopUrl>
    </li>
    <li>
      <packageId>some.mod.without.link</packageId>
      <displayName>No Link</displayName>
    </li>
  </modDependencies>
</ModMetaData>
"#,
        )
        .unwrap();

        let data = parse_about_xml(&path).unwrap();
        assert_eq!(
            data.dependencies,
            vec!["brrainz.harmony".to_string(), "some.mod.without.link".to_string()],
        );
        // Ссылка есть только у первой — вторая в источники не попадает.
        assert_eq!(
            data.dependency_sources,
            vec![("brrainz.harmony".to_string(), 2009463077u64)],
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
