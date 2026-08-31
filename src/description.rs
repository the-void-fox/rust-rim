//! Описания модов → Markdown.
//!
//! В `About.xml` формально лежит Unity rich text (`<b>`, `<color=…>`), но
//! авторы массово вставляют туда разметку со страницы Steam Workshop —
//! BBCode (`[h2]`, `[table]`, `[img]`, `[list]`). Плюс переносы строк там
//! настоящие, а Markdown одиночный `\n` склеивает.
//!
//! Модуль сводит оба формата к Markdown, который умеет рендерить
//! `egui_commonmark`, и расставляет жёсткие переносы.

/// Схемы, по которым ссылку можно безопасно отдать в Markdown.
const SAFE_SCHEMES: [&str; 2] = ["http://", "https://"];

/// Максимальная длина видимого текста ссылки. Длинный URL — это один
/// неразрывный «слово», которое не переносится: строка описания становится
/// шире панели и ломает раскладку.
const MAX_LABEL: usize = 45;

/// Таблицы Markdown egui рисует без переноса строк, а панель деталей узкая
/// (порядка 40 знаков). Что не влезает — раскладывается строками.
const MAX_TABLE_WIDTH: usize = 60;
const MAX_TABLE_COLUMNS: usize = 3;

/// Как рендерить описание.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Options {
    /// Показывать картинки прямо в описании.
    ///
    /// Выключено по умолчанию: описания ссылаются на произвольные хосты,
    /// и выбор мода в списке превращался бы в сетевой запрос к ним.
    pub inline_images: bool,
}

/// Преобразует описание мода в Markdown (без загрузки картинок).
pub fn to_markdown(raw: &str) -> String {
    to_markdown_with(raw, Options::default())
}

pub fn to_markdown_with(raw: &str, opts: Options) -> String {
    let unescaped = unescape(raw);
    let mut out = String::with_capacity(unescaped.len() + unescaped.len() / 4);
    convert(&unescaped, opts, &mut out);
    tidy(&out)
}

/// Часть About.xml сгенерирована с экранированными переносами: в описании
/// лежит буквальный «\n» из двух символов, а не перевод строки.
fn unescape(raw: &str) -> std::borrow::Cow<'_, str> {
    if !raw.contains("\\n") && !raw.contains("\\t") {
        return std::borrow::Cow::Borrowed(raw);
    }
    std::borrow::Cow::Owned(
        raw.replace("\\r\\n", "\n").replace("\\n", "\n").replace("\\t", "\t"),
    )
}

// ─── Разбор разметки ─────────────────────────────────────────────────────────

