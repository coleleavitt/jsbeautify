use super::types::AngularDefKind;

/// Convert a CSS selector to `PascalCase` class name.
/// - `lms-app-root` → `LmsAppRootComponent`
/// - `[myDirective]` → `MyDirectiveDirective`
/// - `option` → `OptionComponent`
/// - `router-outlet` → `RouterOutletComponent`
pub(super) fn generate_class_name(selector: &str, kind: AngularDefKind) -> String {
    if selector.is_empty() {
        return String::new();
    }

    // Strip attribute selector brackets
    let clean = selector
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_start_matches('.');

    // Split on hyphens, dots, underscores
    let parts: Vec<&str> = clean.split(['-', '.', '_']).collect();

    let mut name = String::new();
    for part in parts {
        if !part.is_empty() {
            // Capitalize first letter
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                name.extend(first.to_uppercase());
                name.push_str(chars.as_str());
            }
        }
    }

    // Append the kind suffix if not already present
    let suffix = kind.suffix();
    if !name.ends_with(suffix) {
        name.push_str(suffix);
    }

    name
}
