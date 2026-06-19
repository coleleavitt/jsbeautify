use crate::ast_deobfuscate::state::OffsetOperation;

use super::types::ChecksumTerm;

pub(super) fn brute_force_rotation(
    strings: &[String],
    terms: &[ChecksumTerm],
    target: f64,
    decoder_offset: i32,
    offset_op: OffsetOperation,
) -> Option<usize> {
    let len = strings.len();
    if len == 0 {
        return None;
    }

    let max_attempts = (len * 2).min(1_000_000);
    let mut rotated: Vec<String> = strings.to_vec();

    for attempt in 0..max_attempts {
        if let Some(checksum) = evaluate_checksum(&rotated, terms, decoder_offset, offset_op) {
            let matches = if target == target.floor() && checksum == checksum.floor() {
                (checksum as i64) == (target as i64)
            } else {
                (checksum - target).abs() < 0.5
            };

            if matches {
                return Some(attempt);
            }
        }

        if !rotated.is_empty() {
            let first = rotated.remove(0);
            rotated.push(first);
        }
    }

    None
}

fn evaluate_checksum(
    strings: &[String],
    terms: &[ChecksumTerm],
    decoder_offset: i32,
    offset_op: OffsetOperation,
) -> Option<f64> {
    let mut sum = 0.0f64;

    for term in terms {
        let mut product = 1.0f64;
        for (i, &idx) in term.indices.iter().enumerate() {
            let actual_index = match offset_op {
                OffsetOperation::None => idx as usize,
                OffsetOperation::Subtract => {
                    let result = idx - decoder_offset;
                    if result < 0 {
                        return None;
                    }
                    result as usize
                }
                OffsetOperation::Add => (idx + decoder_offset) as usize,
            };

            if actual_index >= strings.len() {
                return None;
            }

            let s = &strings[actual_index];
            let parsed = parse_int_js(s)?;
            let divisor = term.divisors[i];
            product *= parsed / divisor;
        }

        sum = f64::from(term.sign).mul_add(product, sum);
    }

    Some(sum)
}

/// JS `parseInt()` semantics: parse leading digits, ignore trailing non-digits.
pub(super) fn parse_int_js(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    if s.starts_with("0x") || s.starts_with("0X") {
        let hex_str = &s[2..];
        let end = hex_str.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(hex_str.len());
        if end == 0 {
            return None;
        }
        return i64::from_str_radix(&hex_str[..end], 16).ok().map(|v| v as f64);
    }

    let (sign, digits_start) = if s.starts_with('-') {
        (-1.0, 1)
    } else if s.starts_with('+') {
        (1.0, 1)
    } else {
        (1.0, 0)
    };

    let rest = &s[digits_start..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    if end == 0 {
        return None;
    }

    rest[..end].parse::<f64>().ok().map(|v| sign * v)
}

pub(super) fn apply_rotation(strings: &mut Vec<String>, count: usize) {
    for _ in 0..count {
        if let Some(first) = strings.first().cloned() {
            strings.remove(0);
            strings.push(first);
        }
    }
}
