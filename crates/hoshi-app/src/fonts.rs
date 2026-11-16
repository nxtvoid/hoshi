use hoshi_core::{SystemFont, group_font_families};

/// The font families installed on the machine, one per name. Reads every font file, so it takes
/// a moment: call it off the UI thread.
pub fn system_fonts() -> Vec<SystemFont> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    group_font_families(database.faces().filter_map(|face| {
        let (family, _) = face.families.first()?;
        Some((family.as_str(), face.monospaced))
    }))
}
