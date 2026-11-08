// Тесты анализатора логов RimWorld: разбор Player.log и атрибуция виновников.

use rust_rim::log_analysis::{analyze, parse_log, ModIndex, Severity};
use rust_rim::mod_data::{ModEntry, ModId, ModSource, Profile};

fn fake_mod(name: &str, package_id: &str, folder: &str) -> ModEntry {
    ModEntry {
        name: name.to_string(),
        package_id: ModId::new(package_id),
        version: String::new(),
        author: "author".into(),
        supported_versions: vec!["1.6".into()],
        path: std::path::PathBuf::from(format!("/home/user/Mods/{folder}")),
        source: ModSource::Local,
        dependencies: Vec::new(),
        dependency_sources: Vec::new(),
        load_after: Vec::new(),
        load_before: Vec::new(),
        incompatible_with: Vec::new(),
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
fn vanilla_mono_noise_is_not_an_error() {
    // Эти строки Mono печатает десятками при каждом запуске. Без фильтра
    // любой прогон -quicktest выглядел бы сбойным.
    let log = "Fallback handler could not load library X:/Games/RimWorld/data-0001.dll\n\
               Could not load signature of System.Foo:Bar\n\
               Could not load UnityEngine.Texture2D at Things/Chair from mod /home/user/Mods/CoolChairs/Textures: file not found\n";
    let issues = parse_log(log);

    assert_eq!(issues.len(), 1, "должна остаться только настоящая ошибка: {issues:#?}");
    assert!(issues[0].title.contains("Texture2D"), "{}", issues[0].title);
}

#[test]
fn localization_noise_is_ignored_together_with_its_stack() {
    // Эти строки есть в каждом запуске с непустым языком: игра подставляет
    // английский текст и работает дальше. Формы взяты из живых логов.
    //
    // Важна не только первая строка: у «Failed to resolve text» тянется
    // трасса через GrammarResolver, и её нельзя оставлять — иначе запись
    // всплывёт снова, уже без узнаваемого заголовка.
    let log = "Translation data for language Russian / Русский has 25 errors. Generate translation report for more info.\n\
               Failed to resolve text. Trying again with English. Exception: System.ArgumentNullException: Value cannot be null.\n\
                 at Verse.Grammar.GrammarResolver.ResolveUnsafe (System.String rootKeyword) [0x00000] in <hash>:0 \n\
                 at Verse.Grammar.GrammarResolver.Resolve (System.String rootKeyword) [0x0000e] in <hash>:0 \n\
               Grammar unresolvable. Root 'r_name'\n\
               Exception filling window for RimWorld.MainTabWindow_Inspect: System.NullReferenceException: boom\n";

    let issues = parse_log(log);
    assert_eq!(issues.len(), 1, "остаться должна только настоящая ошибка: {issues:#?}");
    assert!(issues[0].title.contains("MainTabWindow_Inspect"), "{}", issues[0].title);
}

#[test]
fn a_real_error_that_merely_mentions_translation_still_counts() {
    // Фильтр смотрит на начало строки, а не на вхождение: исключение внутри
    // мода-переводчика — настоящая ошибка, и прятать её нельзя.
    let log = "Exception in TranslationWorker: System.NullReferenceException: boom\n";
    let issues = parse_log(log);
    assert_eq!(issues.len(), 1, "{issues:#?}");
    assert_eq!(issues[0].severity, Severity::Error);
}

#[test]
fn attributes_suspects() {
    let mods = vec![
        fake_mod("Vanilla Furniture Expanded", "vanillaexpanded.vfecore", "VFECore"),
        fake_mod("Cool Chairs Pack", "cool.chairs.pack", "CoolChairs"),
        fake_mod("Unrelated Mod Here", "some.other.mod", "Unrelated"),
    ];
    // Третий мод в сборку не входит — подозреваемый должен помечаться неактивным.
    let mut profile = Profile::new();
    profile.activate(mods[0].package_id.clone());
    profile.activate(mods[1].package_id.clone());

    let parts: Vec<(&ModEntry, Vec<String>)> = vec![
        (&mods[0], vec!["VanillaFurnitureExpanded".into()]),
        (&mods[1], vec!["CoolChairs".into()]),
        (&mods[2], vec!["UnrelatedAssembly".into()]),
    ];
    let index = ModIndex::build_with_dlls(&parts, &profile);
    let issues = analyze(SAMPLE_LOG, &index);

    // 1. Неймспейс стека → DLL мода
    let inspect = issues.iter().find(|i| i.title.contains("MainTabWindow_Inspect")).unwrap();
    let top = inspect.suspects.first().expect("нет подозреваемых по стеку");
    assert_eq!(top.package_id.as_str(), "vanillaexpanded.vfecore", "suspects: {:?}", inspect.suspects);
    assert!(top.evidence.iter().any(|e| e.contains("стек")), "{:?}", top.evidence);

    // 2. Путь …/Mods/<папка>/… → мод
    let texture = issues.iter().find(|i| i.title.contains("Could not load UnityEngine.Texture2D")).unwrap();
    let top = texture.suspects.first().expect("нет подозреваемых по пути");
    assert_eq!(top.package_id.as_str(), "cool.chairs.pack", "suspects: {:?}", texture.suspects);

    // 3. packageId в тексте cross-reference ошибки
    let xref = issues.iter().find(|i| i.title.contains("cross-reference")).unwrap();
    assert!(xref.suspects.iter().any(|s| s.package_id.as_str() == "cool.chairs.pack"),
        "suspects: {:?}", xref.suspects);

    // 4. Ванильный стек с Harmony-патчем → подсказка, а не ложный виновник
    let job = issues.iter().find(|i| i.title.contains("JobDriver threw exception")).unwrap();
    assert!(job.suspects.is_empty(), "ложные подозреваемые: {:?}", job.suspects);
    let hint = job.harmony_hint.as_deref().expect("нет harmony-подсказки");
    assert!(hint.contains("SpawnSetup"), "{hint}");

    // Непричастный мод нигде не всплывает
    assert!(!issues.iter().flat_map(|i| &i.suspects)
        .any(|s| s.package_id.as_str() == "some.other.mod"));

    // Ошибки идут раньше предупреждений
    let first_warn = issues.iter().position(|i| i.severity == Severity::Warning);
    let last_err = issues.iter().rposition(|i| i.severity == Severity::Error);
    if let (Some(w), Some(e)) = (first_warn, last_err) {
        assert!(e < w || issues[w..].iter().all(|i| i.severity == Severity::Warning));
    }
}

// ─── Живые форматы Player.log с модом Harmony ────────────────────────────────
//
// Фрагменты ниже взяты из прогонов сборки с Performance Optimizer и Camera+,
// где старый разбор выдавал 118 групп и ни одного подозреваемого.

/// Первый прогон: дубликат стоит раньше оригинала, заголовки с именами пешек.
const HARMONY_RUN_1: &str = "\
RimWorld 1.6.4633 rev1261
Exception ticking Elk20919 (at (168, 0, 56)): System.NullReferenceException: Object reference not set to an instance of an object
[Ref EC178E71] Duplicate stacktrace, see ref for original
Exception ticking Singh (at (158, 0, 109)): System.NullReferenceException: Object reference not set to an instance of an object
[Ref EC178E71]
  at Verse.Pawn_HealthTracker.HealthTickInterval (System.Int32 delta) <0x6bc75370 + 0x00000> in <46372f5dadbf4af8939e608076251180>:0 
    - TRANSPILER PerformanceOptimizer.Main: IEnumerable`1 PerformanceOptimizer.Optimization_FasterGetCompReplacement:Transpiler(IEnumerable`1 instructions, MethodBase method)
  at Verse.Pawn.TickInterval (System.Int32 delta) [0x00073] in <46372f5dadbf4af8939e608076251180>:0 
  at Verse.Thing.DoTick () [0x000eb] in <46372f5dadbf4af8939e608076251180>:0 
  at Verse.TickList.Tick () [0x00139] in <46372f5dadbf4af8939e608076251180>:0 
Exception ticking Josh (at (161, 0, 105)): System.NullReferenceException: Object reference not set to an instance of an object
[Ref EC178E71] Duplicate stacktrace, see ref for original
Root level exception in OnGUI(): System.NullReferenceException: Object reference not set to an instance of an object
[Ref A14251EB]
  at RimWorld.GlobalControls.GlobalControlsOnGUI () <0x7f6b4ba0 + 0x00000> in <46372f5dadbf4af8939e608076251180>:0 
    - TRANSPILER PerformanceOptimizer.Main: IEnumerable`1 PerformanceOptimizer.Optimization_FasterGetCompReplacement:Transpiler(IEnumerable`1 instructions, MethodBase method)
    - PREFIX net.pardeike.rimworld.mod.camera+: Void CameraPlus.GlobalControlsEdgeInsetPatch:Prefix(Matrix4x4& __state)
    - POSTFIX net.pardeike.rimworld.mod.camera+: Void CameraPlus.GlobalControlsEdgeInsetPatch:Postfix()
  at RimWorld.MapInterface.MapInterfaceOnGUI_BeforeMainTabs () [0x00109] in <46372f5dadbf4af8939e608076251180>:0 
  at RimWorld.UIRoot_Play.UIRootOnGUI () [0x0001f] in <46372f5dadbf4af8939e608076251180>:0 
    - POSTFIX net.pardeike.rimworld.mod.camera+: Void CameraPlus.UIRoot_Play_UIRootOnGUI_Patch:Postfix()
  at Verse.Root.OnGUI () [0x00046] in <46372f5dadbf4af8939e608076251180>:0 
    - POSTFIX net.pardeike.rimworld.mod.camera+: Void CameraPlus.Root_OnGUI_Patch:Postfix()
