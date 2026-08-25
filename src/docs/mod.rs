pub const SECTION_INDEX: &str = include_str!("section_index.md");
pub const SECTION_DEFAULT: &str = include_str!("section_default.md");
pub const SECTION_RULES: &str = include_str!("section_rules.md");
pub const SECTION_LAYOUT: &str = include_str!("section_layout.md");
pub const SECTION_API: &str = include_str!("section_api.md");
pub const SECTION_TOKENS: &str = include_str!("section_tokens.md");
pub const SECTION_ICONS: &str = include_str!("section_icons.md");

pub fn get_docs(section: Option<&str>) -> String {
    match section {
        Some("rules") => SECTION_RULES.to_string(),
        Some("layout") => SECTION_LAYOUT.to_string(),
        Some("api") => SECTION_API.to_string(),
        Some("tokens") => SECTION_TOKENS.to_string(),
        Some("icons") => SECTION_ICONS.to_string(),
        Some(_) => format!("{}\n\n{}", SECTION_INDEX, SECTION_DEFAULT),
        None => format!("{}\n\n{}", SECTION_INDEX, SECTION_DEFAULT),
    }
}
