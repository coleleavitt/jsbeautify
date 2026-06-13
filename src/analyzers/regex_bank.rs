//! Built-in secret/credential detection regexes.
//!
//! Patterns from jxscout-pro-v2 v2.1.1 + extractify + trufflehog + gitleaks.
//! 60+ literal patterns covering all major cloud providers and services.

use lazy_static::lazy_static;
use regex::Regex;

/// A single named regex rule.
#[derive(Debug, Clone)]
pub struct SecretRule {
    pub kind: &'static str,
    pub pattern: &'static str,
    pub regex: &'static Regex,
    pub is_property_name: bool,
    pub severity: Severity,
}

/// Finding severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "info" => Some(Self::Info),
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "high" => Some(Self::High),
            "critical" | "crit" => Some(Self::Critical),
            _ => None,
        }
    }
}

macro_rules! lazy_regex {
    ($name:ident, $pat:expr) => {
        lazy_static! {
            static ref $name: Regex = Regex::new($pat).expect(concat!("Failed to compile ", stringify!($name)));
        }
    };
}

// ══════════════════════════════════════════════════════════════════════════════
// LITERAL-VALUE PATTERNS (60+)
// ══════════════════════════════════════════════════════════════════════════════

// ── AWS ──
lazy_regex!(
    AWS_KEY_ID,
    r"\b((?:A3T[A-Z0-9]|AKIA|AGPA|AIDA|AROA|AIPA|ANPA|ANVA|ASIA)[A-Z0-9]{16})\b"
);
lazy_regex!(
    AWS_MWS_TOKEN,
    r"(?i)(amzn\.mws\.[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})"
);

// ── GitHub ──
lazy_regex!(GITHUB_PAT, r"\b(ghp_[a-zA-Z0-9]{36})\b");
lazy_regex!(GITHUB_PAT_FINE, r"\b(github_pat_[0-9a-zA-Z_]{82})\b");
lazy_regex!(GITHUB_OAUTH, r"\b(gho_[a-zA-Z0-9]{36})\b");
lazy_regex!(GITHUB_APP, r"\b((?:ghu|ghs)_[a-zA-Z0-9]{36})\b");
lazy_regex!(GITHUB_REFRESH, r"\b(ghr_[a-zA-Z0-9]{76})\b");

// ── GitLab ──
lazy_regex!(GITLAB_PAT, r"\b(glpat-[0-9a-zA-Z\-_]{20})\b");
lazy_regex!(GITLAB_RUNNER, r"\b(GR1348941[0-9a-zA-Z\-_]{20})\b");
lazy_regex!(GITLAB_PIPELINE, r"\b(glptt-[0-9a-zA-Z\-_]{20})\b");

// ── Slack ──
lazy_regex!(
    SLACK_TOKEN,
    r"xox[baprs]-[0-9a-zA-Z]{12,}-[0-9a-zA-Z]{12,}(?:-[0-9a-zA-Z]{12,})*"
);
lazy_regex!(
    SLACK_WEBHOOK,
    r"(?i)(https://hooks\.slack\.com/services/T[a-z0-9_]{8,12}/B[a-z0-9_]{8,12}/[a-z0-9_]{24})"
);

// ── Discord ──
lazy_regex!(
    DISCORD_WEBHOOK,
    r"(?i)(https://discord(?:app)?\.com/api/webhooks/[0-9]+/[a-zA-Z0-9_-]+)"
);
lazy_regex!(
    DISCORD_BOT_TOKEN,
    r"(?i)((?:mfa\.)?[a-z0-9_-]{20,}\.[\w-]{6}\.[\w-]{27})"
);

// ── Google ──
lazy_regex!(GOOGLE_API_KEY, r"\b(AIza[0-9A-Za-z_-]{35})\b");
lazy_regex!(GOOGLE_OAUTH, r"\b(ya29\.[0-9A-Za-z_-]{20,100})\b");
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

// ── Stripe ──
lazy_regex!(STRIPE_LIVE, r"\b((?:sk|rk)_live_[0-9a-zA-Z]{23,})\b");
lazy_regex!(STRIPE_TEST, r"\b((?:sk|rk)_test_[0-9a-zA-Z]{23,})\b");
lazy_regex!(STRIPE_PUBLISHABLE, r"\b(pk_(?:live|test)_[0-9a-zA-Z]{23,})\b");

// ── Heroku ──
lazy_regex!(
    HEROKU_API_KEY,
    r"(?i)heroku.{0,20}key.{0,20}\b([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\b"
);

