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
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    #[must_use]
    pub fn parse_label(s: &str) -> Option<Self> {
        s.parse().ok()
    }
}

impl std::str::FromStr for Severity {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "info" => Ok(Self::Info),
            "low" => Ok(Self::Low),
            "medium" | "med" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "critical" | "crit" => Ok(Self::Critical),
            _ => Err(()),
        }
    }
}
