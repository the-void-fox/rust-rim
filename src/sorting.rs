use std::collections::{BinaryHeap, HashMap};
use std::cmp::Reverse;
use serde::Deserialize;
use crate::mod_data::{ModEntry, ModSource};

const COMMUNITY_RULES_URL: &str =
    "https://raw.githubusercontent.com/RimSort/Community-Rules-Database/main/communityRules.json";

const CORE_ID: &str = "ludeon.rimworld";

// ─── Порядок DLC по релизу (RimSort: RIMWORLD_DLC_METADATA) ──────────────────
// Core обрабатывается отдельно через ModSource::Core.
// Неизвестные DLC получают индекс 99.
const DLC_RELEASE_ORDER: &[&str] = &[
    "ludeon.rimworld.royalty",   // 0 — Royalty
    "ludeon.rimworld.ideology",  // 1 — Ideology
    "ludeon.rimworld.biotech",   // 2 — Biotech
    "ludeon.rimworld.anomaly",   // 3 — Anomaly
    "ludeon.rimworld.odyssey",   // 4 — Odyssey
];

// ─── Тир 0: мода обязательно до Core (RimSort: KNOWN_TIER_ZERO_MODS) ─────────
// Дополняет обнаружение по loadBefore Core в About.xml.
const KNOWN_TIER_ZERO_MODS: &[&str] = &[
    "zetrith.prepatcher",
    "brrainz.harmony",
    "brrainz.visualexceptions",
    "fishery.core",
    "gottimeline.loadingprogress",
    "fastergameloading.core",
    "justincc.sitecore",
];

// ─── Тир 1: фреймворки (RimSort: KNOWN_TIER_ONE_MODS + loadTop=true) ─────────
// Загружаются сразу после Core/DLC, до обычных модов.
const KNOWN_TIER_ONE_MODS: &[&str] = &[
    "oskarpotocki.vanillafactionsexpanded.core",
    "vanillaexpanded.backgrounds",
    "unlimitedhugs.hugslib",
    "imranfish.xmlextensions",
    "smashphil.vehicleframework",
    "redmattis.betterprerequisites",
    "owlchemist.cherrypicker",
    "adaptive.storage.framework",
    "aoba.framework",
    "aoba.exosuit.framework",
    "ebsg.framework",
    "thesepeople.ritualattachableoutcomes",
    "ohno.asf.ab.local",
];

// ─── Тир 3: загружаются последними (RimSort: KNOWN_TIER_THREE_MODS) ──────────
const KNOWN_TIER_THREE_MODS: &[&str] = &[
    "krkr.rocketman",
];

// ─── Публичные структуры ─────────────────────────────────────────────────────

pub struct CommunityRules {
    pub timestamp: u64,
    rules: HashMap<String, ModRule>,
}

struct ModRule {
    load_before: Vec<String>,
    load_after:  Vec<String>,
    load_top:    bool,
    load_bottom: bool,
}

// ─── JSON-десериализация ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RawRules {
    timestamp: u64,
    rules: HashMap<String, RawModRule>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawModRule {
    #[serde(rename = "loadBefore")]
    load_before: HashMap<String, serde_json::Value>,
    #[serde(rename = "loadAfter")]
    load_after: HashMap<String, serde_json::Value>,
    #[serde(rename = "loadTop")]
    load_top: Option<LoadFlag>,
    #[serde(rename = "loadBottom")]
    load_bottom: Option<LoadFlag>,
}

#[derive(Deserialize)]
struct LoadFlag {
    #[serde(default)]
    value: bool,
}

impl From<RawRules> for CommunityRules {
    fn from(raw: RawRules) -> Self {
        let rules = raw.rules
            .into_iter()
            .map(|(id, r)| {
                let rule = ModRule {
                    load_before: r.load_before.into_keys().map(|s| s.to_lowercase()).collect(),
                    load_after:  r.load_after.into_keys().map(|s| s.to_lowercase()).collect(),
                    load_top:    r.load_top.map(|f| f.value).unwrap_or(false),
                    load_bottom: r.load_bottom.map(|f| f.value).unwrap_or(false),
                };
                (id.to_lowercase(), rule)
            })
            .collect();
        CommunityRules { timestamp: raw.timestamp, rules }
    }
}

// ─── Загрузка правил ─────────────────────────────────────────────────────────

pub fn fetch_community_rules() -> anyhow::Result<CommunityRules> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .build()
        .into();
    let raw: RawRules = agent
        .get(COMMUNITY_RULES_URL)
        .call()
        .map_err(|e| anyhow::anyhow!("{}", e))?
        .body_mut()
        .read_json()?;
    Ok(CommunityRules::from(raw))
}

// ─── Вспомогательные функции ──────────────────────────────────────────────────

fn dlc_release_index(package_id: &str) -> u8 {
    DLC_RELEASE_ORDER
        .iter()
        .position(|&id| id == package_id)
        .map(|i| i as u8)
        .unwrap_or(99)
}

