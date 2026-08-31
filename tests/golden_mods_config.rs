// Golden-тесты ModsConfig.xml: чтение порядка загрузки и сохранение полей,
// которые rust-rim не редактирует (version, knownExpansions).
//
// Порча этого файла = порча конфига игры у пользователя, поэтому поведение
// зафиксировано до рефакторинга доменного слоя.

use rust_rim::mod_data::{parse_mods_config, write_mod_list, write_mods_config};

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("rustrim_modsconfig_{}_{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ModsConfigData>
	<version>1.6.4633 rev1261</version>
	<activeMods>
		<li>zetrith.prepatcher</li>
		<li>brrainz.harmony</li>
		<li>ludeon.rimworld</li>
		<li>ludeon.rimworld.royalty</li>
	</activeMods>
	<knownExpansions>
		<li>ludeon.rimworld.royalty</li>
		<li>ludeon.rimworld.ideology</li>
	</knownExpansions>
</ModsConfigData>
"#;

#[test]
fn reads_active_mods_in_load_order() {
    let dir = tmp_dir("read");
    let path = dir.join("ModsConfig.xml");
    std::fs::write(&path, SAMPLE).unwrap();

    let active = parse_mods_config(&path).unwrap();

    assert_eq!(
        active,
        ids(&[
            "zetrith.prepatcher",
            "brrainz.harmony",
            "ludeon.rimworld",
            "ludeon.rimworld.royalty",
        ]),
    );
}

#[test]
fn known_expansions_are_not_read_as_active() {
    // knownExpansions лежит рядом с activeMods и содержит те же <li> —
    // парсер обязан различать их по родителю.
    let dir = tmp_dir("known_not_active");
    let path = dir.join("ModsConfig.xml");
    std::fs::write(&path, SAMPLE).unwrap();

    let active = parse_mods_config(&path).unwrap();

    assert_eq!(active.len(), 4);
    assert!(!active.contains(&"ludeon.rimworld.ideology".to_string()));
}

#[test]
fn write_preserves_version_and_known_expansions() {
    let dir = tmp_dir("preserve");
    let path = dir.join("ModsConfig.xml");
    std::fs::write(&path, SAMPLE).unwrap();

    write_mods_config(&path, &ids(&["ludeon.rimworld", "some.new.mod"])).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();

    // Версия игры и список известных DLC принадлежат игре, не менеджеру.
    assert!(written.contains("<version>1.6.4633 rev1261</version>"));
    assert!(written.contains("<knownExpansions>"));
    assert!(written.contains("<li>ludeon.rimworld.ideology</li>"));

    // А активный список — заменён целиком.
    assert_eq!(
        parse_mods_config(&path).unwrap(),
        ids(&["ludeon.rimworld", "some.new.mod"]),
    );
}

#[test]
fn write_then_read_round_trips() {
    let dir = tmp_dir("roundtrip");
    let path = dir.join("ModsConfig.xml");
    std::fs::write(&path, SAMPLE).unwrap();

    let order = ids(&["c.mod", "a.mod", "b.mod", "ludeon.rimworld"]);
    write_mods_config(&path, &order).unwrap();

    assert_eq!(parse_mods_config(&path).unwrap(), order);
}

#[test]
fn write_to_missing_file_uses_defaults() {
    let dir = tmp_dir("missing");
    let path = dir.join("ModsConfig.xml");

    write_mods_config(&path, &ids(&["ludeon.rimworld"])).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();

    assert!(written.contains("<version>1.0.0</version>"));
    assert!(!written.contains("knownExpansions"));
    assert_eq!(parse_mods_config(&path).unwrap(), ids(&["ludeon.rimworld"]));
}

#[test]
fn exported_mod_list_is_readable_back() {
    // Экспорт списка (совместимый с RimSort) должен читаться тем же парсером.
    let dir = tmp_dir("modlist");
    let path = dir.join("my_list.xml");

    let order = ids(&["brrainz.harmony", "ludeon.rimworld", "some.mod"]);
    write_mod_list(&path, &order).unwrap();

    assert_eq!(parse_mods_config(&path).unwrap(), order);
    // Экспорт списка не тащит за собой knownExpansions.
    assert!(!std::fs::read_to_string(&path).unwrap().contains("knownExpansions"));
}

#[test]
fn empty_active_list_is_valid() {
    let dir = tmp_dir("empty");
    let path = dir.join("ModsConfig.xml");
    std::fs::write(&path, SAMPLE).unwrap();

    write_mods_config(&path, &[]).unwrap();

    assert!(parse_mods_config(&path).unwrap().is_empty());
}
