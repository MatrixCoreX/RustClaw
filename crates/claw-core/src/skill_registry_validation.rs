pub(super) fn validate_named_capability(
    name: &str,
    token: &str,
    label: &str,
) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err(format!("{label} name length must be 1..=64: `{token}`"));
    }
    if !name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
    }) {
        return Err(format!("{label} name must match [a-z0-9_]: `{token}`"));
    }
    Ok(())
}

pub(super) fn validate_required_companion_capabilities(
    registry: &SkillsRegistry,
    path: &Path,
) -> Result<(), String> {
    let declared = registry
        .by_name
        .values()
        .flat_map(|entry| {
            entry
                .planner_capabilities
                .iter()
                .filter(|mapping| !entry.planner_capability_aliases.contains_key(&mapping.name))
        })
        .map(|mapping| mapping.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for (skill_name, entry) in &registry.by_name {
        for mapping in &entry.planner_capabilities {
            for companion in &mapping.required_companions {
                if companion == &mapping.name {
                    return Err(format!(
                        "planner capability `{}` in skill `{skill_name}` cannot require itself as a companion in {}",
                        mapping.name,
                        path.display()
                    ));
                }
                if !declared.contains(companion.as_str()) {
                    return Err(format!(
                        "planner capability `{}` in skill `{skill_name}` requires unknown companion `{companion}` in {}",
                        mapping.name,
                        path.display()
                    ));
                }
            }
        }
    }
    Ok(())
}
use std::path::Path;

use super::SkillsRegistry;