fn has_loadbefore_core(m: &ModEntry, rules: Option<&CommunityRules>) -> bool {
    if m.load_before.iter().any(|id| id == CORE_ID) {
        return true;
    }
    rules
        .and_then(|r| r.rules.get(&m.package_id))
        .map(|r| r.load_before.iter().any(|id| id == CORE_ID))
        .unwrap_or(false)
}

// ─── Топологическая сортировка ────────────────────────────────────────────────

/// Сортирует активные моды по многоуровневой системе тиров (по аналогии с RimSort):
///
/// **Тиры:**
/// - **Тир 0** — pre-Core: Harmony, PrePatcher и т.п.; мода с `loadBefore Core`;
///              их dependencies (через BFS по обратным рёбрам).
/// - **Тир 1** — Core (ludeon.rimworld)
/// - **Тир 2** — DLC в порядке релиза: Royalty → Ideology → Biotech → Anomaly → Odyssey
/// - **Тир 3** — Фреймворки: loadTop-мода, KNOWN_TIER_ONE_MODS и их рекурсивные зависимости
/// - **Тир 4** — Обычные моды
/// - **Тир 5** — loadBottom-мода (RocketMan и т.п.) и зависимые от них мода
///
/// Внутри тира тай-брейк — алфавитный порядок по имени мода.
pub fn sort_active_mods(mods: &mut Vec<ModEntry>, rules: Option<&CommunityRules>) {
    let positions: Vec<usize> = mods.iter().enumerate()
        .filter(|(_, m)| m.is_active)
        .map(|(i, _)| i)
        .collect();

    if positions.len() < 2 { return; }

    let active: Vec<ModEntry> = positions.iter().map(|&i| mods[i].clone()).collect();
    let n = active.len();

    let id_to_local: HashMap<&str, usize> = active.iter().enumerate()
        .map(|(i, m)| (m.package_id.as_str(), i))
        .collect();

    // ── Базовые флаги loadTop / loadBottom из community rules ─────────────────
    let is_load_top: Vec<bool> = active.iter().map(|m| {
        rules.and_then(|r| r.rules.get(&m.package_id))
             .map(|r| r.load_top)
             .unwrap_or(false)
    }).collect();

    let is_load_bottom: Vec<bool> = active.iter().map(|m| {
        rules.and_then(|r| r.rules.get(&m.package_id))
             .map(|r| r.load_bottom)
             .unwrap_or(false)
    }).collect();

    // ── Граф рёбер: edges[a] = b означает a должен идти ДО b ─────────────────
    let mut edges:     Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_degree: Vec<usize>      = vec![0; n];

    let mut add_edge = |from: usize, to: usize| {
        if from != to && !edges[from].contains(&to) {
            edges[from].push(to);
            in_degree[to] += 1;
        }
    };

    for (idx, m) in active.iter().enumerate() {
        // loadAfter + dependencies → dep должен идти ДО этого мода
        for id in m.load_after.iter().chain(m.dependencies.iter()) {
            if let Some(&dep) = id_to_local.get(id.as_str()) {
                add_edge(dep, idx);
            }
        }
        // loadBefore → этот мод должен идти ДО target
        for id in &m.load_before {
            if let Some(&b) = id_to_local.get(id.as_str()) {
                add_edge(idx, b);
            }
        }
        // Правила сообщества
        if let Some(rule) = rules.and_then(|r| r.rules.get(&m.package_id)) {
            for id in &rule.load_after {
                if let Some(&dep) = id_to_local.get(id.as_str()) {
                    add_edge(dep, idx);
                }
            }
            for id in &rule.load_before {
                if let Some(&b) = id_to_local.get(id.as_str()) {
                    add_edge(idx, b);
                }
            }
        }
    }

    // Обратный граф (нужен для BFS расширения тиров 0 и 3)
    let mut rev_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (a, neighbors) in edges.iter().enumerate() {
        for &b in neighbors {
            rev_edges[b].push(a);
        }
    }

    // ── Тир 0 (pre-Core): BFS по обратным рёбрам ─────────────────────────────
    // Начало: KNOWN_TIER_ZERO_MODS + моды с loadBefore Core
    let is_dlc_or_core = |m: &ModEntry| matches!(m.source, ModSource::Core | ModSource::DLC(_));

    let mut is_pre_core: Vec<bool> = active.iter().map(|m| {
        !is_dlc_or_core(m)
            && (KNOWN_TIER_ZERO_MODS.contains(&m.package_id.as_str())
                || has_loadbefore_core(m, rules))
    }).collect();

    // Если X — pre-Core и a→X (a грузится до X), то a тоже pre-Core
    {
        let mut bfs: std::collections::VecDeque<usize> = is_pre_core
            .iter().enumerate()
            .filter(|&(_, &v)| v)
            .map(|(i, _)| i)
            .collect();
        while let Some(x) = bfs.pop_front() {
            for &a in &rev_edges[x] {
                if !is_pre_core[a] && !is_dlc_or_core(&active[a]) {
                    is_pre_core[a] = true;
                    bfs.push_back(a);
                }
            }
        }
    }

    // ── Тир 3 (loadTop / фреймворки): BFS по обратным рёбрам ─────────────────
    // Начало: KNOWN_TIER_ONE_MODS + мода с loadTop=true из community rules
    let mut is_framework: Vec<bool> = (0..n).map(|i| {
        !is_dlc_or_core(&active[i])
            && !is_pre_core[i]
            && (KNOWN_TIER_ONE_MODS.contains(&active[i].package_id.as_str())
                || is_load_top[i])
    }).collect();

    // Если X — framework и a→X, то a тоже framework
    {
        let mut bfs: std::collections::VecDeque<usize> = is_framework
            .iter().enumerate()
            .filter(|&(_, &v)| v)
            .map(|(i, _)| i)
            .collect();
        while let Some(x) = bfs.pop_front() {
            for &a in &rev_edges[x] {
                if !is_framework[a] && !is_dlc_or_core(&active[a]) && !is_pre_core[a] {
                    is_framework[a] = true;
                    bfs.push_back(a);
                }
            }
        }
    }

    // ── Тир 5 (loadBottom): BFS по прямым рёбрам ─────────────────────────────
    // Если X — loadBottom и X→b (b грузится после X), то b тоже loadBottom
    let mut is_load_bottom_exp: Vec<bool> = (0..n).map(|i| {
        !is_dlc_or_core(&active[i])
            && (KNOWN_TIER_THREE_MODS.contains(&active[i].package_id.as_str())
                || is_load_bottom[i])
    }).collect();

    {
        let mut bfs: std::collections::VecDeque<usize> = is_load_bottom_exp
            .iter().enumerate()
            .filter(|&(_, &v)| v)
            .map(|(i, _)| i)
            .collect();
        while let Some(x) = bfs.pop_front() {
            for &b in &edges[x] {
                if !is_load_bottom_exp[b] && !is_dlc_or_core(&active[b]) {
                    is_load_bottom_exp[b] = true;
                    bfs.push_back(b);
                }
            }
        }
    }

    // ── Ключ приоритета: (тир, суб-тир) ──────────────────────────────────────
    // Тир 0: pre-Core        → (0, 0)
    // Тир 1: Core            → (1, 0)
    // Тир 2: DLC             → (2, <индекс релиза>)
    // Тир 3: Фреймворки      → (3, 0)
    // Тир 4: Обычные         → (4, 0)
    // Тир 5: loadBottom      → (5, 0)
    let priority_key = |idx: usize| -> (u8, u8) {
        if is_pre_core[idx] { return (0, 0); }
        match &active[idx].source {
            ModSource::Core   => (1, 0),
            ModSource::DLC(_) => (2, dlc_release_index(&active[idx].package_id)),
            _ if is_framework[idx]         => (3, 0),
            _ if is_load_bottom_exp[idx]   => (5, 0),
            _                              => (4, 0),
        }
    };

    // ── Алгоритм Кана с приоритетной очередью ────────────────────────────────
    // Ключ кучи: (тир, суб-тир, имя_lowercase, idx)
    // При равных тире и суб-тире — алфавитный порядок по имени.
    let mut heap: BinaryHeap<Reverse<(u8, u8, String, usize)>> = BinaryHeap::new();
    for i in 0..n {
        if in_degree[i] == 0 {
            let (t, s) = priority_key(i);
            heap.push(Reverse((t, s, active[i].name.to_lowercase(), i)));
        }
    }

    let mut sorted: Vec<usize> = Vec::with_capacity(n);
    while let Some(Reverse((_, _, _, idx))) = heap.pop() {
        sorted.push(idx);
        for next in edges[idx].clone() {
            in_degree[next] -= 1;
            if in_degree[next] == 0 {
                let (t, s) = priority_key(next);
                heap.push(Reverse((t, s, active[next].name.to_lowercase(), next)));
            }
        }
    }

    // Циклические зависимости: добавляем в порядке приоритета
    let mut remaining: Vec<usize> = (0..n).filter(|&i| in_degree[i] > 0).collect();
    remaining.sort_by(|&a, &b| {
        let ka = priority_key(a);
        let kb = priority_key(b);
        ka.cmp(&kb)
            .then_with(|| active[a].name.to_lowercase().cmp(&active[b].name.to_lowercase()))
    });
    if !remaining.is_empty() {
        tracing::warn!(
            "Circular dependencies detected among {} mods: {:?}",
            remaining.len(),
            remaining.iter().map(|&i| &active[i].package_id).collect::<Vec<_>>()
        );
    }
    sorted.extend(remaining);

    let sorted_mods: Vec<ModEntry> = sorted.iter().map(|&i| active[i].clone()).collect();
    for (pos, entry) in positions.iter().zip(sorted_mods) {
        mods[*pos] = entry;
    }
}
