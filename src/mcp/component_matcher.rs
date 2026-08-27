use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredComponent {
    pub name: String,
    pub file_path: String,
    pub import_path: String,
    pub is_default_export: bool,
    pub named_exports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComponentScanResult {
    pub total_components: usize,
    pub scanned_directories: Vec<String>,
    pub components: HashMap<String, DiscoveredComponent>,
}

/// Scans a project directory for existing UI components (.tsx, .jsx, .vue, .svelte)
pub async fn scan_project_components(base_dir: &str) -> ComponentScanResult {
    let mut result = ComponentScanResult::default();
    let search_dirs = vec![
        "src/components",
        "components",
        "src/ui",
        "app/components",
        "src/app/components",
    ];

    for rel_dir in search_dirs {
        let dir_path = Path::new(base_dir).join(rel_dir);
        if dir_path.exists() && dir_path.is_dir() {
            result.scanned_directories.push(rel_dir.to_string());
            let mut entries_to_visit = vec![dir_path];

            while let Some(current_dir) = entries_to_visit.pop() {
                if let Ok(mut read_dir) = tokio::fs::read_dir(&current_dir).await {
                    while let Ok(Some(entry)) = read_dir.next_entry().await {
                        let path = entry.path();
                        if path.is_dir() {
                            // Avoid node_modules / .git
                            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                if !name.starts_with('.') && name != "node_modules" && name != "dist" && name != "build" {
                                    entries_to_visit.push(path);
                                }
                            }
                        } else if path.is_file() {
                            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                                if matches!(ext, "tsx" | "jsx" | "vue" | "svelte") {
                                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                        if stem != "index" && !stem.ends_with(".test") && !stem.ends_with(".stories") {
                                            let comp_name = normalize_component_name(stem);
                                            let rel_file = path.strip_prefix(base_dir).unwrap_or(&path).to_string_lossy().to_string();

                                            // Derive standard import path: @/components/ui/button
                                            let import_path = format!("@/{}", rel_file.trim_start_matches('/').trim_end_matches(&format!(".{}", ext)));

                                            result.components.insert(comp_name.to_lowercase(), DiscoveredComponent {
                                                name: comp_name,
                                                file_path: rel_file,
                                                import_path,
                                                is_default_export: false,
                                                named_exports: Vec::new(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    result.total_components = result.components.len();
    result
}

/// Normalize file name (e.g. `button` or `user-card` -> `Button` or `UserCard`)
fn normalize_component_name(stem: &str) -> String {
    let parts: Vec<&str> = stem.split(|c| c == '-' || c == '_').collect();
    if parts.len() > 1 {
        parts
            .into_iter()
            .map(|p| {
                let mut c = p.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect()
    } else {
        let mut c = stem.chars();
        match c.next() {
            None => stem.to_string(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        }
    }
}

/// Matches a Figma node or ComponentSet name against discovered project components
pub fn match_figma_to_codebase_component<'a>(
    figma_name: &str,
    scan_result: &'a ComponentScanResult,
) -> Option<&'a DiscoveredComponent> {
    let clean = figma_name.trim().to_lowercase();
    
    // Direct match
    if let Some(comp) = scan_result.components.get(&clean) {
        return Some(comp);
    }

    // Normalized match (remove spaces and special chars)
    let stripped: String = clean.chars().filter(|c| c.is_alphanumeric()).collect();
    for (k, comp) in &scan_result.components {
        let k_stripped: String = k.chars().filter(|c| c.is_alphanumeric()).collect();
        if stripped == k_stripped {
            return Some(comp);
        }
    }

    // Prefix/Suffix Match (e.g., Figma "Primary Button" or "Button / Primary" -> "Button")
    for (k, comp) in &scan_result.components {
        if clean.contains(k) {
            return Some(comp);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_component_name() {
        assert_eq!(normalize_component_name("button"), "Button");
        assert_eq!(normalize_component_name("user-profile-card"), "UserProfileCard");
        assert_eq!(normalize_component_name("dialog_modal"), "DialogModal");
    }

    #[test]
    fn test_match_figma_to_codebase() {
        let mut scan = ComponentScanResult::default();
        scan.components.insert("button".to_string(), DiscoveredComponent {
            name: "Button".to_string(),
            file_path: "src/components/ui/button.tsx".to_string(),
            import_path: "@/components/ui/button".to_string(),
            is_default_export: false,
            named_exports: vec!["Button".to_string()],
        });

        let matched = match_figma_to_codebase_component("Button / Primary", &scan);
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "Button");
    }
}
