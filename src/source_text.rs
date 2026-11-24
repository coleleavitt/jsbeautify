//! Reading JavaScript source files as text.
//!
//! JavaScript has no encoding marker of its own. Most files are UTF-8, but
//! some bundlers emit Latin-1: Bun marks the modules of a standalone
//! executable as `latin1` when every character fits in one byte. A file that
//! is not valid UTF-8 is read as Latin-1 (each byte is the code point of the
//! same value, which is how such a runtime reads it), with a warning that
//! names the file and the first byte that is not UTF-8.

use std::io;
use std::path::Path;

/// Text decoded from a file's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    /// The offset of the first byte that is not UTF-8, when the bytes were
    /// read as Latin-1.
    pub latin1_from: Option<usize>,
}

/// `bytes` as UTF-8, or as Latin-1 when they are not valid UTF-8.
#[must_use]
pub fn decode(bytes: Vec<u8>) -> Decoded {
    match String::from_utf8(bytes) {
        Ok(text) => Decoded {
            text,
            latin1_from: None,
        },
        Err(error) => {
            let at = error.utf8_error().valid_up_to();
            Decoded {
                text: error.into_bytes().into_iter().map(char::from).collect(),
                latin1_from: Some(at),
            }
        }
    }
}

/// [`decode`], warning on stderr when `name` was read as Latin-1.
#[must_use]
pub fn decode_named(name: &str, bytes: Vec<u8>) -> String {
    let compressed = compression(&bytes);
    let decoded = decode(bytes);
    if let Some(format) = compressed {
        eprintln!("warning: {name}: {format}-compressed, not JavaScript text; decompress it first");
    } else if let Some(at) = decoded.latin1_from {
        eprintln!("warning: {name}: not valid UTF-8 at byte {at}; reading it as Latin-1");
    }
    decoded.text
}

/// The compression format whose magic number `bytes` start with.
fn compression(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        Some("zstd")
    } else if bytes.starts_with(&[0x1f, 0x8b]) {
        Some("gzip")
    } else {
        None
    }
}

/// Read the file at `path` with [`decode_named`].
///
/// # Errors
/// Returns the error of reading the file, with its path.
pub fn read(path: &Path) -> io::Result<String> {
    let bytes =
        std::fs::read(path).map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    Ok(decode_named(&path.display().to_string(), bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_is_kept() {
        let decoded = decode("caf\u{e9} \u{2192}".as_bytes().to_vec());
        assert_eq!(
            decoded,
            Decoded {
                text: "caf\u{e9} \u{2192}".to_string(),
                latin1_from: None
            }
        );
    }

    #[test]
    fn latin1_bytes_become_their_code_points() {
        let decoded = decode(b"var s=\"caf\xe9\";".to_vec());
        assert_eq!(decoded.text, "var s=\"caf\u{e9}\";");
        assert_eq!(decoded.latin1_from, Some(10));
    }

    #[test]
    fn compressed_files_are_recognized() {
        assert_eq!(compression(&[0x28, 0xb5, 0x2f, 0xfd, 0]), Some("zstd"));
        assert_eq!(compression(&[0x1f, 0x8b, 8]), Some("gzip"));
        assert_eq!(compression(b"var a"), None);
    }

    #[test]
    fn every_byte_round_trips() {
        let bytes: Vec<u8> = (0..=255).collect();
        let decoded = decode(bytes.clone());
        let back: Vec<u8> = decoded
            .text
            .chars()
            .map(|c| u8::try_from(u32::from(c)).expect("latin-1"))
            .collect();
        assert_eq!(back, bytes);
    }
}
