// Тесты анализатора логов RimWorld: разбор Player.log и атрибуция виновников.

use rust_rim::log_analysis::{analyze, parse_log, ModIndex, Severity};
use rust_rim::mod_data::{ModEntry, ModSource};

fn fake_mod(name: &str, package_id: &str, folder: &str, active: bool) -> ModEntry {
    ModEntry {
        name: name.to_string(),
        package_id: package_id.to_string(),
        version: String::new(),
        author: "author".into(),
        supported_versions: vec!["1.6".into()],
        path: std::path::PathBuf::from(format!("/home/user/Mods/{folder}")),
        source: ModSource::Local,
        dependencies: Vec::new(),
        load_after: Vec::new(),
        load_before: Vec::new(),
        incompatible_with: Vec::new(),
        is_active: active,
        description: String::new(),
        preview_path: None,
    }
}

const SAMPLE_LOG: &str = r#"Mono path[0] = '/home/user/RimWorld/RimWorldLinux_Data/Managed'
Initialize engine version: 2019.4.30f1
RimWorld 1.6.4518 rev641

Exception filling window for RimWorld.MainTabWindow_Inspect: System.NullReferenceException: Object reference not set to an instance of an object
  at VanillaFurnitureExpanded.CompRefuelableStats.get_Props () [0x00000] in <9madeuphash>:0
  at RimWorld.InspectPaneUtility.DoTabs (RimWorld.IInspectPane pane) [0x00089] in <hash2>:0
  at Verse.Root.Update () [0x00012] in <hash3>:0

(Filename: /home/builduser/buildslave/unity/build/Runtime/Export/Debug/Debug.bindings.h Line: 39)

Loaded file is from an older version
Exception filling window for RimWorld.MainTabWindow_Inspect: System.NullReferenceException: Object reference not set to an instance of an object
  at VanillaFurnitureExpanded.CompRefuelableStats.get_Props () [0x00000] in <9madeuphash>:0
  at RimWorld.InspectPaneUtility.DoTabs (RimWorld.IInspectPane pane) [0x00089] in <hash2>:0

(Filename: /home/builduser/... Line: 39)

Could not load UnityEngine.Texture2D at Things/Building/Chair from mod /home/user/Mods/CoolChairs/Textures: file not found

Warning: DefName duplicate: Chair_Fancy defined twice

JobDriver threw exception in initAction for pawn Alice driver=JobDriver_Meditate
System.NullReferenceException: Object reference not set to an instance of an object
  at (wrapper dynamic-method) Verse.Pawn.Verse.Pawn.SpawnSetup_Patch3(Verse.Pawn,Verse.Map,bool)
  at Verse.AI.JobDriver.ReadyForNextToil () [0x00031] in <hash4>:0

(Filename: ... Line: 0)

Could not resolve cross-reference: No Verse.ThingDef named FancyTable_marker found to give to RimWorld.StuffCategoryDef cool.chairs.pack extra info

Loading finished, 0 errors
"#;

#[test]
fn parses_and_groups() {
    let issues = parse_log(SAMPLE_LOG);

    // Повторное исключение схлопнуто в count=2
    let inspect = issues.iter()
        .find(|i| i.title.contains("MainTabWindow_Inspect"))
        .expect("нет записи про MainTabWindow_Inspect");
    assert_eq!(inspect.count, 2);
    assert_eq!(inspect.severity, Severity::Error);
    assert!(inspect.frames.len() >= 2, "кадры стека не собраны: {:?}", inspect.frames);

    // Предупреждение распознано
    assert!(issues.iter().any(|i| i.severity == Severity::Warning
        && i.title.contains("DefName duplicate")));

    // Статистика «0 errors» не считается ошибкой
    assert!(!issues.iter().any(|i| i.title.contains("Loading finished")));
}

#[test]
fn attributes_suspects() {
    let mods = vec![
        fake_mod("Vanilla Furniture Expanded", "vanillaexpanded.vfecore", "VFECore", true),
        fake_mod("Cool Chairs Pack", "cool.chairs.pack", "CoolChairs", true),
        fake_mod("Unrelated Mod Here", "some.other.mod", "Unrelated", false),
    ];
    let parts: Vec<(&ModEntry, Vec<String>)> = vec![
        (&mods[0], vec!["VanillaFurnitureExpanded".into()]),
        (&mods[1], vec!["CoolChairs".into()]),
        (&mods[2], vec!["UnrelatedAssembly".into()]),
    ];
    let index = ModIndex::build_with_dlls(&parts);
    let issues = analyze(SAMPLE_LOG, &index);

    // 1. Неймспейс стека → DLL мода
    let inspect = issues.iter().find(|i| i.title.contains("MainTabWindow_Inspect")).unwrap();
    let top = inspect.suspects.first().expect("нет подозреваемых по стеку");
    assert_eq!(top.package_id, "vanillaexpanded.vfecore", "suspects: {:?}", inspect.suspects);
    assert!(top.evidence.iter().any(|e| e.contains("стек")), "{:?}", top.evidence);

    // 2. Путь …/Mods/<папка>/… → мод
    let texture = issues.iter().find(|i| i.title.contains("Could not load UnityEngine.Texture2D")).unwrap();
    let top = texture.suspects.first().expect("нет подозреваемых по пути");
    assert_eq!(top.package_id, "cool.chairs.pack", "suspects: {:?}", texture.suspects);

    // 3. packageId в тексте cross-reference ошибки
    let xref = issues.iter().find(|i| i.title.contains("cross-reference")).unwrap();
    assert!(xref.suspects.iter().any(|s| s.package_id == "cool.chairs.pack"),
        "suspects: {:?}", xref.suspects);

    // 4. Ванильный стек с Harmony-патчем → подсказка, а не ложный виновник
    let job = issues.iter().find(|i| i.title.contains("JobDriver threw exception")).unwrap();
    assert!(job.suspects.is_empty(), "ложные подозреваемые: {:?}", job.suspects);
    let hint = job.harmony_hint.as_deref().expect("нет harmony-подсказки");
    assert!(hint.contains("SpawnSetup"), "{hint}");

    // Непричастный мод нигде не всплывает
    assert!(!issues.iter().flat_map(|i| &i.suspects)
        .any(|s| s.package_id == "some.other.mod"));

    // Ошибки идут раньше предупреждений
    let first_warn = issues.iter().position(|i| i.severity == Severity::Warning);
    let last_err = issues.iter().rposition(|i| i.severity == Severity::Error);
    if let (Some(w), Some(e)) = (first_warn, last_err) {
        assert!(e < w || issues[w..].iter().all(|i| i.severity == Severity::Warning));
    }
}
