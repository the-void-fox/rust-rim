// Регрессия миграции на quick-xml 0.41: сущности (&amp;) приходят отдельным
// событием GeneralRef, текст должен аккумулироваться, CDATA — поддерживаться.

use rust_rim::mod_data::parser::parse_about_xml;

fn write_about(dir: &std::path::Path, xml: &str) -> std::path::PathBuf {
    let about = dir.join("About");
    std::fs::create_dir_all(&about).unwrap();
    let path = about.join("About.xml");
    std::fs::write(&path, xml).unwrap();
    path
}

#[test]
fn entities_and_cdata() {
    let tmp = std::env::temp_dir().join(format!("rustrim_parser_test_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();

    let path = write_about(&tmp, r#"<?xml version="1.0" encoding="utf-8"?>
<ModMetaData>
    <name>Cats &amp; Dogs &#8212; Extended</name>
    <packageId>Author.CatsAndDogs</packageId>
    <author>A &lt;B&gt; C</author>
    <description><![CDATA[Первая строка <b>жирным</b>
и вторая & сырой амперсанд]]></description>
    <supportedVersions><li>1.5</li><li>1.6</li></supportedVersions>
    <modDependencies>
        <li><packageId>Ludeon.RimWorld</packageId></li>
    </modDependencies>
</ModMetaData>"#);

    let data = parse_about_xml(&path).unwrap();
    assert_eq!(data.name, "Cats & Dogs — Extended");
    assert_eq!(data.package_id, "Author.CatsAndDogs");
    assert_eq!(data.author, "A <B> C");
    assert!(data.description.contains("Первая строка <b>жирным</b>"), "{}", data.description);
    assert!(data.description.contains("вторая & сырой амперсанд"), "{}", data.description);
    assert_eq!(data.supported_versions, vec!["1.5", "1.6"]);
    assert_eq!(data.dependencies, vec!["Ludeon.RimWorld"]);

    let _ = std::fs::remove_dir_all(&tmp);
}
