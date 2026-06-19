use rustc_hash::FxHashMap;

use crate::Result;

use super::SourcemapParser;
use super::vlq::decode_vlq;

#[derive(Debug, Clone)]
pub(super) struct MappingEntry {
    pub(super) gen_col: u32,
    pub(super) source_idx: usize,
    pub(super) orig_line: u32,
    pub(super) orig_col: u32,
}

impl SourcemapParser {
    pub(super) fn build_mapping_index(
        &self,
        mappings: &str,
        source_count: usize,
    ) -> Result<FxHashMap<u32, Vec<MappingEntry>>> {
        let mut index: FxHashMap<u32, Vec<MappingEntry>> = FxHashMap::default();
        let mut gen_line = 1u32;
        let mut gen_col: i64;
        let mut source_idx = 0i64;
        let mut orig_line = 0i64;
        let mut orig_col = 0i64;

        for line in mappings.split(';') {
            gen_col = 0;

            if line.is_empty() {
                gen_line += 1;
                continue;
            }

            for segment in line.split(',') {
                if segment.is_empty() {
                    continue;
                }

                let values = decode_vlq(segment)?;
                if values.is_empty() {
                    continue;
                }

                gen_col += values[0];

                if values.len() >= 4 {
                    source_idx += values[1];
                    orig_line += values[2];
                    orig_col += values[3];

                    if source_idx >= 0 && (source_idx as usize) < source_count {
                        index.entry(gen_line).or_default().push(MappingEntry {
                            gen_col: gen_col as u32,
                            source_idx: source_idx as usize,
                            orig_line: (orig_line + 1) as u32,
                            orig_col: orig_col as u32,
                        });
                    }
                }
            }

            if let Some(entries) = index.get_mut(&gen_line) {
                entries.sort_by_key(|e| e.gen_col);
            }

            gen_line += 1;
        }

        Ok(index)
    }

    pub(super) fn lookup_original_position(
        &self,
        index: &FxHashMap<u32, Vec<MappingEntry>>,
        line: u32,
        col: u32,
    ) -> Option<MappingEntry> {
        let entries = index.get(&line)?;
        let pos = entries.partition_point(|e| e.gen_col <= col);
        if pos == 0 {
            return None;
        }
        Some(entries[pos - 1].clone())
    }

    pub(super) fn is_identifier(&self, word: &str) -> bool {
        word.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
    }

    pub(super) fn get_identifier_at(&self, source: &str, line: u32, column: u32) -> Option<String> {
        let target_line = source.lines().nth((line.saturating_sub(1)) as usize)?;
        let rest = target_line.get((column as usize)..)?;
        self.identifier_pattern.find(rest).map(|m| m.as_str().to_string())
    }
}
