//! Decoding a file preview without lying about it.
//!
//! The first version used \`String::from_utf8_lossy\` and rejected anything with a NUL byte. That
//! is wrong twice on a Chinese Windows machine: a GBK text file has no NUL and is not UTF-8, so
//! it "succeeds" and comes back as a wall of replacement characters — the preview looks
//! corrupted rather than unsupported, and the user cannot tell which. And a UTF-16 file has NULs
//! everywhere, so good text was rejected as binary.
//!
//! The order below is deliberate: BOM first (a BOM is a declaration, not a guess), then strict
//! UTF-8, then GB18030 (a superset of GBK — the default on this platform), and only then give up.

/// What a caller needs to know about a decoded preview.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decoded {
    pub text: String,
    /// `utf-8`, `utf-16le`, `utf-16be`, `gb18030` — so a UI can say which.
    pub encoding: &'static str,
    /// True when bytes had to be replaced. The preview is readable, but not faithful.
    pub lossy: bool,
}

/// Decode bytes that are believed to be text.
///
/// Returns `None` when the bytes are not text in any encoding we recognise — the caller turns
/// that into "this is a binary file", which is the honest answer.
pub fn decode(bytes: &[u8]) -> Option<Decoded> {
    // 1. A byte-order mark is an explicit declaration; trust it before guessing.
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some(Decoded {
            text: String::from_utf8_lossy(rest).into_owned(),
            encoding: "utf-8",
            lossy: std::str::from_utf8(rest).is_err(),
        });
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }

    // 2. Strict UTF-8. This is the common case and it is exact.
    if let Ok(text) = std::str::from_utf8(bytes) {
        // A file with no NUL and valid UTF-8 is text. Control characters beyond tab/newline
        // are the remaining signal that this is really a binary blob that happens to decode.
        if !looks_binary(bytes) {
            return Some(Decoded {
                text: text.to_owned(),
                encoding: "utf-8",
                lossy: false,
            });
        }
        return None;
    }

    // 3. GB18030 covers GBK and GB2312, which is what a .txt on a Chinese Windows usually is.
    let (text, _, had_errors) = encoding_rs::GB18030.decode(bytes);
    if !had_errors && !text.is_empty() && !looks_binary(bytes) {
        return Some(Decoded {
            text: text.into_owned(),
            encoding: "gb18030",
            lossy: false,
        });
    }

    // 4. Last resort: UTF-8 with replacement characters, but say so.
    let text = String::from_utf8_lossy(bytes);
    if looks_binary(bytes) {
        return None;
    }
    Some(Decoded {
        text: text.into_owned(),
        encoding: "utf-8",
        lossy: true,
    })
}

fn utf16(rest: &[u8], little_endian: bool) -> Option<Decoded> {
    if !rest.len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = rest
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    let text = String::from_utf16_lossy(&units);
    Some(Decoded {
        text,
        encoding: if little_endian {
            "utf-16le"
        } else {
            "utf-16be"
        },
        lossy: false,
    })
}

/// Heuristic: a high share of control bytes means this is not prose.
///
/// NUL alone is not the test — UTF-16 text is full of them — but by the time this runs the
/// UTF-16 cases are already handled.
fn looks_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(4096)];
    if sample.is_empty() {
        return false;
    }
    let control = sample
        .iter()
        .filter(|byte| {
            let value = **byte;
            value < 0x09 || (value > 0x0D && value < 0x20) || value == 0x7F
        })
        .count();
    control * 100 / sample.len() > 5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_utf8_is_exact() {
        let decoded = decode("中文 text".as_bytes()).expect("text");
        assert_eq!(decoded.encoding, "utf-8");
        assert!(!decoded.lossy);
        assert_eq!(decoded.text, "中文 text");
    }

    #[test]
    fn gbk_is_decoded_rather_than_mangled() {
        // "中文" in GBK. Lossy UTF-8 decoding produces replacement characters, which looks like
        // a corrupt file instead of a different encoding.
        let gbk = [0xD6u8, 0xD0, 0xCE, 0xC4];
        let decoded = decode(&gbk).expect("gbk text");
        assert_eq!(decoded.text, "中文", "got {:?}", decoded.text);
        assert!(!decoded.lossy);
    }

    #[test]
    fn utf16_with_a_bom_is_text_not_binary() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "hello 中文".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let decoded = decode(&bytes).expect("utf16 text");
        assert_eq!(decoded.encoding, "utf-16le");
        assert_eq!(decoded.text, "hello 中文");
    }

    #[test]
    fn random_bytes_are_reported_as_binary() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(512).collect();
        assert!(decode(&bytes).is_none());
    }

    #[test]
    fn an_utf8_bom_is_stripped() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("x".as_bytes());
        assert_eq!(decode(&bytes).expect("text").text, "x");
    }
}
