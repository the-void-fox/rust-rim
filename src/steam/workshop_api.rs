use anyhow::{anyhow, bail, Result};
use serde::Deserialize as _;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::RwLock;

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
// Поиска по мастерской без ключа у Steam нет: QueryFiles отвечает 403. Поэтому
// путей два.
//
// Основной — страница steamcommunity.com/workshop/browse. Она собрана на React,
// но данные рендерятся на сервере и лежат в странице как JSON. Обёртку Steam
// меняет регулярно (сначала `JSON.parse("...")` в скрипте, с осени 2026 —
// `<script type="application/json" id="valve-ssr-data">`), поэтому к обёртке
// разбор не привязан: см. [`find_browse_data`].
//
// Запасной — официальный IPublishedFileService/QueryFiles, если пользователь
// указал ключ Steam Web API. Он документирован и от вёрстки не зависит.

static API_KEY: RwLock<String> = RwLock::new(String::new());

/// Ключ Steam Web API для запасного пути поиска. Пустая строка — без ключа.
pub fn set_api_key(key: &str) {
    if let Ok(mut k) = API_KEY.write() {
        *k = key.trim().to_string();
    }
}

fn api_key() -> Option<String> {
    API_KEY.read().ok().map(|k| k.clone()).filter(|k| !k.is_empty())
}

/// Что ищем: моды или сборки.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Section {
    Items,
    Collections,
}

/// Одна найденная запись — общее для модов и сборок.
struct Found {
    id: u64,
    title: String,
    author: String,
    preview_url: String,
}

/// Возвращает список модов и флаг наличия следующей страницы.
pub fn fetch_workshop_page(
    query: &str,
    page: u32,
    sort: SortOrder,
) -> Result<(Vec<WorkshopItem>, bool)> {
    let (found, next) = search(Section::Items, query, page, sort)?;
    let items = found
        .into_iter()
        .map(|f| WorkshopItem { id: f.id, title: f.title, author: f.author, preview_url: f.preview_url })
        .collect();
    Ok((items, next))
}

/// Ключ есть — сначала официальный API; не вышло — страница. И наоборот
/// без ключа. Ошибку показываем ту, что объясняет больше.
fn search(section: Section, query: &str, page: u32, sort: SortOrder) -> Result<(Vec<Found>, bool)> {
    let Some(key) = api_key() else {
        return search_page(section, query, page, sort).map_err(|e| {
            anyhow!("{e}\nЕсли так продолжится — укажите ключ Steam Web API в настройках (вкладка «Поведение»).")
        });
    };
    match search_api(&key, section, query, page, sort) {
        Ok(r) => Ok(r),
        Err(api_err) => search_page(section, query, page, sort)
            .map_err(|page_err| anyhow!("Steam Web API: {api_err}\nСтраница мастерской: {page_err}")),
    }
}

