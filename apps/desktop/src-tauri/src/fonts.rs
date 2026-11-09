use hoshi_core::{SystemFont, group_font_families};

#[tauri::command(async)]
pub fn list_system_fonts() -> Vec<SystemFont> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    group_font_families(database.faces().filter_map(|face| {
        let (family, _) = face.families.first()?;
        Some((family.as_str(), face.monospaced))
    }))
}
