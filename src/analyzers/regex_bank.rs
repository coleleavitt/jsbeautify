//! Built-in secret/credential detection regexes.
//!
//! Patterns recovered from jxscout-pro-v2 v2.1.1 binary
//! (`crates/core/analyzers/src/regex_analyzer.rs`).
//!
//! Match-kind names match jxscout's catalog so output is drop-in compatible
//! with `get-matches --json`.

use lazy_static::lazy_static;
use regex::Regex;

/// A single named regex rule.
#[derive(Debug, Clone)]
pub struct SecretRule {
    pub kind: &'static str,
    pub pattern: &'static str,
    pub regex: &'static Regex,
    /// Whether the pattern targets *property names* (object keys, JSON keys),
    /// in which case it should be matched against identifier-context only.
    pub is_property_name: bool,
}

macro_rules! lazy_regex {
    ($name:ident, $pat:expr) => {
        lazy_static! {
            static ref $name: Regex = Regex::new($pat).expect(concat!("Failed to compile ", stringify!($name)));
        }
    };
}

// ── Literal-value patterns ─────────────────────────────────────────────────────
lazy_regex!(GITHUB_PAT, r"ghp_[0-9a-zA-Z]{36}");
lazy_regex!(SLACK_TOKEN, r"xox[baprs]-[0-9a-zA-Z\-]{10,48}");
lazy_regex!(JWT, r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+");
lazy_regex!(PRIVATE_KEY, r"-----BEGIN (?:RSA |EC )?PRIVATE KEY-----");
lazy_regex!(STRIPE_SECRET_KEY, r"(?:sk|rk)_(?:live|test)_[0-9a-zA-Z]{24,}");
lazy_regex!(
    GOOGLE_CALENDAR_URI,
    r"https://www\.google\.com/calendar/embed\?src=[A-Za-z0-9%@&;=\-_./]+"
);
lazy_regex!(
    GOOGLE_FORMS,
    r#"https://docs\.google\.com/forms/(?:d|d/e)/[A-Za-z0-9_-]+(?:/|[?#][^\s"'<>]*)?"#
);
lazy_regex!(
    GOOGLE_DOCS,
    r#"https://docs\.google\.com/(?:document|spreadsheets|presentation|drawings)/d/[A-Za-z0-9_-]+(?:/|[?#][^\s"'<>]*)?"#
);

// ── Property-name patterns (case-insensitive identifier matchers) ──────────────
lazy_regex!(P_SECRET, r"(?i)secret");
lazy_regex!(P_PASSWD, r"(?i)passwd");
lazy_regex!(P_AUTHORIZATION, r"(?i)authorization");
lazy_regex!(P_AWS_ACCESS_KEY_ID, r"(?i)aws[_-]?access[_-]?key[_-]?id");
lazy_regex!(P_AWS_SECRET_ACCESS_KEY, r"(?i)aws[_-]?secret[_-]?access[_-]?key");
lazy_regex!(P_ACCESS, r"(?i)access[_-]?(?:key|token|secret)");
lazy_regex!(P_AUTH, r"(?i)auth[_-]?(?:key|token|secret)");
lazy_regex!(P_SESSION, r"(?i)session[_-]?(?:key|token|secret)");
lazy_regex!(P_CONSUMER, r"(?i)consumer[_-]?(?:key|token|secret)");
lazy_regex!(P_PUBLIC, r"(?i)public[_-]?(?:key|token|secret)");
lazy_regex!(P_CLIENT, r"(?i)client[_-]?(?:id|token|key)");
lazy_regex!(P_SLACK_BOT, r"(?i)slack[_-]?bot[_-]?token");
lazy_regex!(P_SLACK, r"(?i)slack[_-]?token");
lazy_regex!(P_ID_DSA, r"(?i)id[_-]?dsa");
lazy_regex!(P_SSH, r"(?i)ssh[_-]?key");
lazy_regex!(P_ENCRYPT, r"(?i)encrypt[_-]?(?:secret|key)");
lazy_regex!(P_DECRYPT, r"(?i)decrypt[_-]?(?:secret|key)");
lazy_regex!(P_GITHUB, r"(?i)github[_-]?(?:key|token|secret)");
lazy_regex!(P_IRC_PASS, r"(?i)irc[_-]?pass");
lazy_regex!(P_API, r"(?i)api[_-]?(?:key|token|secret)");

/// All built-in rules, queryable as a slice.
pub fn all_rules() -> Vec<SecretRule> {
    vec![
        SecretRule {
            kind: "github_pat",
            pattern: GITHUB_PAT.as_str(),
            regex: &GITHUB_PAT,
            is_property_name: false,
        },
        SecretRule {
            kind: "slack_token",
            pattern: SLACK_TOKEN.as_str(),
            regex: &SLACK_TOKEN,
            is_property_name: false,
        },
        SecretRule {
            kind: "jwt",
            pattern: JWT.as_str(),
            regex: &JWT,
            is_property_name: false,
        },
        SecretRule {
            kind: "private_key",
            pattern: PRIVATE_KEY.as_str(),
            regex: &PRIVATE_KEY,
            is_property_name: false,
        },
        SecretRule {
            kind: "stripe_secret_key",
            pattern: STRIPE_SECRET_KEY.as_str(),
            regex: &STRIPE_SECRET_KEY,
            is_property_name: false,
        },
        SecretRule {
            kind: "google_calendar_uri",
            pattern: GOOGLE_CALENDAR_URI.as_str(),
            regex: &GOOGLE_CALENDAR_URI,
            is_property_name: false,
        },
        SecretRule {
            kind: "google_forms",
            pattern: GOOGLE_FORMS.as_str(),
            regex: &GOOGLE_FORMS,
            is_property_name: false,
        },
        SecretRule {
            kind: "google_docs",
            pattern: GOOGLE_DOCS.as_str(),
            regex: &GOOGLE_DOCS,
            is_property_name: false,
        },
        SecretRule {
            kind: "sensitive_property_name:secret",
            pattern: P_SECRET.as_str(),
            regex: &P_SECRET,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:passwd",
            pattern: P_PASSWD.as_str(),
            regex: &P_PASSWD,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:authorization",
            pattern: P_AUTHORIZATION.as_str(),
            regex: &P_AUTHORIZATION,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:aws_access_key_id",
            pattern: P_AWS_ACCESS_KEY_ID.as_str(),
            regex: &P_AWS_ACCESS_KEY_ID,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:aws_secret_access_key",
            pattern: P_AWS_SECRET_ACCESS_KEY.as_str(),
            regex: &P_AWS_SECRET_ACCESS_KEY,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:access",
            pattern: P_ACCESS.as_str(),
            regex: &P_ACCESS,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:auth",
            pattern: P_AUTH.as_str(),
            regex: &P_AUTH,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:session",
            pattern: P_SESSION.as_str(),
            regex: &P_SESSION,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:consumer",
            pattern: P_CONSUMER.as_str(),
            regex: &P_CONSUMER,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:public",
            pattern: P_PUBLIC.as_str(),
            regex: &P_PUBLIC,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:client",
            pattern: P_CLIENT.as_str(),
            regex: &P_CLIENT,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:slack_bot_token",
            pattern: P_SLACK_BOT.as_str(),
            regex: &P_SLACK_BOT,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:slack_token",
            pattern: P_SLACK.as_str(),
            regex: &P_SLACK,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:id_dsa",
            pattern: P_ID_DSA.as_str(),
            regex: &P_ID_DSA,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:ssh_key",
            pattern: P_SSH.as_str(),
            regex: &P_SSH,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:encrypt",
            pattern: P_ENCRYPT.as_str(),
            regex: &P_ENCRYPT,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:decrypt",
            pattern: P_DECRYPT.as_str(),
            regex: &P_DECRYPT,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:github",
            pattern: P_GITHUB.as_str(),
            regex: &P_GITHUB,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:irc_pass",
            pattern: P_IRC_PASS.as_str(),
            regex: &P_IRC_PASS,
            is_property_name: true,
        },
        SecretRule {
            kind: "sensitive_property_name:api",
            pattern: P_API.as_str(),
            regex: &P_API,
            is_property_name: true,
        },
    ]
}

/// Rules whose patterns match against full source text (literal value rules).
pub fn literal_rules() -> Vec<SecretRule> {
    all_rules().into_iter().filter(|r| !r.is_property_name).collect()
}

/// Rules whose patterns match against property-name strings only.
pub fn property_name_rules() -> Vec<SecretRule> {
    all_rules().into_iter().filter(|r| r.is_property_name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_pat_matches() {
        let rule = all_rules().into_iter().find(|r| r.kind == "github_pat").unwrap();
        assert!(rule.regex.is_match("token=ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));
    }

    #[test]
    fn jwt_matches() {
        let rule = all_rules().into_iter().find(|r| r.kind == "jwt").unwrap();
        assert!(
            rule.regex
                .is_match("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4eHgifQ.signature_part_here")
        );
    }

    #[test]
    fn stripe_matches() {
        let rule = all_rules().into_iter().find(|r| r.kind == "stripe_secret_key").unwrap();
        assert!(rule.regex.is_match("sk_live_XXXXXXXXXXXXXXXXFAKE000"));
        assert!(rule.regex.is_match("rk_test_XXXXXXXXXXXXXXXXFAKE000"));
    }

    #[test]
    fn private_key_matches() {
        let rule = all_rules().into_iter().find(|r| r.kind == "private_key").unwrap();
        assert!(rule.regex.is_match("-----BEGIN PRIVATE KEY-----"));
        assert!(rule.regex.is_match("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(rule.regex.is_match("-----BEGIN EC PRIVATE KEY-----"));
    }

    #[test]
    fn property_name_classifier() {
        assert_eq!(literal_rules().len() + property_name_rules().len(), all_rules().len());
        assert!(property_name_rules().iter().all(|r| r.is_property_name));
        assert!(literal_rules().iter().all(|r| !r.is_property_name));
    }
}
