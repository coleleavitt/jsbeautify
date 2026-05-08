//! Output schema. Mirrors jxscout's `match` JSON shape so `--analyze --json`
//! output is drop-in compatible with `jxscout-pro-v2 -c get-matches --json`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    Js,
    Html,
    SourceMap,
    ReversedSource,
}

impl FileType {
    /// Infer from filename extension. Defaults to `Js`.
    #[must_use]
    pub fn from_path(path: &str) -> Self {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".html") || lower.ends_with(".htm") {
            Self::Html
        } else if lower.ends_with(".js.map") || lower.ends_with(".map") {
            Self::SourceMap
        } else {
            Self::Js
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub struct PositionRange {
    pub start: Position,
    pub end: Position,
}

impl PositionRange {
    #[must_use]
    pub const fn new(start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> Self {
        Self {
            start: Position {
                line: start_line,
                column: start_col,
            },
            end: Position {
                line: end_line,
                column: end_col,
            },
        }
    }

    /// Compute from a UTF-8 byte offset range against the source.
    #[must_use]
    pub fn from_byte_range(source: &str, start: usize, end: usize) -> Self {
        let (sl, sc) = byte_offset_to_line_col(source, start);
        let (el, ec) = byte_offset_to_line_col(source, end);
        Self::new(sl, sc, el, ec)
    }
}

/// Convert byte offset → (1-indexed line, 0-indexed column).
fn byte_offset_to_line_col(source: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(source.len());
    let mut line = 1u32;
    let mut col = 0u32;
    for (i, b) in source.as_bytes().iter().enumerate() {
        if i >= offset {
            break;
        }
        if *b == b'\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalyzerMatch {
    /// e.g. `"github_pat"`, `"path"`, `"hostname"`, `"location_assignment"`.
    pub match_kind: String,
    /// The actual matched text.
    pub match_value: String,
    pub position: PositionRange,
    pub file_type: FileType,
    pub file_path: Option<String>,
    /// Optional analyzer-specific metadata as a JSON object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_basic() {
        let src = "hello\nworld\nfoo";
        let r = PositionRange::from_byte_range(src, 0, 5);
        assert_eq!(r.start, Position { line: 1, column: 0 });
        assert_eq!(r.end, Position { line: 1, column: 5 });

        let r2 = PositionRange::from_byte_range(src, 6, 11);
        assert_eq!(r2.start, Position { line: 2, column: 0 });
        assert_eq!(r2.end, Position { line: 2, column: 5 });
    }

    #[test]
    fn file_type_from_path() {
        assert_eq!(FileType::from_path("a.js"), FileType::Js);
        assert_eq!(FileType::from_path("a.html"), FileType::Html);
        assert_eq!(FileType::from_path("a.htm"), FileType::Html);
        assert_eq!(FileType::from_path("a.js.map"), FileType::SourceMap);
        assert_eq!(FileType::from_path("a.weirdext"), FileType::Js);
    }

    #[test]
    fn json_round_trip() {
        let m = AnalyzerMatch {
            match_kind: "github_pat".into(),
            match_value: "ghp_AAA".into(),
            position: PositionRange::new(1, 0, 1, 7),
            file_type: FileType::Js,
            file_path: Some("/x.js".into()),
            metadata: None,
        };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains(r#""match_kind":"github_pat""#));
        let back: AnalyzerMatch = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
    }
}
