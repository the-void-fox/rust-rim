use anyhow::{anyhow, bail, Result};
use serde::Deserialize as _;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write as _;

pub const APP_ID: u64 = 294100;

const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

/// Страница браузера Workshop весит ~1 МБ; лимит с запасом.
const BODY_LIMIT: u64 = 32 * 1024 * 1024;

// ─── Типы ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WorkshopItem {
    pub id: u64,
    pub title: String,
    pub author: String,
    pub preview_url: String,
}

#[derive(Debug, Clone)]
pub struct CollectionItem {
    pub id: u64,
    pub title: String,
    pub author: String,
    pub preview_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortOrder {
    Trending,
    Latest,
    MostSubscribed,
    RecentlyUpdated,
}

impl SortOrder {
    pub fn as_param(self) -> &'static str {
        match self {
            Self::Trending        => "trend",
            Self::Latest          => "mostrecent",
            Self::MostSubscribed  => "totaluniquesubscriptions",
            Self::RecentlyUpdated => "lastupdated",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Trending        => "Популярные",
            Self::Latest          => "Новые",
            Self::MostSubscribed  => "По подпискам",
            Self::RecentlyUpdated => "Обновлённые",
        }
    }

    pub const ALL: [SortOrder; 4] = [
        Self::Trending,
        Self::Latest,
        Self::MostSubscribed,
        Self::RecentlyUpdated,
    ];
}

// ─── Запрос к Steam Workshop ─────────────────────────────────────────────────
//
// Steam переписал страницы Workshop на React: серверная разметка с классами
// вида .workshopItem исчезла, CSS-скрейпинг больше невозможен. Однако данные
// по-прежнему рендерятся на сервере и встраиваются в страницу как JSON:
//
//   window.SSR.renderContext = JSON.parse("{...\"queryData\":\"{...}\"...}");
//
// Внутри queryData лежит кэш react-query, где запись с ключом
// ["workshop_browse", {...}] содержит результаты поиска (publishedfileid,
// title, preview_url, ...) и creator_player_link_details с именами авторов.

