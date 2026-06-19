//! Hostname scope matching with wildcard globs.
//!
//! Ports `crates/core/scope/src/lib.rs` from jxscout-pro-v2.
//!
//! Supports patterns like:
//!   - `example.com` — exact match
//!   - `*.example.com` — any subdomain
//!   - `*.api.target.io` — deep subdomain wildcard
//!   - `*internal*` — contains match
//!
//! Used to filter analyzer results to only in-scope targets.

/// A compiled scope rule set.
#[derive(Debug, Clone)]
pub struct ScopeMatcher {
    in_scope: Vec<ScopePattern>,
    out_of_scope: Vec<ScopePattern>,
}

#[derive(Debug, Clone)]
struct ScopePattern {
    /// How to match.
    kind: PatternKind,
}

#[derive(Debug, Clone)]
enum PatternKind {
    /// Exact hostname match (case-insensitive).
    Exact(String),
    /// `*.suffix` — any subdomain of suffix.
    SubdomainWildcard(String),
    /// `*contains*` — substring match.
    Contains(String),
    /// Regex-based (for complex patterns).
    Regex(regex::Regex),
}

impl ScopeMatcher {
    /// Create a new scope matcher from in-scope and out-of-scope pattern lists.
    ///
    /// Patterns are comma-separated strings or pre-split slices.
    #[must_use]
    pub fn new(in_scope: &[&str], out_of_scope: &[&str]) -> Self {
        Self {
            in_scope: in_scope.iter().map(|p| ScopePattern::parse(p)).collect(),
            out_of_scope: out_of_scope.iter().map(|p| ScopePattern::parse(p)).collect(),
        }
    }

    /// Parse from comma-separated strings (jxscout CLI format).
    #[must_use]
    pub fn from_csv(in_scope_csv: &str, out_of_scope_csv: &str) -> Self {
        let parse_csv = |s: &str| -> Vec<ScopePattern> {
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(ScopePattern::parse)
                .collect()
        };
        Self {
            in_scope: parse_csv(in_scope_csv),
            out_of_scope: parse_csv(out_of_scope_csv),
        }
    }

    /// Check if a hostname/URL is in scope.
    ///
    /// Logic:
    /// 1. If `out_of_scope` matches → NOT in scope (exclusions take precedence)
    /// 2. If `in_scope` is empty → everything is in scope
    /// 3. If `in_scope` matches → in scope
    /// 4. Otherwise → NOT in scope
    #[must_use]
    pub fn is_in_scope(&self, hostname: &str) -> bool {
        let host = extract_hostname(hostname);

        // Exclusions take precedence
        if self.out_of_scope.iter().any(|p| p.matches(&host)) {
            return false;
        }

        // Empty in_scope = allow all
        if self.in_scope.is_empty() {
            return true;
        }

        // Must match at least one in_scope pattern
        self.in_scope.iter().any(|p| p.matches(&host))
    }

    /// Filter a list of values, keeping only in-scope ones.
    #[must_use]
    pub fn filter_in_scope<'a>(&self, values: &'a [String]) -> Vec<&'a String> {
        values.iter().filter(|v| self.is_in_scope(v)).collect()
    }
}

impl ScopePattern {
    fn parse(pattern: &str) -> Self {
        let p = pattern.trim().to_ascii_lowercase();

        let kind = p.strip_prefix("*.").map_or_else(
            || {
                if p.starts_with('*') && p.ends_with('*') && p.len() > 2 {
                    PatternKind::Contains(p[1..p.len() - 1].to_string())
                } else if p.contains('*') || p.contains('?') {
                    let regex_str = glob_to_regex(&p);
                    regex::Regex::new(&regex_str).map_or_else(|_| PatternKind::Exact(p.clone()), PatternKind::Regex)
                } else {
                    PatternKind::Exact(p.clone())
                }
            },
            |rest| PatternKind::SubdomainWildcard(rest.to_string()),
        );

        Self { kind }
    }

    fn matches(&self, hostname: &str) -> bool {
        let host = hostname.to_ascii_lowercase();
        match &self.kind {
            PatternKind::Exact(p) => host == *p,
            PatternKind::SubdomainWildcard(suffix) => host == *suffix || host.ends_with(&format!(".{suffix}")),
            PatternKind::Contains(sub) => host.contains(sub.as_str()),
            PatternKind::Regex(re) => re.is_match(&host),
        }
    }
}

/// Convert a simple glob pattern to a regex string.
fn glob_to_regex(glob: &str) -> String {
    let mut regex = String::from("^");
    for ch in glob.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                regex.push('\\');
                regex.push(ch);
            }
            _ => regex.push(ch),
        }
    }
    regex.push('$');
    regex
}

/// Extract hostname from a URL or return as-is if already a hostname.
fn extract_hostname(input: &str) -> String {
    let s = input.trim();
    // Strip protocol
    let without_proto = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .or_else(|| s.strip_prefix("//"))
        .unwrap_or(s);
    // Take until path/port
    without_proto
        .split('/')
        .next()
        .unwrap_or(without_proto)
        .split(':')
        .next()
        .unwrap_or(without_proto)
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        let scope = ScopeMatcher::new(&["example.com"], &[]);
        assert!(scope.is_in_scope("example.com"));
        assert!(scope.is_in_scope("https://example.com/api"));
        assert!(!scope.is_in_scope("sub.example.com"));
        assert!(!scope.is_in_scope("other.com"));
    }

    #[test]
    fn wildcard_subdomain() {
        let scope = ScopeMatcher::new(&["*.example.com"], &[]);
        assert!(scope.is_in_scope("api.example.com"));
        assert!(scope.is_in_scope("deep.sub.example.com"));
        assert!(scope.is_in_scope("example.com")); // bare domain matches too
        assert!(!scope.is_in_scope("notexample.com"));
    }

    #[test]
    fn contains_pattern() {
        let scope = ScopeMatcher::new(&["*internal*"], &[]);
        assert!(scope.is_in_scope("internal.corp.com"));
        assert!(scope.is_in_scope("api-internal.example.com"));
        assert!(!scope.is_in_scope("external.com"));
    }

    #[test]
    fn out_of_scope_takes_precedence() {
        let scope = ScopeMatcher::new(&["*.example.com"], &["admin.example.com"]);
        assert!(scope.is_in_scope("api.example.com"));
        assert!(!scope.is_in_scope("admin.example.com"));
    }

    #[test]
    fn empty_in_scope_allows_all() {
        let scope = ScopeMatcher::new(&[], &["blocked.com"]);
        assert!(scope.is_in_scope("anything.com"));
        assert!(!scope.is_in_scope("blocked.com"));
    }

    #[test]
    fn csv_parsing() {
        let scope = ScopeMatcher::from_csv("*.target.io, internal.corp.com", "cdn.target.io");
        assert!(scope.is_in_scope("api.target.io"));
        assert!(scope.is_in_scope("internal.corp.com"));
        assert!(!scope.is_in_scope("cdn.target.io"));
        assert!(!scope.is_in_scope("other.com"));
    }
}
