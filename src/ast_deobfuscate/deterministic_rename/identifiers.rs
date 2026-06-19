pub(super) fn extract_identifiers(source: &str) -> Vec<String> {
    let mut identifiers = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut string_char = ' ';
    let mut prev_char = ' ';

    for c in source.chars() {
        if !in_string && (c == '"' || c == '\'' || c == '`') {
            in_string = true;
            string_char = c;
            prev_char = c;
            continue;
        }

        if in_string {
            if c == string_char && prev_char != '\\' {
                in_string = false;
            }
            prev_char = c;
            continue;
        }

        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            current.push(c);
        } else if !current.is_empty() {
            if current
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_' || ch == '$')
                && current.len() <= 4
            {
                identifiers.push(current.clone());
            }
            current.clear();
        }

        prev_char = c;
    }

    if !current.is_empty()
        && current
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_' || ch == '$')
        && current.len() <= 4
    {
        identifiers.push(current);
    }

    identifiers
}
