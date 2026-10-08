//! Secrets an extension proves itself with: a token per running child and a
//! nonce per open page.

use data_encoding::BASE64URL_NOPAD;

/// 32 random bytes, base64url without padding.
pub fn mint() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(BASE64URL_NOPAD.encode(&bytes))
}

/// Whether `a` and `b` are equal, taking the same time wherever they differ.
pub fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mints_a_fresh_url_safe_secret_each_time() {
        let (a, b) = (mint().unwrap(), mint().unwrap());
        assert_eq!(a.len(), 43);
        assert_ne!(a, b);
        assert!(a
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
    }

    #[test]
    fn compares_whole_strings() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "ab"));
    }
}
