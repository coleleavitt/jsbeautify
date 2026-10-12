use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use rc4::consts::{U8, U16, U32};
use rc4::{Key, KeyInit, Rc4, StreamCipher};

use crate::ast_deobfuscate::state::DecoderType;

use super::DecoderInliner;

impl DecoderInliner {
    pub(super) fn apply_decoder(&self, value: &str, decoder_type: &DecoderType) -> Option<String> {
        deob_trace!("[AST]   apply_decoder called with value: {value:?}, type: {decoder_type:?}");
        let result = match decoder_type {
            DecoderType::Simple => Some(value.to_string()),
            DecoderType::Base64 => self.decode_base64(value),
            DecoderType::Xor { key } => self.decode_xor(value, key),
            DecoderType::Rc4 { key } => self.decode_rc4(value, key),
        };
        deob_trace!("[AST]   apply_decoder result: {result:?}");
        result
    }

    fn decode_base64(&self, value: &str) -> Option<String> {
        deob_trace!("[AST]     decode_base64 input: {value:?}");
        let result = BASE64_STANDARD
            .decode(value.as_bytes())
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok());
        deob_trace!("[AST]     decode_base64 output: {result:?}");
        result
    }

    fn decode_xor(&self, value: &str, key: &[u8]) -> Option<String> {
        deob_trace!("[AST]     decode_xor input: {value:?}, key: {key:?}");
        if key.is_empty() {
            deob_trace!("[AST]     decode_xor: empty key, returning None");
            return None;
        }

        let bytes = value.as_bytes();
        let decoded: Vec<u8> = bytes
            .iter()
            .enumerate()
            .map(|(i, &byte)| byte ^ key[i % key.len()])
            .collect();

        let result = String::from_utf8(decoded).ok();
        deob_trace!("[AST]     decode_xor output: {result:?}");
        result
    }

    fn decode_rc4(&self, value: &str, key: &[u8]) -> Option<String> {
        deob_trace!("[AST]     decode_rc4 input: {:?}, key len: {}", value, key.len());
        if key.is_empty() {
            deob_trace!("[AST]     decode_rc4: empty key, returning None");
            return None;
        }

        let mut data = BASE64_STANDARD.decode(value.as_bytes()).ok()?;
        deob_trace!("[AST]     decode_rc4 base64-decoded bytes: {data:?}");

        match key.len() {
            8 => {
                deob_trace!("[AST]     decode_rc4: using 8-byte key");
                let key_arr = Key::<U8>::from_slice(key);
                let mut cipher = Rc4::<_>::new(key_arr);
                cipher.apply_keystream(&mut data);
            }
            16 => {
                deob_trace!("[AST]     decode_rc4: using 16-byte key");
                let key_arr = Key::<U16>::from_slice(key);
                let mut cipher = Rc4::<_>::new(key_arr);
                cipher.apply_keystream(&mut data);
            }
            32 => {
                deob_trace!("[AST]     decode_rc4: using 32-byte key");
                let key_arr = Key::<U32>::from_slice(key);
                let mut cipher = Rc4::<_>::new(key_arr);
                cipher.apply_keystream(&mut data);
            }
            _ => {
                deob_trace!("[AST]     decode_rc4: unsupported key length: {} bytes", key.len());
                return None;
            }
        }

        deob_trace!("[AST]     decode_rc4 output bytes: {data:?}");
        let result = String::from_utf8(data).ok();
        deob_trace!("[AST]     decode_rc4 output: {result:?}");
        result
    }
}