// ── Twilio ──
lazy_regex!(TWILIO_ACCOUNT_SID, r"\b(AC[0-9a-fA-F]{32})\b");
lazy_regex!(
    TWILIO_AUTH_TOKEN,
    r"(?i)twilio.{0,20}(?:token|secret|auth).{0,20}\b([0-9a-f]{32})\b"
);

// ── SendGrid ──
lazy_regex!(SENDGRID_API_KEY, r"\b(SG\.[0-9A-Za-z_-]{22}\.[0-9A-Za-z_-]{43})\b");

// ── Mailchimp ──
lazy_regex!(MAILCHIMP_API_KEY, r"\b([0-9a-f]{32}-us[0-9]{1,2})\b");

// ── Mailgun ──
lazy_regex!(
    MAILGUN_API_KEY,
    r"(?i)(?:mailgun|mg).{0,20}key.{0,20}\b(key-[0-9a-zA-Z]{32})\b"
);

// ── Firebase ──
lazy_regex!(FIREBASE_URL, r"(?i)(https://[a-z0-9-]+\.firebaseio\.com)");
lazy_regex!(FIREBASE_API_KEY, r"\b(AIza[0-9A-Za-z_-]{35})\b");

// ── Shopify ──
lazy_regex!(SHOPIFY_SECRET, r"\b(shpss_[a-fA-F0-9]{32})\b");
lazy_regex!(SHOPIFY_ACCESS, r"\b(shpat_[a-fA-F0-9]{32})\b");
lazy_regex!(SHOPIFY_CUSTOM, r"\b(shpca_[a-fA-F0-9]{32})\b");
lazy_regex!(SHOPIFY_PRIVATE, r"\b(shppa_[a-fA-F0-9]{32})\b");

// ── Microsoft Teams ──
lazy_regex!(
    TEAMS_WEBHOOK,
    r"(?i)(https://[a-z0-9]+\.webhook\.office\.com/webhookb2/[a-z0-9-]+/IncomingWebhook/[a-z0-9]+/[a-z0-9-]+)"
);

// ── Dynatrace ──
lazy_regex!(
    DYNATRACE_TOKEN,
    r"\b(dt0[a-zA-Z]{1}[0-9]{2}\.[A-Z0-9]{24}\.[A-Z0-9]{64})\b"
);

// ── Linear ──
lazy_regex!(LINEAR_API_KEY, r"\b(lin_api_[a-zA-Z0-9]{40})\b");

// ── Algolia ──
lazy_regex!(ALGOLIA_API_KEY, r"(?i)algolia.{0,20}key.{0,20}\b([a-f0-9]{32})\b");

// ── Netlify ──
lazy_regex!(NETLIFY_TOKEN, r"(?i)(nfp_[a-z0-9]{40})");

// ── CircleCI ──
lazy_regex!(CIRCLECI_TOKEN, r"(?i)(circle-token-[a-z0-9]{40})");

// ── Snyk ──
lazy_regex!(
    SNYK_TOKEN,
    r"\b([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\b"
);

// ── Mapbox ──
lazy_regex!(
    MAPBOX_SECRET,
    r"(?i)mapbox.{0,30}(sk\.[a-z0-9\-+/=]{32,128}\.[a-z0-9\-+/=]{20,30})"
);
lazy_regex!(MAPBOX_PUBLIC, r"\b(pk\.[a-zA-Z0-9]{60,})\b");

// ── OpenAI / Anthropic / AI ──
lazy_regex!(OPENAI_API_KEY, r"\b(sk-[a-zA-Z0-9]{20,}T3BlbkFJ[a-zA-Z0-9]{20,})\b");
lazy_regex!(OPENAI_KEY_NEW, r"\b(sk-proj-[a-zA-Z0-9_-]{40,})\b");
lazy_regex!(ANTHROPIC_API_KEY, r"\b(sk-ant-[a-zA-Z0-9_-]{40,})\b");

