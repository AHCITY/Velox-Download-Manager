//! Single source of truth for the browser-extension identity (audit EXT-1).
//!
//! The extension ID is DERIVED from the `key` field of
//! `download-engine/extension/manifest.json`: SHA-256 over the decoded
//! SubjectPublicKeyInfo, first 16 bytes, each hex nibble mapped to the letters
//! a–p (Chrome's rule). The code previously pinned a different ID
//! ("hpaxoarvxcmmqijm"), so the native-messaging host manifest allowlisted an
//! origin Chrome never grants — every `connectNative()` failed with "Access to
//! the native messaging host is forbidden" and the extension could never reach
//! the app. The tests below derive the ID from the shipped manifest, so the
//! pinned value and the shipped key can never drift apart again.

use sha2::{Digest, Sha256};

/// The extension ID Chrome computes from the shipped manifest key.
pub const EXTENSION_ID: &str = "ffnpjmbhfmaamhlbohbmamnmoaikpdmc";

/// Map the first 32 hex chars of a digest to Chrome's a–p alphabet
/// (0..9 → a..j, a..f → k..p).
pub fn id_from_key_hex(hex: &str) -> String {
    hex.chars()
        .take(32)
        .map(|c| match c {
            '0'..='9' => (b'a' + (c as u8 - b'0')) as char,
            'a'..='f' => (b'k' + (c as u8 - b'a')) as char,
            _ => unreachable!("digest hex is lowercase"),
        })
        .collect()
}

/// Chrome's rule applied to a raw SPKI DER blob.
pub fn id_from_spki(spki_der: &[u8]) -> String {
    let hex: String = Sha256::digest(spki_der)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    id_from_key_hex(&hex)
}

#[cfg(test)]
mod tests {
    use super::{id_from_key_hex, id_from_spki, EXTENSION_ID};
    use std::path::PathBuf;

    /// The `key` value of download-engine/extension/manifest.json.
    fn shipped_manifest_key() -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extension/manifest.json");
        let s = std::fs::read_to_string(&path)
            .expect("extension/manifest.json must exist beside the workspace");
        let start = s
            .find("\"key\"")
            .expect("manifest.json must carry a key field");
        let rest = &s[start..];
        let colon = rest.find(':').expect("key field followed by a colon");
        let after = &rest[colon..];
        let q1 = after.find('"').expect("quoted key value") + 1;
        let tail = &after[q1..];
        let q2 = tail.find('"').expect("closing quote of the key value");
        tail[..q2].to_string()
    }

    /// Minimal standard-alphabet base64 decoder (no external dep for a test).
    fn b64_decode(s: &str) -> Vec<u8> {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let val = |c: u8| T.iter().position(|&t| t == c).unwrap() as u32;
        let bytes: Vec<u8> = s.bytes().filter(|&c| c != b'=' && c != b'\n').collect();
        bytes
            .chunks(4)
            .flat_map(|c| {
                let n = c.iter().fold(0u32, |a, &b| (a << 6) | val(b));
                match c.len() {
                    4 => vec![(n >> 16) as u8, (n >> 8) as u8, n as u8],
                    3 => vec![(n >> 10) as u8, (n >> 2) as u8],
                    2 => vec![(n >> 4) as u8],
                    _ => vec![],
                }
            })
            .collect()
    }

    #[test]
    fn hex_nibbles_map_to_the_ap_alphabet() {
        let input = "0123456789abcdef".repeat(2);
        assert_eq!(id_from_key_hex(&input), "abcdefghijklmnop".repeat(2));
    }

    #[test]
    fn shipped_manifest_key_derives_the_pinned_id() {
        let key = shipped_manifest_key();
        let der = b64_decode(&key);
        assert_eq!(der.len(), 294, "RSA-2048 SPKI is 294 bytes");
        let derived = id_from_spki(&der);
        assert_eq!(derived.len(), 32);
        assert!(derived.chars().all(|c| ('a'..='p').contains(&c)));
        assert_eq!(
            derived, EXTENSION_ID,
            "manifest key and pinned ID drifted apart"
        );
        assert_ne!(derived, "hpaxoarvxcmmqijm", "the old, wrong ID came back");
    }

    #[test]
    fn id_is_wellformed() {
        assert_eq!(EXTENSION_ID.len(), 32);
        assert!(EXTENSION_ID.chars().all(|c| ('a'..='p').contains(&c)));
    }
}