/// Возвращает список модов и флаг наличия следующей страницы.
pub fn fetch_workshop_page(
    query: &str,
    page: u32,
    sort: SortOrder,
) -> Result<(Vec<WorkshopItem>, bool)> {
    let url = format!(
        "https://steamcommunity.com/workshop/browse/?appid={}&searchtext={}&section=readytouseitems&browsesort={}&p={}",
        APP_ID, url_encode(query), sort.as_param(), page
    );

    let html = fetch_html(&url)?;
    let data = extract_browse_data(&html)?;
    let authors = author_names(&data);

    let items = data["results"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    let id: u64 = r["publishedfileid"].as_str()?.parse().ok()?;
                    Some(WorkshopItem {
                        id,
                        title: r["title"].as_str().unwrap_or_default().to_string(),
                        author: authors
                            .get(r["creator"].as_str().unwrap_or_default())
                            .cloned()
                            .unwrap_or_default(),
                        preview_url: thumb_url(r["preview_url"].as_str().unwrap_or_default()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok((items, has_next_page(&data)))
}

// ─── Сборки (Collections) ────────────────────────────────────────────────────

pub fn fetch_collections_page(
    query: &str,
    page: u32,
    sort: SortOrder,
) -> Result<(Vec<CollectionItem>, bool)> {
    let search_part = if query.is_empty() {
        String::new()
    } else {
        format!("&searchtext={}", url_encode(query))
    };
    let url = format!(
        "https://steamcommunity.com/workshop/browse/?appid={}{}&section=collections&browsesort={}&p={}",
        APP_ID, search_part, sort.as_param(), page
    );

    let html = fetch_html(&url)?;
    let data = extract_browse_data(&html)?;
    let authors = author_names(&data);

    let items = data["results"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    let id: u64 = r["publishedfileid"].as_str()?.parse().ok()?;
                    Some(CollectionItem {
                        id,
                        title: r["title"].as_str().unwrap_or_default().to_string(),
                        author: authors
                            .get(r["creator"].as_str().unwrap_or_default())
                            .cloned()
                            .unwrap_or_default(),
                        preview_url: thumb_url(r["preview_url"].as_str().unwrap_or_default()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok((items, has_next_page(&data)))
}

/// Возвращает (название сборки, список модов).
/// Использует старый POST-API Steam (ключ не нужен) — он всё ещё работает.
pub fn fetch_collection_mods(collection_id: u64) -> Result<(String, Vec<WorkshopItem>)> {
    // Шаг 1: получить дочерние ID
    let body = format!("collectioncount=1&publishedfileids[0]={}", collection_id);
    let json = post_form("https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/", &body)?;

    let detail = &json["response"]["collectiondetails"][0];

    let children = detail["children"].as_array()
        .ok_or_else(|| anyhow!("Сборка пустая или недоступна"))?;

    let ids: Vec<u64> = children.iter()
        .filter_map(|c| c["publishedfileid"].as_str())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect();

    if ids.is_empty() {
        return Ok(("Collection".to_string(), Vec::new()));
    }

    // Шаг 2: получить детали каждого мода.
    // GetCollectionDetails не возвращает название сборки, поэтому запрашиваем
    // и саму сборку (первым элементом) — её title и есть название.
    let mut body = format!("itemcount={}&publishedfileids[0]={}", ids.len() + 1, collection_id);
    for (i, id) in ids.iter().enumerate() {
        let _ = write!(body, "&publishedfileids[{}]={}", i + 1, id);
    }

    let json = post_form("https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/", &body)?;
    let details = json["response"]["publishedfiledetails"]
        .as_array()
        .ok_or_else(|| anyhow!("Нет данных о модах сборки"))?;

    let mut coll_title = "Collection".to_string();
    let items = details.iter()
        .filter_map(|d| {
            let id = d["publishedfileid"].as_str()?.parse::<u64>().ok()?;
            let title = d["title"].as_str().unwrap_or("").to_string();
            if id == collection_id {
                if !title.is_empty() {
                    coll_title = title;
                }
                return None;
            }
            let preview_url = thumb_url(d["preview_url"].as_str().unwrap_or(""));
            Some(WorkshopItem { id, title, author: String::new(), preview_url })
        })
        .collect();

    Ok((coll_title, items))
}

// ─── HTTP ────────────────────────────────────────────────────────────────────

fn fetch_html(url: &str) -> Result<String> {
    let mut resp = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept-Language", "en-US,en;q=0.9")
        .call()
        .map_err(|e| anyhow!("HTTP ошибка: {e}"))?;
    Ok(resp.body_mut().with_config().limit(BODY_LIMIT).read_to_string()?)
}

fn post_form(url: &str, body: &str) -> Result<Value> {
    let mut resp = ureq::post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("User-Agent", USER_AGENT)
        .send(body)
        .map_err(|e| anyhow!("HTTP ошибка: {e}"))?;
    let text = resp.body_mut().with_config().limit(BODY_LIMIT).read_to_string()?;
    Ok(serde_json::from_str(&text)?)
}

// ─── Разбор SSR-данных страницы ──────────────────────────────────────────────

/// Извлекает `state.data` запроса `workshop_browse` из встроенного SSR-кэша.
fn extract_browse_data(html: &str) -> Result<Value> {
    const MARKER: &str = "window.SSR.renderContext=JSON.parse(";

    let start = html.find(MARKER)
        .ok_or_else(|| anyhow!("Steam изменил формат страницы: renderContext не найден"))?
        + MARKER.len();

    // Аргумент JSON.parse(...) — строковый литерал, совместимый с JSON.
    // Deserializer читает ровно один литерал и игнорирует остальной скрипт.
    let mut de = serde_json::Deserializer::from_str(&html[start..]);
    let inner = String::deserialize(&mut de)
        .map_err(|e| anyhow!("Не удалось прочитать renderContext: {e}"))?;

    let ctx: Value = serde_json::from_str(&inner)?;
    let query_data: Value = serde_json::from_str(
        ctx["queryData"].as_str().ok_or_else(|| anyhow!("queryData отсутствует"))?,
    )?;

    let queries = query_data["queries"].as_array()
        .ok_or_else(|| anyhow!("queries отсутствуют в queryData"))?;

    for q in queries {
        if q["queryKey"][0].as_str() == Some("workshop_browse") {
            let data = &q["state"]["data"];
            if data.is_object() {
                return Ok(data.clone());
            }
        }
    }
    bail!("Данные workshop_browse не найдены на странице")
}

/// Превращает полноразмерный URL превью в URL миниатюры через CDN-ресайз Steam.
/// Полные превью — мегабайтные PNG; миниатюра ~8 КБ JPEG (в 60 раз меньше).
fn thumb_url(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let sep = if raw.contains('?') { '&' } else { '?' };
    format!("{raw}{sep}imw=256&imh=256&ima=fit&impolicy=Letterbox")
}

/// Карта steamid → имя автора из creator_player_link_details.
fn author_names(data: &Value) -> HashMap<String, String> {
    data["creator_player_link_details"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let pd = &p["public_data"];
                    Some((
                        pd["steamid"].as_str()?.to_string(),
                        pd["persona_name"].as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn has_next_page(data: &Value) -> bool {
    let cur   = data["current_page"].as_u64().unwrap_or(1);
    let total = data["total_pages"].as_u64().unwrap_or(1);
    cur < total
}

// ─── Вспомогательное ──────────────────────────────────────────────────────────

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b' '                                          => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~'                  => out.push(b as char),
            _                                             => { let _ = out.write_fmt(format_args!("%{b:02X}")); }
        }
    }
    out
}
