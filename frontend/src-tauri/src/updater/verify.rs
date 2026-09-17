//! specs/0058 — "are these the bytes we verified?", as a pure function.
//!
//! `Update::download` checks the minisign signature over the buffer it holds in memory;
//! everything after that (writing the tarball to the staging dir, reading it back at
//! install time, possibly days later) happens outside that guarantee. Hashing the
//! verified buffer once and re-hashing the file before handing it to `Update::install`
//! closes the gap without another network round trip.

use sha2::{Digest, Sha256};

/// SHA-256 of `bytes`.
pub fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Do these bytes still hash to the digest recorded when they were signature-verified?
pub fn matches(bytes: &[u8], expected: &[u8; 32]) -> bool {
    digest(bytes) == *expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_bytes_pass_and_a_single_flipped_byte_fails() {
        let payload = b"nixon-update-tarball".to_vec();
        let expected = digest(&payload);
        assert!(matches(&payload, &expected));

        let mut tampered = payload.clone();
        tampered[3] ^= 0x01;
        assert!(
            !matches(&tampered, &expected),
            "one flipped byte must not verify"
        );
        // Truncation is the other realistic corruption (a half-written staging file).
        assert!(!matches(&payload[..payload.len() - 1], &expected));
    }

    #[test]
    fn digest_is_the_known_sha256_of_the_empty_input() {
        let hex: String = digest(b"").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
