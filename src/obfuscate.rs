//! Light obfuscation for stored API keys (defeats casual Ctrl+F scanning,
//! not a security boundary). Matches the format written by earlier versions.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

const PREFIX: &str = "enc:";
const XOR_KEY: u8 = 0x5A;

pub fn obfuscate(key: &str) -> String {
    if key.is_empty() || key.starts_with(PREFIX) {
        return key.to_owned();
    }
    let xored: Vec<u8> = key.bytes().map(|b| b ^ XOR_KEY).collect();
    format!("{PREFIX}{}", STANDARD.encode(xored))
}

/// Return the plaintext key. Values without the prefix are already plaintext;
/// an undecodable value is returned unchanged rather than dropped.
pub fn deobfuscate(value: &str) -> String {
    let Some(encoded) = value.strip_prefix(PREFIX) else {
        return value.to_owned();
    };
    STANDARD
        .decode(encoded)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes.into_iter().map(|b| b ^ XOR_KEY).collect()).ok())
        .unwrap_or_else(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let encoded = obfuscate("AIzaSy-secret");
        assert!(encoded.starts_with("enc:"));
        assert_ne!(encoded, "AIzaSy-secret");
        assert_eq!(deobfuscate(&encoded), "AIzaSy-secret");
    }

    #[test]
    fn idempotent_and_plaintext_passthrough() {
        let encoded = obfuscate("key");
        assert_eq!(obfuscate(&encoded), encoded);
        assert_eq!(obfuscate(""), "");
        assert_eq!(deobfuscate("plain"), "plain");
    }

    #[test]
    fn matches_python_encoding() {
        // base64(b"abc" XOR 0x5A) as produced by the previous Python version.
        assert_eq!(obfuscate("abc"), "enc:Ozg5");
    }
}