[[Mac] VRE Highmate Retexture] Patch operation Verse.PatchOperationReplace(/Defs/GeneDef[defName=\"VRE_FelineEars\"]/renderNodeProperties/li/colorType) failed
file: X:\\Games\\RimWorld (2018)\\RimWorld\\Mods\\3465460387\\Patches\\CatgirlPatches.xml
Config error in Corpse_Autocleaner: Nutrition == 0 but preferability is DesperateOnly instead of NeverForNutrition
NullReferenceException: Object reference not set to an instance of an object
UnityEngine.DebugLogHandler:Internal_LogException(Exception, Object)
UnityEngine.Debug:LogException(Exception)
";

/// Второй прогон той же сборки: другой мир, другие пешки, и та же поломка
/// роняет другой ванильный метод.
const HARMONY_RUN_2: &str = "\
RimWorld 1.6.4633 rev1261
Exception ticking Caribou20936 (at (51, 0, 134)): System.NullReferenceException: Object reference not set to an instance of an object
[Ref 85DD0702]
  at RimWorld.InvisibilityUtility.IsHiddenFromPlayer (Verse.Pawn pawn) <0x6aeba570 + 0x00000> in <46372f5dadbf4af8939e608076251180>:0 
    - TRANSPILER PerformanceOptimizer.Main: IEnumerable`1 PerformanceOptimizer.Optimization_FasterGetCompReplacement:Transpiler(IEnumerable`1 instructions, MethodBase method)
  at Verse.Pawn.Tick () [0x00285] in <46372f5dadbf4af8939e608076251180>:0 
  at Verse.Thing.DoTick () [0x0007b] in <46372f5dadbf4af8939e608076251180>:0 
";

fn harmony_index() -> (Vec<ModEntry>, Profile) {
    let mods = vec![
        fake_mod("Performance Optimizer", "taranchuk.performanceoptimizer", "2664723367"),
        fake_mod("Camera+", "brrainz.cameraplus", "867467808"),
        fake_mod("[Mac] VRE Highmate Retexture", "mac.vre.highmate", "3465460387"),
    ];
    let mut profile = Profile::new();
    for m in &mods {
        profile.activate(m.package_id.clone());
    }
    (mods, profile)
}

fn harmony_issues(log: &str) -> Vec<rust_rim::log_analysis::LogIssue> {
    let (mods, profile) = harmony_index();
    let parts: Vec<(&ModEntry, Vec<String>)> = vec![
        (&mods[0], vec!["PerformanceOptimizer".into()]),
        (&mods[1], vec!["CameraPlus".into()]),
        (&mods[2], vec![]),
    ];
    analyze(log, &ModIndex::build_with_dlls(&parts, &profile))
}

#[test]
fn duplicates_of_one_stack_are_one_issue_with_the_original_stack() {
    let issues = harmony_issues(HARMONY_RUN_1);
    let ticking: Vec<_> = issues.iter().filter(|i| i.title.starts_with("Exception ticking")).collect();
    // Три заголовка с разными пешками — один стек, одна запись.
    assert_eq!(ticking.len(), 1, "{ticking:#?}");
    assert_eq!(ticking[0].count, 3);
    // Первым в логе стоял дубликат без стека — стек взят у оригинала.
    assert_eq!(ticking[0].frames[0].method, "Verse.Pawn_HealthTracker.HealthTickInterval");
    assert!(ticking[0].frames[0].patched);
}

#[test]
fn transpiler_on_the_throwing_frame_names_the_culprit() {
    let issues = harmony_issues(HARMONY_RUN_1);
    let ticking = issues.iter().find(|i| i.title.starts_with("Exception ticking")).unwrap();
    let top = ticking.suspects.first().expect("нет подозреваемых");
    assert_eq!(top.package_id.as_str(), "taranchuk.performanceoptimizer", "{:?}", ticking.suspects);
    assert!(top.evidence.iter().any(|e| e.contains("TRANSPILER")), "{:?}", top.evidence);

    // В ошибке интерфейса Camera+ тоже патчит верхний кадр, но транспайлер
    // весомее, а постфиксы на корнях стека уликой не считаются.
    let gui = issues.iter().find(|i| i.title.starts_with("Root level exception in OnGUI")).unwrap();
    assert_eq!(gui.suspects[0].package_id.as_str(), "taranchuk.performanceoptimizer", "{:?}", gui.suspects);
    assert!(gui.suspects.iter().any(|s| s.package_id.as_str() == "brrainz.cameraplus"));
}

#[test]
fn failed_patch_and_config_errors_are_errors() {
    let issues = harmony_issues(HARMONY_RUN_1);
    let patch = issues
        .iter()
        .find(|i| i.title.contains("Patch operation"))
        .expect("неудавшийся патч не распознан");
    assert_eq!(patch.severity, Severity::Error);
    // Мод назвал себя в скобках, и путь к его папке — в строке file:.
    assert_eq!(patch.suspects[0].package_id.as_str(), "mac.vre.highmate", "{:?}", patch.suspects);
    assert!(patch.full_text.contains("file: "), "строка file: потеряна");

    let config = issues.iter().find(|i| i.title.starts_with("Config error")).expect("Config error не распознан");
    assert_eq!(config.severity, Severity::Error);
}

#[test]
fn unity_stack_lines_are_not_separate_errors() {
    let issues = harmony_issues(HARMONY_RUN_1);
    assert!(
        !issues.iter().any(|i| i.title.starts_with("UnityEngine.")),
        "{:#?}",
        issues.iter().map(|i| &i.title).collect::<Vec<_>>(),
    );
}

#[test]
fn the_same_breakage_has_the_same_cause_in_another_run() {
    use rust_rim::bisect::Target;
    use rust_rim::testing::Verdict;

    let first = harmony_issues(HARMONY_RUN_1);
    let second = harmony_issues(HARMONY_RUN_2);
    let target = Target::from_issues(&first);
    let Target::Cause { key, .. } = &target else { panic!("{target:?}") };
    assert_eq!(key, "System.NullReferenceException|patched:PerformanceOptimizer.Main");
    // Ни одного общего заголовка — и всё же это та же ошибка.
    assert!(!second.iter().any(|b| first.iter().any(|a| a.title == b.title)));
    assert!(target.matches(&Verdict::LoadedWithErrors, &second));
}
