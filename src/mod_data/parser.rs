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
    pub load_after: Vec<String>,
    pub load_before: Vec<String>,
    pub incompatible_with: Vec<String>,
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
    let mut data = AboutData {
        name: String::new(),
        package_id: String::new(),
        version: String::new(),
        author: String::new(),
        description: String::new(),
        supported_versions: Vec::new(),
        dependencies: Vec::new(),
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
                        ("packageId", "li", "modDependencies") => data.dependencies.push(text),
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