// ── Generic / Broad ──
lazy_regex!(JWT, r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+");
lazy_regex!(
    PRIVATE_KEY,
    r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY(?:\sBLOCK)?-----"
);
lazy_regex!(
    BEARER_TOKEN,
    r#"(?i)(?:bearer|token)\s*[:=]\s*["']([a-zA-Z0-9_\-\.]{20,})["']"#
);
lazy_regex!(BASIC_AUTH_URL, r"(?i)(https?://[^:]+:[^@\s]{3,}@[^\s]+)");
lazy_regex!(
    INTERNAL_IP,
    r"\b((?:10|127)\.\d{1,3}\.\d{1,3}\.\d{1,3}|(?:172\.(?:1[6-9]|2[0-9]|3[01]))\.\d{1,3}\.\d{1,3}|(?:192\.168)\.\d{1,3}\.\d{1,3})\b"
);
lazy_regex!(
    S3_BUCKET,
    r#"(?i)((?:s3://|https?://[a-z0-9.-]+\.s3[a-z0-9.-]*\.amazonaws\.com)[^\s"'<>]+)"#
);
lazy_regex!(GCP_SERVICE_ACCOUNT, r#""type"\s*:\s*"service_account""#);

// ── PayPal / Square ──
lazy_regex!(
    PAYPAL_BRAINTREE,
    r"(?i)(access_token\$production\$[0-9a-z]{16}\$[0-9a-f]{32})"
);
lazy_regex!(SQUARE_ACCESS, r"\b(sq0atp-[0-9A-Za-z_-]{22})\b");
lazy_regex!(SQUARE_OAUTH, r"\b(sq0csp-[0-9A-Za-z_-]{43})\b");

// ── NPM ──
lazy_regex!(NPM_TOKEN, r"\b(npm_[a-zA-Z0-9]{36})\b");

// ── PyPI ──
lazy_regex!(PYPI_TOKEN, r"\b(pypi-[a-zA-Z0-9_-]{50,})\b");

// ── DigitalOcean ──
lazy_regex!(DIGITALOCEAN_TOKEN, r"\b(dop_v1_[a-f0-9]{64})\b");
lazy_regex!(DIGITALOCEAN_OAUTH, r"\b(doo_v1_[a-f0-9]{64})\b");
lazy_regex!(DIGITALOCEAN_REFRESH, r"\b(dor_v1_[a-f0-9]{64})\b");

// ── Vault / HashiCorp ──
lazy_regex!(VAULT_TOKEN, r"\b(hvs\.[a-zA-Z0-9_-]{24,})\b");

// ── Supabase ──
lazy_regex!(SUPABASE_KEY, r"\b(sbp_[a-f0-9]{40})\b");

// ══════════════════════════════════════════════════════════════════════════════
// PROPERTY-NAME PATTERNS (case-insensitive identifier matchers)
// ══════════════════════════════════════════════════════════════════════════════
lazy_regex!(P_SECRET, r"(?i)secret");
lazy_regex!(P_PASSWD, r"(?i)passw");
lazy_regex!(P_AUTHORIZATION, r"(?i)authorization");
lazy_regex!(P_AWS_ACCESS_KEY_ID, r"(?i)aws[_-]?access[_-]?key[_-]?id");
lazy_regex!(P_AWS_SECRET_ACCESS_KEY, r"(?i)aws[_-]?secret[_-]?access[_-]?key");
lazy_regex!(P_ACCESS, r"(?i)access[_-]?(?:key|token|secret)");
lazy_regex!(P_AUTH, r"(?i)auth[_-]?(?:key|token|secret)");
lazy_regex!(P_SESSION, r"(?i)session[_-]?(?:key|token|secret)");
lazy_regex!(P_CONSUMER, r"(?i)consumer[_-]?(?:key|token|secret)");
lazy_regex!(P_PUBLIC, r"(?i)public[_-]?(?:key|token|secret)");
lazy_regex!(P_CLIENT, r"(?i)client[_-]?(?:id|token|key|secret)");
lazy_regex!(P_SLACK_BOT, r"(?i)slack[_-]?bot[_-]?token");
lazy_regex!(P_SLACK, r"(?i)slack[_-]?token");
lazy_regex!(P_ID_DSA, r"(?i)id[_-]?(?:dsa|rsa|ecdsa|ed25519)");
lazy_regex!(P_SSH, r"(?i)ssh[_-]?(?:key|priv)");
lazy_regex!(P_ENCRYPT, r"(?i)encrypt[_-]?(?:secret|key)");
lazy_regex!(P_DECRYPT, r"(?i)decrypt[_-]?(?:secret|key)");
lazy_regex!(P_GITHUB, r"(?i)github[_-]?(?:key|token|secret)");
lazy_regex!(P_IRC_PASS, r"(?i)irc[_-]?pass");
lazy_regex!(P_API, r"(?i)api[_-]?(?:key|token|secret)");
lazy_regex!(P_PRIVATE_KEY, r"(?i)private[_-]?key");
lazy_regex!(
    P_DB_PASSWORD,
    r"(?i)(?:db|database|mysql|postgres|redis)[_-]?pass(?:word)?"
);

/// All built-in rules.
pub fn all_rules() -> Vec<SecretRule> {
    let mut rules = Vec::new();
    // Literal rules
    macro_rules! lit {
        ($kind:expr, $regex:expr, $sev:expr) => {
            rules.push(SecretRule {
                kind: $kind,
                pattern: $regex.as_str(),
                regex: &$regex,
                is_property_name: false,
                severity: $sev,
            });
        };
    }
    lit!("aws_key_id", AWS_KEY_ID, Severity::Critical);
    lit!("aws_mws_token", AWS_MWS_TOKEN, Severity::Critical);
    lit!("github_pat", GITHUB_PAT, Severity::Critical);
    lit!("github_pat_fine", GITHUB_PAT_FINE, Severity::Critical);
    lit!("github_oauth", GITHUB_OAUTH, Severity::High);
    lit!("github_app", GITHUB_APP, Severity::High);
    lit!("github_refresh", GITHUB_REFRESH, Severity::High);
    lit!("gitlab_pat", GITLAB_PAT, Severity::Critical);
    lit!("gitlab_runner", GITLAB_RUNNER, Severity::High);
    lit!("gitlab_pipeline", GITLAB_PIPELINE, Severity::Medium);
    lit!("slack_token", SLACK_TOKEN, Severity::High);
    lit!("slack_webhook", SLACK_WEBHOOK, Severity::Medium);
    lit!("discord_webhook", DISCORD_WEBHOOK, Severity::Medium);
    lit!("discord_bot_token", DISCORD_BOT_TOKEN, Severity::High);
    lit!("google_api_key", GOOGLE_API_KEY, Severity::Medium);
    lit!("google_oauth", GOOGLE_OAUTH, Severity::High);
    lit!("google_calendar_uri", GOOGLE_CALENDAR_URI, Severity::Low);
    lit!("google_forms", GOOGLE_FORMS, Severity::Info);
    lit!("google_docs", GOOGLE_DOCS, Severity::Info);
    lit!("stripe_live_key", STRIPE_LIVE, Severity::Critical);
    lit!("stripe_test_key", STRIPE_TEST, Severity::Low);
    lit!("stripe_publishable", STRIPE_PUBLISHABLE, Severity::Info);
    lit!("heroku_api_key", HEROKU_API_KEY, Severity::High);
    lit!("twilio_account_sid", TWILIO_ACCOUNT_SID, Severity::Medium);
    lit!("twilio_auth_token", TWILIO_AUTH_TOKEN, Severity::Critical);
    lit!("sendgrid_api_key", SENDGRID_API_KEY, Severity::Critical);
    lit!("mailchimp_api_key", MAILCHIMP_API_KEY, Severity::High);
    lit!("mailgun_api_key", MAILGUN_API_KEY, Severity::High);
    lit!("firebase_url", FIREBASE_URL, Severity::Medium);
    lit!("shopify_secret", SHOPIFY_SECRET, Severity::Critical);
    lit!("shopify_access", SHOPIFY_ACCESS, Severity::High);
    lit!("shopify_custom", SHOPIFY_CUSTOM, Severity::High);
    lit!("shopify_private", SHOPIFY_PRIVATE, Severity::High);
    lit!("teams_webhook", TEAMS_WEBHOOK, Severity::Medium);
    lit!("dynatrace_token", DYNATRACE_TOKEN, Severity::High);
    lit!("linear_api_key", LINEAR_API_KEY, Severity::Medium);
    lit!("algolia_api_key", ALGOLIA_API_KEY, Severity::Medium);
    lit!("netlify_token", NETLIFY_TOKEN, Severity::High);
    lit!("circleci_token", CIRCLECI_TOKEN, Severity::High);
    lit!("mapbox_secret", MAPBOX_SECRET, Severity::High);
    lit!("mapbox_public", MAPBOX_PUBLIC, Severity::Info);
    lit!("openai_api_key", OPENAI_API_KEY, Severity::Critical);
    lit!("openai_key_new", OPENAI_KEY_NEW, Severity::Critical);
    lit!("anthropic_api_key", ANTHROPIC_API_KEY, Severity::Critical);
    lit!("jwt", JWT, Severity::Medium);
    lit!("private_key", PRIVATE_KEY, Severity::Critical);
    lit!("bearer_token", BEARER_TOKEN, Severity::High);
    lit!("basic_auth_url", BASIC_AUTH_URL, Severity::High);
    lit!("internal_ip", INTERNAL_IP, Severity::Low);
    lit!("s3_bucket", S3_BUCKET, Severity::Medium);
    lit!("gcp_service_account", GCP_SERVICE_ACCOUNT, Severity::Critical);
    lit!("paypal_braintree", PAYPAL_BRAINTREE, Severity::Critical);
    lit!("square_access", SQUARE_ACCESS, Severity::High);
    lit!("square_oauth", SQUARE_OAUTH, Severity::High);
    lit!("npm_token", NPM_TOKEN, Severity::High);
    lit!("pypi_token", PYPI_TOKEN, Severity::High);
    lit!("digitalocean_token", DIGITALOCEAN_TOKEN, Severity::Critical);
    lit!("digitalocean_oauth", DIGITALOCEAN_OAUTH, Severity::High);
    lit!("digitalocean_refresh", DIGITALOCEAN_REFRESH, Severity::High);
    lit!("vault_token", VAULT_TOKEN, Severity::Critical);
    lit!("supabase_key", SUPABASE_KEY, Severity::High);

    // Property-name rules
    macro_rules! prop {
        ($kind:expr, $regex:expr) => {
            rules.push(SecretRule {
                kind: $kind,
                pattern: $regex.as_str(),
                regex: &$regex,
                is_property_name: true,
                severity: Severity::Medium,
            });
        };
    }
    prop!("sensitive_property_name:secret", P_SECRET);
    prop!("sensitive_property_name:passwd", P_PASSWD);
    prop!("sensitive_property_name:authorization", P_AUTHORIZATION);
    prop!("sensitive_property_name:aws_access_key_id", P_AWS_ACCESS_KEY_ID);
    prop!("sensitive_property_name:aws_secret_access_key", P_AWS_SECRET_ACCESS_KEY);
    prop!("sensitive_property_name:access", P_ACCESS);
    prop!("sensitive_property_name:auth", P_AUTH);
    prop!("sensitive_property_name:session", P_SESSION);
    prop!("sensitive_property_name:consumer", P_CONSUMER);
    prop!("sensitive_property_name:public", P_PUBLIC);
    prop!("sensitive_property_name:client", P_CLIENT);
    prop!("sensitive_property_name:slack_bot", P_SLACK_BOT);
    prop!("sensitive_property_name:slack", P_SLACK);
    prop!("sensitive_property_name:id_dsa", P_ID_DSA);
    prop!("sensitive_property_name:ssh", P_SSH);
    prop!("sensitive_property_name:encrypt", P_ENCRYPT);
    prop!("sensitive_property_name:decrypt", P_DECRYPT);
    prop!("sensitive_property_name:github", P_GITHUB);
    prop!("sensitive_property_name:irc_pass", P_IRC_PASS);
    prop!("sensitive_property_name:api", P_API);
    prop!("sensitive_property_name:private_key", P_PRIVATE_KEY);
    prop!("sensitive_property_name:db_password", P_DB_PASSWORD);

    rules
}

/// Only the literal-value rules (non-property patterns).
pub fn literal_rules() -> Vec<SecretRule> {
    all_rules().into_iter().filter(|r| !r.is_property_name).collect()
}

/// Only the property-name rules.
pub fn property_name_rules() -> Vec<SecretRule> {
    all_rules().into_iter().filter(|r| r.is_property_name).collect()
}

/// Get severity for a given match_kind (falls back to Info for unknown kinds).
pub fn severity_for_kind(kind: &str) -> Severity {
    // Check literal rules first
    for rule in all_rules() {
        if rule.kind == kind {
            return rule.severity;
        }
    }
    // Analyzer-specific defaults
    match kind {
        "html_manipulation" | "dangerously_set_inner_html" => Severity::High,
        // Taint-flow + entropy findings (severity usually set explicitly).
        "dom_xss" => Severity::High,
        "open_redirect" => Severity::Medium,
        "high_entropy_secret" => Severity::Medium,
        "location_assignment" => Severity::Medium,
        "window_onmessage" | "post_message" => Severity::Medium,
        "url_search_params" | "storage_access" => Severity::Low,
        "path" | "hostname" | "url" | "comment_path" | "comment_hostname" => Severity::Info,
        "graphql" => Severity::Low,
        "npm_package" => Severity::Info,
        "package_json" => Severity::Medium,
        "package_json_dep" => Severity::Info,
        "api_call" => Severity::Low,
        _ if kind.starts_with("sensitive_property_name:") => Severity::Medium,
        _ if kind.starts_with("custom:") => Severity::Medium,
        _ => Severity::Info,
    }
}
