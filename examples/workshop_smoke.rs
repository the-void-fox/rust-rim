// Дымовой тест Workshop API: cargo run --example workshop_smoke
use rust_rim::steam::workshop_api;
use workshop_api::SortOrder;

fn main() -> anyhow::Result<()> {
    let (items, has_next) = workshop_api::fetch_workshop_page("medieval", 1, SortOrder::Trending)?;
    println!("Модов: {} (has_next: {has_next})", items.len());
    for it in items.iter().take(5) {
        println!("  {} — {} (by {})", it.id, it.title, it.author);
    }
    let (colls, _) = workshop_api::fetch_collections_page("", 1, SortOrder::Trending)?;
    println!("Коллекций: {}", colls.len());
    if let Some(c) = colls.first() {
        println!("  {} — {} (by {})", c.id, c.title, c.author);
        let (title, mods) = workshop_api::fetch_collection_mods(c.id)?;
        println!("  Содержимое '{}': {} модов", title, mods.len());
    }
    Ok(())
}