fn convert(input: &str, opts: Options, out: &mut String) {
    let mut rest = input;
    while !rest.is_empty() {
        let Some(pos) = rest.find(['[', '<']) else {
            out.push_str(rest);
            return;
        };
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];

        let consumed = if rest.starts_with('[') {
            bbcode(rest, opts, out)
        } else {
            unity(rest, out)
        };

        match consumed {
            Some(n) => rest = &rest[n..],
            None => {
                // Не разметка — например «x < y» или «[WIP]» в названии.
                let ch = rest.chars().next().expect("rest не пуст");
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
}

/// Разбирает `[name]`, `[name=arg]` или `[/name]` в начале строки.
/// Возвращает (имя в нижнем регистре, аргумент, длину тега).
fn tag_at(s: &str) -> Option<(String, Option<String>, usize)> {
    let close = s.find(']')?;
    let body = &s[1..close];
    if body.is_empty() || body.contains('[') || body.contains('\n') {
        return None;
    }
    let (name, arg) = match body.split_once('=') {
        Some((n, a)) => (n, Some(a.trim_matches('"').to_string())),
        None => (body, None),
    };
    Some((name.trim().to_lowercase(), arg, close + 1))
}

/// Находит закрывающий `[/tag]`, возвращая (содержимое, длину вместе с тегом).
fn split_until_close<'a>(s: &'a str, tag: &str) -> Option<(&'a str, usize)> {
    let needle = format!("[/{tag}]");
    let at = s.to_lowercase().find(&needle)?;
    Some((&s[..at], at + needle.len()))
}

fn bbcode(s: &str, opts: Options, out: &mut String) -> Option<usize> {
    let (name, arg, tag_len) = tag_at(s)?;
    let after = &s[tag_len..];

    let inline = |marker: &str, out: &mut String| -> Option<usize> {
        let (inner, len) = split_until_close(after, &name)?;
        out.push_str(marker);
        convert(inner.trim_matches('\n'), opts, out);
        out.push_str(marker);
        Some(tag_len + len)
    };

    match name.as_str() {
        "b" => inline("**", out),
        "i" => inline("*", out),
        // Подчёркивания в Markdown нет — показываем как курсив, иначе
        // разметка просто исчезнет.
        "u" => inline("*", out),
        "strike" | "s" => inline("~~", out),

        "h1" | "h2" | "h3" => {
            let (inner, len) = split_until_close(after, &name)?;
            let hashes = "#".repeat(name[1..].parse::<usize>().unwrap_or(2));
            out.push_str("\n\n");
            out.push_str(&hashes);
            out.push(' ');
            convert(inner.trim(), opts, out);
            out.push_str("\n\n");
            Some(tag_len + len)
        }

        "url" => {
            let (inner, len) = split_until_close(after, "url")?;
            emit_link(arg.as_deref(), inner, out);
            Some(tag_len + len)
        }

        "img" => {
            let (inner, len) = split_until_close(after, "img")?;
            emit_image(arg.as_deref().unwrap_or(inner).trim(), opts, out);
            Some(tag_len + len)
        }

        "list" | "olist" => {
            let (inner, len) = split_until_close(after, &name)?;
            emit_list(inner, name == "olist", opts, out);
            Some(tag_len + len)
        }

        "table" => {
            let (inner, len) = split_until_close(after, "table")?;
            emit_table(inner, opts, out);
            Some(tag_len + len)
        }

        "code" => {
            let (inner, len) = split_until_close(after, "code")?;
            out.push_str("\n\n```\n");
            out.push_str(inner.trim_matches('\n'));
            out.push_str("\n```\n\n");
            Some(tag_len + len)
        }

        "noparse" => {
            let (inner, len) = split_until_close(after, "noparse")?;
            out.push_str(inner);
            Some(tag_len + len)
        }

        "quote" | "spoiler" => {
            let (inner, len) = split_until_close(after, &name)?;
            let mut body = String::new();
            convert(inner.trim(), opts, &mut body);
            out.push_str("\n\n");
            if name == "spoiler" {
                out.push_str("> 🔒 ");
                out.push_str(body.trim().replace('\n', "\n> ").as_str());
            } else {
                out.push_str("> ");
                out.push_str(body.trim().replace('\n', "\n> ").as_str());
            }
            out.push_str("\n\n");
            Some(tag_len + len)
        }

        "hr" => {
            out.push_str("\n\n---\n\n");
            Some(tag_len)
        }

        // Маркер элемента вне [list] — встречается в описаниях как есть.
        "*" => {
            out.push_str("\n- ");
            Some(tag_len)
        }

        _ => None,
    }
}

/// Unity rich text: `<b>`, `<i>`, `<color=…>`, `<size=…>`, `<br>`.
fn unity(s: &str, out: &mut String) -> Option<usize> {
    let close = s.find('>')?;
    let body = &s[1..close];
    if body.is_empty() || body.contains('<') || body.contains('\n') {
        return None;
    }
    let lower = body.to_ascii_lowercase();
    match lower.as_str() {
        "b" | "/b" => out.push_str("**"),
        "i" | "/i" => out.push_str("*"),
        "br" | "br/" => out.push('\n'),
        "/color" | "/size" | "/material" | "/quad" => {}
        _ if lower.starts_with("color=")
            || lower.starts_with("size=")
            || lower.starts_with("material=")
            || lower.starts_with("quad") => {}
        // Не тег форматирования (например «x < y») — оставляем как есть.
        _ => return None,
    }
    Some(close + 1)
}

// ─── Сборка Markdown-конструкций ─────────────────────────────────────────────

fn is_safe_url(url: &str) -> bool {
    let lower = url.trim().to_lowercase();
    SAFE_SCHEMES.iter().any(|s| lower.starts_with(s))
}

fn emit_link(target: Option<&str>, text: &str, out: &mut String) {
    let url = target.map(str::trim).unwrap_or(text.trim());
    let mut label = String::new();
    // Внутри подписи ссылки картинка всегда остаётся текстом: вложенных
    // ссылок в Markdown нет, а «[![](i)](l)» ломает разбор.
    convert(text.trim(), Options { inline_images: false }, &mut label);
    // В описаниях сплошь встречается картинка внутри ссылки:
    // [url=…][img]…[/img][/url]. Вложенных ссылок в Markdown нет — без
    // расплющивания получался «[[текст](A)](B)», который CommonMark
    // показывал сырым текстом вместе с длинными URL.
    let mut label = shorten(&flatten_links(&label));
    if label.is_empty() {
        label.push_str(&shorten(url));
    }

    if is_safe_url(url) {
        out.push('[');
        out.push_str(label.trim());
        out.push_str("](");
        out.push_str(url);
        out.push(')');
    } else {
        // Схемы вроде steam:// или javascript: в ссылку не превращаем.
        out.push_str(label.trim());
    }
}

/// Убирает разметку ссылок и картинок, оставляя видимый текст.
fn flatten_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        rest = &rest[open + 1..];
        let Some(close) = rest.find("](") else {
            out.push('[');
            continue;
        };
        let text = &rest[..close];
        let after = &rest[close + 2..];
        let Some(end) = after.find(')') else {
            out.push('[');
            continue;
        };
        out.push_str(text);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Укорачивает подпись, если это одно длинное «слово» (обычно голый URL).
fn shorten(label: &str) -> String {
    let label = label.trim();
    if label.chars().count() <= MAX_LABEL || label.contains(char::is_whitespace) {
        return label.to_string();
    }
    let head: String = label.chars().take(MAX_LABEL - 1).collect();
    format!("{head}…")
}

/// Картинки из описаний почти всегда лежат на CDN Steam.
///
/// Мы не тянем их автоматически: выбор мода в списке не должен порождать
/// сетевой запрос к постороннему хосту. Вместо этого — обычная ссылка,
/// открывается по клику.
fn emit_image(url: &str, opts: Options, out: &mut String) {
    if !is_safe_url(url) {
        return;
    }
    if opts.inline_images {
        out.push_str("![](");
        out.push_str(url);
        out.push(')');
        return;
    }
    out.push_str("[🖼 изображение](");
    out.push_str(url);
    out.push(')');
}

fn emit_list(inner: &str, ordered: bool, opts: Options, out: &mut String) {
    out.push('\n');
    for (i, item) in inner.split("[*]").skip(1).enumerate() {
        let mut text = String::new();
        convert(item.trim(), opts, &mut text);
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if ordered {
            out.push_str(&format!("{}. ", i + 1));
        } else {
            out.push_str("- ");
        }
        // Многострочный пункт складываем в одну строку: вложенные блоки
        // в описаниях модов не встречаются, а «висящий» перенос ломает список.
        out.push_str(&text.replace('\n', " "));
        out.push('\n');
    }
    out.push('\n');
}

fn emit_table(inner: &str, opts: Options, out: &mut String) {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut rest = inner;
    while let Some((row, consumed)) = next_row(rest, opts) {
        rows.push(row);
        rest = &rest[consumed..];
    }
    if rows.is_empty() {
        return;
    }

    let width = rows.iter().map(Vec::len).max().unwrap_or(0);

    // Широкую таблицу разворачиваем в строки: они переносятся, а таблица —
    // нет, и вылезает за панель, ломая раскладку всего окна.
    let widest_row = rows
        .iter()
        .map(|r| r.iter().map(|c| visible_len(c) + 3).sum::<usize>())
        .max()
        .unwrap_or(0);
    // С картинками таблица тем более не переносится — сразу строками.
    if opts.inline_images || width > MAX_TABLE_COLUMNS || widest_row > MAX_TABLE_WIDTH {
        out.push('\n');
        for row in &rows {
            let line: Vec<&str> = row.iter().map(String::as_str).filter(|c| !c.is_empty()).collect();
            if line.is_empty() {
                continue;
            }
            out.push_str(&line.join(" · ").replace("\\|", "|"));
            out.push('\n');
        }
        out.push('\n');
        return;
    }

    out.push('\n');
    for (i, row) in rows.iter().enumerate() {
        out.push_str("| ");
        for col in 0..width {
            out.push_str(row.get(col).map(String::as_str).unwrap_or(""));
            out.push_str(" | ");
        }
        out.push('\n');
        // Markdown требует строку-разделитель после заголовка; первую строку
        // таблицы всегда считаем заголовком, даже если она из [td].
        if i == 0 {
            out.push('|');
            for _ in 0..width {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out.push('\n');
}

/// Длина видимого текста: адреса ссылок на экран не попадают.
fn visible_len(cell: &str) -> usize {
    flatten_links(cell).chars().count()
}

/// Вырезает очередной `[tr]…[/tr]`, возвращая ячейки и съеденную длину.
fn next_row(s: &str, opts: Options) -> Option<(Vec<String>, usize)> {
    let lower = s.to_lowercase();
    let start = lower.find("[tr]")?;
    let body = &s[start + 4..];
    let (row, len) = split_until_close(body, "tr")?;

    let mut cells = Vec::new();
    let mut rest = row;
    loop {
        let lower = rest.to_lowercase();
        let th = lower.find("[th]");
        let td = lower.find("[td]");
        let (at, tag) = match (th, td) {
            (Some(a), Some(b)) if a < b => (a, "th"),
            (Some(_), Some(b)) => (b, "td"),
            (Some(a), None) => (a, "th"),
            (None, Some(b)) => (b, "td"),
            (None, None) => break,
        };
        let after = &rest[at + 4..];
        let Some((cell, consumed)) = split_until_close(after, tag) else { break };
        let mut text = String::new();
        convert(cell.trim(), opts, &mut text);
        // Вертикальная черта внутри ячейки разорвала бы таблицу.
        cells.push(text.trim().replace('\n', " ").replace('|', "\\|"));
        rest = &after[consumed..];
    }

    Some((cells, start + 4 + len))
}

// ─── Переносы строк ──────────────────────────────────────────────────────────

/// Одиночный `\n` в описании — настоящий перенос строки, а Markdown склеил бы
/// его с соседней строкой в один абзац. Дописываем жёсткий перенос (два
/// пробела), схлопываем лишние пустые строки и не трогаем блоки кода.
fn tidy(s: &str) -> String {
    let normalized = s.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    let mut out = String::with_capacity(normalized.len());
    let mut in_code = false;
    let mut blank_run = 0;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_end();

        if trimmed.trim_start().starts_with("```") {
            in_code = !in_code;
        }

        if !in_code && trimmed.trim().is_empty() {
            blank_run += 1;
            if blank_run == 1 {
                out.push('\n');
            }
            continue;
        }
        blank_run = 0;

        out.push_str(trimmed);

        let next_is_text = lines.get(i + 1).is_some_and(|n| !n.trim().is_empty());
        if !in_code && next_is_text && needs_hard_break(trimmed) {
            out.push_str("  ");
        }
        out.push('\n');
    }

    out.trim_matches('\n').to_string()
}

/// Блочные конструкции переносятся сами; жёсткий перенос им только мешает.
fn needs_hard_break(line: &str) -> bool {
    let t = line.trim_start();
    let list_item = t.starts_with("- ")
        || t.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    !(t.starts_with('#')
        || t.starts_with('|')
        || t.starts_with("---")
        || t.starts_with("```")
        || t.starts_with('>')
        || list_item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_single_newlines_as_line_breaks() {
        // Без жёсткого переноса Markdown склеил бы строки в один абзац.
        assert_eq!(to_markdown("первая\nвторая"), "первая  \nвторая");
    }

    #[test]
    fn collapses_runs_of_blank_lines() {
        assert_eq!(to_markdown("а\n\n\n\nб"), "а\n\nб");
    }

    #[test]
    fn converts_unity_tags() {
        assert_eq!(to_markdown("<b>жирный</b> и <i>курсив</i>"), "**жирный** и *курсив*");
        assert_eq!(to_markdown("<color=#ff0000>красный</color>"), "красный");
        assert_eq!(to_markdown("<size=20>крупный</size>"), "крупный");
    }

    #[test]
    fn keeps_non_tag_angle_brackets() {
        assert_eq!(to_markdown("если x < y и y > z"), "если x < y и y > z");
    }

    #[test]
    fn converts_bbcode_inline() {
        assert_eq!(to_markdown("[b]жирный[/b]"), "**жирный**");
        assert_eq!(to_markdown("[i]курсив[/i]"), "*курсив*");
        assert_eq!(to_markdown("[strike]зачёркнуто[/strike]"), "~~зачёркнуто~~");
    }

    #[test]
    fn converts_headings() {
        assert_eq!(to_markdown("[h1]Заголовок[/h1]"), "# Заголовок");
        assert_eq!(to_markdown("[h2]Раздел[/h2]"), "## Раздел");
        assert_eq!(to_markdown("текст[h3]Мелкий[/h3]ещё"), "текст\n\n### Мелкий\n\nещё");
    }

    #[test]
    fn converts_links() {
        assert_eq!(
            to_markdown("[url=https://example.com]сайт[/url]"),
            "[сайт](https://example.com)",
        );
        assert_eq!(
            to_markdown("[url]https://example.com[/url]"),
            "[https://example.com](https://example.com)",
        );
    }

    #[test]
    fn drops_unsafe_link_schemes() {
        // steam:// и javascript: в кликабельную ссылку не превращаем.
        assert_eq!(to_markdown("[url=javascript:alert(1)]клик[/url]"), "клик");
        assert_eq!(to_markdown("[url=steam://run/294100]запуск[/url]"), "запуск");
    }

    #[test]
    fn images_become_links_not_requests() {
        // Выбор мода не должен порождать запрос к постороннему хосту.
        assert_eq!(
            to_markdown("[img]https://cdn.example/a.png[/img]"),
            "[🖼 изображение](https://cdn.example/a.png)",
        );
        assert_eq!(to_markdown("[img]file:///etc/passwd[/img]"), "");
    }

    fn with_images(raw: &str) -> String {
        to_markdown_with(raw, Options { inline_images: true })
    }

    #[test]
    fn inline_images_are_opt_in() {
        let raw = "[img]https://cdn.example/a.png[/img]";
        assert_eq!(to_markdown(raw), "[🖼 изображение](https://cdn.example/a.png)");
        assert_eq!(with_images(raw), "![](https://cdn.example/a.png)");
    }

    #[test]
    fn inline_image_inside_link_stays_flat() {
        // Даже с включёнными картинками подпись ссылки остаётся текстом:
        // «[![](i)](l)» ломает разбор Markdown.
        let md = with_images(
            "[url=https://discord.gg/x][img]https://cdn.example/a.png[/img][/url]",
        );
        assert_eq!(md, "[🖼 изображение](https://discord.gg/x)");
    }

    #[test]
    fn inline_images_force_table_to_lines() {
        // Картинки делают таблицу заведомо широкой, а переносить её egui
        // не умеет — раскладываем строками.
        let md = with_images(
            "[table][tr][td][img]https://cdn.example/a.png[/img][/td]\
             [td][img]https://cdn.example/b.png[/img][/td][/tr][/table]",
        );
        assert!(!md.contains('|'), "{md}");
        assert!(md.contains("![](https://cdn.example/a.png)"), "{md}");
    }

    #[test]
    fn converts_lists() {
        assert_eq!(to_markdown("[list][*]раз[*]два[/list]"), "- раз\n- два");
        assert_eq!(to_markdown("[olist][*]раз[*]два[/olist]"), "1. раз\n2. два");
    }

    #[test]
    fn converts_tables() {
        let md = to_markdown("[table][tr][th]имя[/th][th]цена[/th][/tr][tr][td]хлеб[/td][td]5[/td][/tr][/table]");
        assert_eq!(md, "| имя | цена |\n| --- | --- |\n| хлеб | 5 |");
    }

    #[test]
    fn wide_table_falls_back_to_lines() {
        // Панель деталей узкая, а таблицу egui не переносит: широкая таблица
        // вылезала за панель и ломала раскладку окна.
        let md = to_markdown(
            "[table][tr][td]довольно длинная ячейка с текстом[/td]\
             [td]и вторая такая же длинная ячейка[/td][/tr][/table]",
        );
        assert!(!md.contains('|'), "широкая таблица не должна остаться таблицей: {md}");
        assert!(md.contains(" · "), "{md}");
    }

    #[test]
    fn many_columns_fall_back_to_lines() {
        let md = to_markdown("[table][tr][td]a[/td][td]b[/td][td]c[/td][td]d[/td][/tr][/table]");
        assert_eq!(md, "a · b · c · d");
    }

    #[test]
    fn narrow_table_stays_a_table() {
        let md = to_markdown("[table][tr][th]имя[/th][th]цена[/th][/tr][tr][td]хлеб[/td][td]5[/td][/tr][/table]");
        assert!(md.starts_with("| имя"), "{md}");
    }

    #[test]
    fn table_cells_escape_pipes() {
        let md = to_markdown("[table][tr][td]a|b[/td][/tr][/table]");
        assert!(md.contains("a\\|b"), "{md}");
    }

    #[test]
    fn nested_markup_inside_blocks() {
        assert_eq!(to_markdown("[list][*][b]жир[/b][/list]"), "- **жир**");
        assert_eq!(to_markdown("[h2][b]Заголовок[/b][/h2]"), "## **Заголовок**");
    }

    #[test]
    fn code_block_keeps_newlines_verbatim() {
        let md = to_markdown("[code]строка1\nстрока2[/code]");
        assert_eq!(md, "```\nстрока1\nстрока2\n```");
        // Внутри кода жёсткие переносы не дописываются.
        assert!(!md.contains("  \n"));
    }

    #[test]
    fn noparse_keeps_markup_literal() {
        assert_eq!(to_markdown("[noparse][b]не тег[/b][/noparse]"), "[b]не тег[/b]");
    }

    #[test]
    fn quote_and_spoiler_become_blockquotes() {
        assert_eq!(to_markdown("[quote]цитата[/quote]"), "> цитата");
        assert!(to_markdown("[spoiler]секрет[/spoiler]").starts_with("> 🔒"));
    }

    #[test]
    fn unclosed_tag_is_left_alone() {
        // Обрезанное описание не должно съедать остаток текста.
        assert_eq!(to_markdown("[b]без закрытия"), "[b]без закрытия");
        assert_eq!(to_markdown("текст [WIP] ещё"), "текст [WIP] ещё");
    }

    #[test]
    fn literal_backslash_n_becomes_line_break() {
        // Часть About.xml сгенерирована с экранированными переносами:
        // в описании лежит «\\n» из двух символов.
        assert_eq!(
            to_markdown("Mod Version: 1.8\\n\\n\\nRemove overhead"),
            "Mod Version: 1.8\n\nRemove overhead",
        );
        assert_eq!(to_markdown("a\\tb"), "a\tb");
    }

    #[test]
    fn image_inside_link_does_not_nest() {
        // Реальный шаблон из описаний Workshop: картинка-бейдж как ссылка.
        // Вложенных ссылок в Markdown нет — подпись обязана стать плоской,
        // иначе CommonMark показывает сырой текст с длинными URL и строка
        // распирает панель по ширине.
        let md = to_markdown(
            "[url=https://discord.gg/h5TY6DA][img]https://img.litet.net/logos/Discord.png[/img][/url]",
        );
        assert_eq!(md, "[🖼 изображение](https://discord.gg/h5TY6DA)");
        assert_eq!(md.matches("](").count(), 1, "ссылка должна быть ровно одна: {md}");
        assert!(!md.contains("[["), "вложенной ссылки быть не должно: {md}");
    }

    #[test]
    fn long_url_label_is_shortened() {
        let md = to_markdown(
            "[url]https://steamcommunity.com/sharedfiles/filedetails/?id=788610933&very=long[/url]",
        );
        let label = md.split_once("](").unwrap().0.trim_start_matches('[');
        assert!(label.chars().count() <= MAX_LABEL, "подпись длиной {}: {label}", label.chars().count());
        assert!(label.ends_with('…'));
        // Сама ссылка при этом остаётся целой.
        assert!(md.contains("id=788610933&very=long"), "{md}");
    }

    #[test]
    fn badge_table_rows_stay_narrow() {
        // Таблица из бейджей-ссылок — самый частый источник широкого контента.
        let md = to_markdown(
            "[table][tr]\
             [td][url=https://discord.gg/h5TY6DA][img]https://img.litet.net/logos/Discord.png[/img][/url][/td]\
             [td][url=https://github.com/emipa606/Bridges][img]https://img.litet.net/logos/GitHub.png[/img][/url][/td]\
             [/tr][/table]",
        );
        let widest = md.lines().map(|l| l.chars().count()).max().unwrap_or(0);
        assert!(widest < 120, "строка таблицы слишком широкая ({widest}): {md}");
        assert!(!md.contains("img.litet.net"), "URL картинки не должен попадать в текст: {md}");
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(to_markdown("обычное описание мода"), "обычное описание мода");
        assert_eq!(to_markdown(""), "");
    }

    #[test]
    fn horizontal_rule() {
        assert_eq!(to_markdown("до[hr]после"), "до\n\n---\n\nпосле");
    }
}
