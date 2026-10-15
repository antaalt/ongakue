//! Shared links: a visual's name and code, encoded into the part of the URL
//! after `#`, which browsers never send to the server.
//!
//! Format: `share=1.<name>.<code>`, where `1` is the format version, the name
//! is URL-safe base64 of its UTF-8 text, and the code is the same of its
//! deflate-compressed text.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// What precedes the encoded visual in the URL fragment.
pub const PREFIX: &str = "share=";
const VERSION: &str = "1";
/// Largest code a link may unpack to: far more than any shader, but stops a
/// crafted link from unpacking to gigabytes.
const MAX_SOURCE: usize = 1 << 20;

#[derive(Debug, PartialEq)]
pub enum ShareError {
    /// Made by a newer version of the app.
    UnknownVersion,
    /// Truncated or altered.
    Broken,
}

impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            ShareError::UnknownVersion => "This shared link comes from a newer version of Ongakue.",
            ShareError::Broken => "This shared link is incomplete or damaged.",
        })
    }
}

/// The URL fragment (without `#`) sharing a visual.
pub fn encode(name: &str, source: &str) -> String {
    let code = miniz_oxide::deflate::compress_to_vec(source.as_bytes(), 9);
    format!(
        "{PREFIX}{VERSION}.{}.{}",
        URL_SAFE_NO_PAD.encode(name),
        URL_SAFE_NO_PAD.encode(code)
    )
}

/// The visual shared by a URL fragment (without `#`): `None` if the fragment
/// isn't a shared link at all, and an error if it is one but can't be read.
pub fn decode(fragment: &str) -> Option<Result<(String, String), ShareError>> {
    let shared = fragment.strip_prefix(PREFIX)?;
    Some(decode_shared(shared))
}

fn decode_shared(shared: &str) -> Result<(String, String), ShareError> {
    let mut parts = shared.split('.');
    let (Some(version), Some(name), Some(code), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(ShareError::Broken);
    };
    if version != VERSION {
        return Err(ShareError::UnknownVersion);
    }
    let text = |bytes: Vec<u8>| String::from_utf8(bytes).map_err(|_| ShareError::Broken);
    let base64 = |part: &str| URL_SAFE_NO_PAD.decode(part).map_err(|_| ShareError::Broken);
    let name = text(base64(name)?)?;
    let code = miniz_oxide::inflate::decompress_to_vec_with_limit(&base64(code)?, MAX_SOURCE)
        .map_err(|_| ShareError::Broken)?;
    Ok((name, text(code)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let source = include_str!("../../../render/src/shaders/radial.wgsl");
        let fragment = encode("my glow ✨", source);
        assert_eq!(
            decode(&fragment),
            Some(Ok(("my glow ✨".to_owned(), source.to_owned())))
        );
        // Only characters that need no escaping in a URL.
        assert!(
            fragment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "=.-_".contains(c)),
            "{fragment}"
        );
    }

    #[test]
    fn links_are_compact() {
        let source = include_str!("../../../render/src/shaders/radial.wgsl");
        let fragment = encode("radial", source);
        // Compressed, then base64 (4 characters per 3 bytes): still shorter
        // than the code itself.
        assert!(
            fragment.len() < source.len() * 3 / 4,
            "{} for {}",
            fragment.len(),
            source.len()
        );
    }

    #[test]
    fn other_fragments_are_not_shared_links() {
        assert_eq!(decode(""), None);
        assert_eq!(decode("section-2"), None);
    }

    #[test]
    fn rejects_unknown_versions_and_damage() {
        let fragment = encode("glow", "fn fs_main() {}");
        assert_eq!(
            decode(&fragment.replacen("share=1.", "share=2.", 1)),
            Some(Err(ShareError::UnknownVersion))
        );
        // Truncated, altered, or with parts missing.
        assert_eq!(
            decode(&fragment[..fragment.len() - 6]),
            Some(Err(ShareError::Broken))
        );
        assert_eq!(
            decode(&fragment.replace('.', "!")),
            Some(Err(ShareError::Broken))
        );
        assert_eq!(decode("share=1.Z2xvdw"), Some(Err(ShareError::Broken)));
        assert_eq!(decode("share=1.Z2xvdw.$$$"), Some(Err(ShareError::Broken)));
    }

    #[test]
    fn rejects_links_unpacking_to_huge_code() {
        // 10 MB of spaces compresses to a few kilobytes.
        let fragment = encode("bomb", &" ".repeat(10 << 20));
        assert!(fragment.len() < 50_000, "{}", fragment.len());
        assert_eq!(decode(&fragment), Some(Err(ShareError::Broken)));
    }
}
