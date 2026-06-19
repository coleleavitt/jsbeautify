pub(super) fn annotate_webpack_modules(code: &str) -> String {
    let mut result = String::with_capacity(code.len() + 4096);
    let mut count = 0u32;

    for line in code.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("var ")
            && let Some(name_end) = rest.find(" = v(() =>")
        {
            let module_name = &rest[..name_end];
            if !module_name.is_empty() && module_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') {
                result.push_str("// ═══════════════════════════════════════\n");
                result.push_str("// Webpack Module: ");
                result.push_str(module_name);
                result.push('\n');
                result.push_str("// ═══════════════════════════════════════\n");
                count = count.wrapping_add(1);
            }
        }
        result.push_str(line);
        result.push('\n');
    }

    if count > 0 {
        eprintln!("[DEOBFUSCATE] Phase 19: Annotated {count} webpack modules");
    }

    result
}
