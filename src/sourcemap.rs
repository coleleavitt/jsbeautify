use serde::{Deserialize, Serialize};

mod bundle;
mod fallback;
mod tree;

pub use bundle::reconstruct_bundle_tree;
pub use tree::{ReversedSource, normalize_source_path, reconstruct_tree, reverse};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMap {
    pub version: u32,
    pub sources: Vec<String>,
    pub names: Vec<String>,
    pub mappings: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_root: Option<String>,
    /// Embedded original sources (`sourcesContent` in spec). When present,
    /// each entry corresponds 1:1 with `sources[i]`. Used by [`reverse`].
    #[serde(default, rename = "sourcesContent", skip_serializing_if = "Option::is_none")]
    pub sources_content: Option<Vec<Option<String>>>,
}

impl SourceMap {
    #[must_use]
    pub fn new(source_file: &str) -> Self {
        Self {
            version: 3,
            sources: vec![source_file.to_string()],
            names: vec![],
            mappings: String::new(),
            file: None,
            source_root: None,
            sources_content: None,
        }
    }

    pub fn add_simple_mapping(&mut self, generated_line: usize, original_line: usize) {
        if generated_line == 0 {
            return;
        }

        let line_offset = i64::try_from(original_line).unwrap_or(i64::MAX) - 1;
        let encoded = encode_vlq(0) + &encode_vlq(0) + &encode_vlq(line_offset) + &encode_vlq(0);

        if !self.mappings.is_empty() {
            self.mappings.push(';');
        }
        self.mappings.push_str(&encoded);
    }

    /// # Errors
    /// Returns an error if the operation fails.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// # Errors
    /// Returns an error if the operation fails.
    pub fn to_data_url(&self) -> Result<String, serde_json::Error> {
        let json = self.to_json()?;
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, json);
        Ok(format!("data:application/json;base64,{encoded}"))
    }

    #[must_use]
    pub fn for_chunk(chunk_name: &str, chunk_filename: &str, line_count: usize) -> Self {
        let mut map = Self::new(chunk_name);
        map.file = Some(chunk_filename.to_string());

        for line in 1..=line_count {
            map.add_simple_mapping(line, line);
        }

        map
    }
}

const VLQ_BASE: i64 = 32;
const VLQ_BASE_MASK: i64 = VLQ_BASE - 1;
const VLQ_CONTINUATION_BIT: i64 = VLQ_BASE;

fn encode_vlq(value: i64) -> String {
    let mut vlq = if value < 0 { ((-value) << 1) | 1 } else { value << 1 };

    let mut result = String::new();
    loop {
        let mut digit = vlq & VLQ_BASE_MASK;
        vlq >>= 5;
        if vlq > 0 {
            digit |= VLQ_CONTINUATION_BIT;
        }
        result.push(encode_base64_digit(digit));
        if vlq == 0 {
            break;
        }
    }
    result
}

fn encode_base64_digit(digit: i64) -> char {
    let chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    chars.chars().nth(digit as usize).unwrap_or('A')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_sourcemap() {
        let map = SourceMap::new("input.js");
        assert_eq!(map.version, 3);
        assert_eq!(map.sources.len(), 1);
        assert_eq!(map.sources[0], "input.js");
    }

    #[test]
    fn test_add_simple_mapping() {
        let mut map = SourceMap::new("input.js");
        map.add_simple_mapping(1, 1);
        assert!(!map.mappings.is_empty());
    }

    #[test]
    fn test_to_json() {
        let map = SourceMap::new("input.js");
        let json = map.to_json().unwrap();
        assert!(json.contains("\"version\":3"));
        assert!(json.contains("input.js"));
    }

    #[test]
    fn test_for_chunk() {
        let map = SourceMap::for_chunk("LoginView", "LoginView.chunk.363842.js", 10);

        assert_eq!(map.version, 3);
        assert_eq!(map.sources.len(), 1);
        assert_eq!(map.sources[0], "LoginView");
        assert_eq!(map.file, Some("LoginView.chunk.363842.js".to_string()));
        assert!(!map.mappings.is_empty());
    }
}