fn search_page(section: Section, query: &str, page: u32, sort: SortOrder) -> Result<(Vec<Found>, bool)> {
    let section = match section {
        Section::Items => "readytouseitems",
        Section::Collections => "collections",
    };
    let search_part = if query.is_empty() {
        String::new()
    } else {
        format!("&searchtext={}", url_encode(query))
    };
    let url = format!(
        "https://steamcommunity.com/workshop/browse/?appid={APP_ID}{search_part}&section={section}&browsesort={}&p={page}",
        sort.as_param()
    );

    let html = fetch_html(&url)?;
    let roots = json_roots(&html);
    let data = roots
        .iter()
        .find_map(find_browse_data)
        .ok_or_else(|| anyhow!("Steam изменил страницу мастерской: результаты поиска не найдены"))?;

    // Имена авторов Steam кладёт то рядом с результатами, то отдельными
    // запросами PlayerLinkDetails — собираем отовсюду.
    let mut authors = HashMap::new();
    for root in &roots {
        collect_authors(root, &mut authors);
    }

    let found = data["results"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    Some(Found {
                        id: id_of(&r["publishedfileid"])?,
                        title: r["title"].as_str().unwrap_or_default().to_string(),
                        author: authors.get(&steamid_of(&r["creator"])).cloned().unwrap_or_default(),
                        preview_url: thumb_url(r["preview_url"].as_str().unwrap_or_default()),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let next = match (data["current_page"].as_u64(), data["total_pages"].as_u64()) {
        (Some(cur), Some(total)) => cur < total,
        // Нумерацию убрали — судим по полноте страницы.
        _ => found.len() >= PAGE_SIZE as usize,
    };
    Ok((found, next))
}

/// Столько записей на странице у браузера мастерской.
const PAGE_SIZE: u32 = 30;

fn search_api(key: &str, section: Section, query: &str, page: u32, sort: SortOrder) -> Result<(Vec<Found>, bool)> {
    // EPublishedFileQueryType и EPublishedFileInfoMatchingFileType из Steamworks.
    let query_type = match sort {
        SortOrder::Trending        => 3,
        SortOrder::Latest          => 1,
        SortOrder::MostSubscribed  => 9,
        SortOrder::RecentlyUpdated => 21,
    };
    let filetype = match section {
        Section::Items => 0,
        Section::Collections => 1,
    };
    let url = format!(
        "https://api.steampowered.com/IPublishedFileService/QueryFiles/v1/?key={}&appid={APP_ID}&creator_appid={APP_ID}\
         &query_type={query_type}&filetype={filetype}&page={page}&numperpage={PAGE_SIZE}&days=7\
         &search_text={}&return_previews=false",
        url_encode(key),
        url_encode(query),
    );
    let json = get_json_keyed(&url, key)?;
    let resp = &json["response"];
    let details = resp["publishedfiledetails"].as_array().cloned().unwrap_or_default();
    if details.is_empty() && resp["total"].is_null() {
        bail!("пустой ответ (неверный ключ?)");
    }

    let creators: Vec<String> = details.iter().map(|d| steamid_of(&d["creator"])).collect();
    // Без имён авторов поиск всё равно полезен — ошибку здесь глотаем.
    let authors = player_names(key, &creators).unwrap_or_default();

    let found: Vec<Found> = details
        .iter()
        .filter_map(|d| {
            Some(Found {
                id: id_of(&d["publishedfileid"])?,
                title: d["title"].as_str().unwrap_or_default().to_string(),
                author: authors.get(&steamid_of(&d["creator"])).cloned().unwrap_or_default(),
                preview_url: thumb_url(d["preview_url"].as_str().unwrap_or_default()),
            })
        })
        .collect();

    let total = resp["total"].as_u64().unwrap_or(0);
    Ok((found, u64::from(page) * u64::from(PAGE_SIZE) < total))
}

/// Проверяет ключ одним маленьким запросом. Возвращает, сколько модов видно
/// в мастерской — чтобы было что показать, кроме «ок».
pub fn check_api_key(key: &str) -> Result<u64> {
    let key = key.trim();
    if key.is_empty() {
        bail!("ключ не указан");
    }
    let url = format!(
        "https://api.steampowered.com/IPublishedFileService/QueryFiles/v1/?key={}&appid={APP_ID}&query_type=1&numperpage=1&totalonly=true",
        url_encode(key),
    );
    get_json_keyed(&url, key)?["response"]["total"]
        .as_u64()
        .ok_or_else(|| anyhow!("Steam не принял ключ"))
}

/// steamid → имя через ISteamUser/GetPlayerSummaries (до 100 за запрос).
fn player_names(key: &str, steamids: &[String]) -> Result<HashMap<String, String>> {
    let mut ids: Vec<&str> = steamids.iter().map(String::as_str).filter(|s| !s.is_empty()).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut out = HashMap::new();
    for chunk in ids.chunks(100) {
        let url = format!(
            "https://api.steampowered.com/ISteamUser/GetPlayerSummaries/v2/?key={}&steamids={}",
            url_encode(key),
            chunk.join(",")
        );
        collect_authors(&get_json_keyed(&url, key)?, &mut out);
    }
    Ok(out)
}

// ─── Сведения о предметах мастерской ─────────────────────────────────────────

/// Что Steam знает о предмете мастерской.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedFile {
    pub id: u64,
    pub title: String,
    /// Когда мод обновляли последний раз, секунды с эпохи.
    pub time_updated: u64,
}

/// За один запрос спрашиваем не больше этого — иначе тело запроса разрастается,
/// а Steam начинает подрезать ответ.
const DETAILS_CHUNK: usize = 100;

/// Спрашивает у Steam сведения о предметах мастерской.
///
/// Метод `GetPublishedFileDetails` работает без ключа API — проверено на живых
/// запросах. Это единственный способ узнать время обновления мода, не подписываясь
/// на него в клиенте Steam.
///
/// Идентификаторы, о которых Steam ничего не знает (мод удалён или скрыт),
/// в ответе просто отсутствуют — это не ошибка.
pub fn fetch_published_files(ids: &[u64]) -> Result<Vec<PublishedFile>> {
    let mut out = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(DETAILS_CHUNK) {
        let mut body = format!("itemcount={}", chunk.len());
        for (i, id) in chunk.iter().enumerate() {
            let _ = write!(body, "&publishedfileids%5B{i}%5D={id}");
        }
        let json = post_form(
            "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/",
            &body,
        )?;
        out.extend(parse_published_files(&json));
    }
    Ok(out)
}

fn parse_published_files(json: &Value) -> Vec<PublishedFile> {
    json["response"]["publishedfiledetails"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|f| {
                    // Время нулевое у предметов, которых больше нет: судить
                    // по ним об обновлениях нельзя.
                    let time_updated = f["time_updated"].as_u64().filter(|t| *t > 0)?;
                    Some(PublishedFile {
                        id: f["publishedfileid"].as_str()?.parse().ok()?,
                        title: f["title"].as_str().unwrap_or_default().to_string(),
                        time_updated,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ─── Сборки (Collections) ────────────────────────────────────────────────────

pub fn fetch_collections_page(
    query: &str,
    page: u32,
    sort: SortOrder,
) -> Result<(Vec<CollectionItem>, bool)> {
    let (found, next) = search(Section::Collections, query, page, sort)?;
    let items = found
        .into_iter()
        .map(|f| CollectionItem { id: f.id, title: f.title, author: f.author, preview_url: f.preview_url })
        .collect();
    Ok((items, next))
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

fn get_json(url: &str) -> Result<Value> {
    let mut resp = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| anyhow!("HTTP ошибка: {e}"))?;
    let text = resp.body_mut().with_config().limit(BODY_LIMIT).read_to_string()?;
    Ok(serde_json::from_str(&text)?)
}

/// GET с ключом в адресе. Текст ошибки ureq может содержать URL, а сообщение
/// попадает в окно программы — и оттуда на скриншоты. Ключ вымарываем.
fn get_json_keyed(url: &str, key: &str) -> Result<Value> {
    get_json(url).map_err(|e| {
        let text = e.to_string();
        anyhow!("{}", text.replace(key, "***").replace(&url_encode(key), "***"))
    })
}

// ─── Разбор SSR-данных страницы ──────────────────────────────────────────────

/// Все JSON-документы, встроенные в страницу.
///
/// Берём и `<script type="application/json">`, и строковые литералы внутри
/// `JSON.parse(...)` — так Steam встраивал данные раньше. Что-то одно из этого
/// переживёт следующую переделку сайта с куда большей вероятностью, чем
/// конкретный маркер.
fn json_roots(html: &str) -> Vec<Value> {
    let mut roots = Vec::new();

    let mut rest = html;
    while let Some(open) = rest.find("<script") {
        rest = &rest[open..];
        let Some(tag_end) = rest.find('>') else { break };
        let tag = &rest[..tag_end];
        let body = &rest[tag_end + 1..];
        let Some(close) = body.find("</script>") else { break };
        if tag.contains("application/json")
            && let Ok(v) = serde_json::from_str::<Value>(body[..close].trim())
        {
            roots.push(v);
        }
        rest = &body[close..];
    }

    const PARSE: &str = "JSON.parse(";
    let mut rest = html;
    while let Some(at) = rest.find(PARSE) {
        rest = &rest[at + PARSE.len()..];
        // Аргумент — строковый литерал, совместимый с JSON; Deserializer
        // читает ровно его и не смотрит на остальной скрипт.
        let mut de = serde_json::Deserializer::from_str(rest);
        if let Ok(inner) = String::deserialize(&mut de)
            && let Ok(v) = serde_json::from_str::<Value>(&inner)
        {
            roots.push(v);
        }
    }

    roots
}

/// Ищет в дереве объект с результатами поиска: массив `results`, элементы
/// которого — предметы мастерской с `publishedfileid`.
///
/// Строки, похожие на JSON, раскрываются: Steam любит класть JSON внутрь
/// строки внутри JSON (`queryData`, `loaderData`).
fn find_browse_data(v: &Value) -> Option<Value> {
    match v {
        Value::Object(map) => {
            // Пустая выдача — тоже ответ: тогда узнаём её по счётчику рядом.
            let is_results = map.get("results")
                .and_then(Value::as_array)
                .is_some_and(|r| match r.first() {
                    Some(x) => id_of(&x["publishedfileid"]).is_some(),
                    None => map.contains_key("total_count"),
                });
            if is_results {
                return Some(v.clone());
            }
            map.values().find_map(find_browse_data)
        }
        Value::Array(arr) => arr.iter().find_map(find_browse_data),
        Value::String(s) => nested_json(s).and_then(|inner| find_browse_data(&inner)),
        _ => None,
    }
}

/// Собирает пары steamid → имя из любых объектов вида
/// `{"steamid": ..., "persona_name"/"personaname": ...}`.
fn collect_authors(v: &Value, out: &mut HashMap<String, String>) {
    match v {
        Value::Object(map) => {
            let name = map.get("persona_name").or_else(|| map.get("personaname")).and_then(Value::as_str);
            if let (Some(name), Some(id)) = (name, map.get("steamid")) {
                let id = steamid_of(id);
                if !id.is_empty() && !name.is_empty() {
                    out.insert(id, name.to_string());
                }
            }
            map.values().for_each(|x| collect_authors(x, out));
        }
        Value::Array(arr) => arr.iter().for_each(|x| collect_authors(x, out)),
        Value::String(s) => {
            if let Some(inner) = nested_json(s) {
                collect_authors(&inner, out);
            }
        }
        _ => {}
    }
}

fn nested_json(s: &str) -> Option<Value> {
    let t = s.trim_start();
    if !(t.starts_with('{') || t.starts_with('[')) {
        return None;
    }
    serde_json::from_str(s).ok()
}

/// Идентификатор предмета: Steam отдаёт его строкой, но вдруг передумает.
fn id_of(v: &Value) -> Option<u64> {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64())
}

fn steamid_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const RESULTS: &str = r#"{"current_page":1,"total_pages":3,"results":[{"publishedfileid":"42","title":"Мод","creator":"7656","preview_url":"https://x/p.png"}],"creator_player_link_details":[{"public_data":{"steamid":"7656","persona_name":"Автор"}}]}"#;

    fn query_data() -> String {
        let q = format!(r#"{{"queries":[{{"queryKey":["workshop_browse",{{}}],"state":{{"data":{RESULTS}}}}}]}}"#);
        serde_json::to_string(&serde_json::json!({ "queryData": q })).unwrap()
    }

    fn check(html: &str) {
        let roots = json_roots(html);
        let data = roots.iter().find_map(find_browse_data).expect("результаты не найдены");
        assert_eq!(id_of(&data["results"][0]["publishedfileid"]), Some(42));
        let mut authors = HashMap::new();
        roots.iter().for_each(|r| collect_authors(r, &mut authors));
        assert_eq!(authors.get("7656").map(String::as_str), Some("Автор"));
    }

    #[test]
    fn старый_формат_json_parse() {
        let literal = serde_json::to_string(&query_data()).unwrap();
        check(&format!("<script>window.SSR.renderContext=JSON.parse({literal});foo()</script>"));
    }

    #[test]
    fn новый_формат_script_application_json() {
        let ctx = query_data();
        check(&format!(
            r#"<script>x()</script><script type="application/json" id="valve-ssr-data">{{"route":"/workshop/browse","renderContext":{ctx}}}</script>"#
        ));
    }

    #[test]
    fn ключ_не_утекает_в_ошибку() {
        // Неразрешимый хост: ошибка гарантированно, и в ней будет адрес.
        let err = get_json_keyed("http://секретный-ключ.invalid/?key=секретный-ключ", "секретный-ключ")
            .unwrap_err()
            .to_string();
        assert!(!err.contains("секретный-ключ"), "{err}");
    }

    #[test]
    fn пустая_выдача_не_ошибка() {
        let html = r#"<script type="application/json">{"results":[],"total_count":0,"current_page":1,"total_pages":0}</script>"#;
        let data = json_roots(html).iter().find_map(find_browse_data).expect("пустая выдача не распознана");
        assert_eq!(data["results"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn без_обёртки_react_query() {
        check(&format!(r#"<script type="application/json">{{"anything":{{"deep":{RESULTS}}}}}</script>"#));
    }
}
