//! Standard base64, because MCP returns an image as a base64 string and this
//! plugin declares no crates.io dependencies (see `Cargo.toml` for why).
//!
//! Encode only. Nothing here ever has to read base64 back.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode bytes as standard base64 with `=` padding (RFC 4648 §4).
pub fn encode(bytes: &[u8]) -> String {
    // 4 output characters per 3 input bytes, rounded up to the next whole group.
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        // Pack the group into the low 24 bits, left-aligned, so a short final
        // chunk leaves zeroes where the missing bytes were. Those zero bits are
        // what the `=` padding below accounts for.
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let group = (b0 << 16) | (b1 << 8) | b2;

        out.push(ALPHABET[(group >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(group >> 12) as usize & 0x3f] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(group >> 6) as usize & 0x3f] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[group as usize & 0x3f] as char);
        } else {
            out.push('=');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::encode;

    #[test]
    fn matches_rfc_4648_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn encodes_high_bytes() {
        // PNG's magic number, which is the first thing a screenshot reply
        // carries and the one input where a sign error would show up.
        assert_eq!(encode(&[0x89, b'P', b'N', b'G']), "iVBORw==");
    }
}
