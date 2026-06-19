use crate::{BeautifyError, Result};

pub(super) fn decode_vlq(input: &str) -> Result<Vec<i64>> {
    const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut values = Vec::new();
    let mut shift = 0u32;
    let mut value = 0i64;

    for ch in input.chars() {
        let digit = BASE64_CHARS
            .iter()
            .zip(0_i64..)
            .find_map(|(&c, idx)| (c == ch as u8).then_some(idx))
            .ok_or_else(|| BeautifyError::InvalidInput(format!("Invalid VLQ character: {ch}")))?;
        let continuation = (digit & 32) != 0;
        let digit_value = digit & 31;

        value += digit_value << shift;

        if continuation {
            shift += 5;
        } else {
            let is_negative = (value & 1) != 0;
            value >>= 1;
            if is_negative {
                value = -value;
            }
            values.push(value);
            value = 0;
            shift = 0;
        }
    }

    Ok(values)
}
