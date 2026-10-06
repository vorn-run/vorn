//! Device tokens: the credential a client on another machine presents.
//!
//! Opaque and hashed at rest: `vorn_<id>_<secret>`, where the id is a UUID
//! sent in the clear so checking one is a primary-key lookup, and only the
//! SHA-256 of the secret is stored. Revoking is one row update.

use sha2::{Digest, Sha256};

const PREFIX: &str = "vorn_";
const SECRET_BYTES: usize = 32;

/// A token's two halves.
#[derive(Debug, PartialEq, Eq)]
pub struct Parsed<'a> {
    pub id: &'a str,
    pub secret: &'a str,
}

/// Splits `vorn_<id>_<secret>` on the first `_` after the id: the secret is
/// base64url, whose alphabet includes `_`, and a UUID never does.
pub fn parse(raw: &str) -> Option<Parsed<'_>> {
    let rest = raw.strip_prefix(PREFIX)?;
    let sep = rest.find('_').filter(|&i| i > 0)?;
    let secret = &rest[sep + 1..];
    if secret.is_empty() {
        return None;
    }
    Some(Parsed {
        id: &rest[..sep],
        secret,
    })
}

/// The SHA-256 of a secret, as the bytes compared.
pub fn digest(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

/// The stored form of a secret's hash: lowercase hex.
pub fn hash_hex(secret: &str) -> String {
    data_encoding::HEXLOWER.encode(&digest(secret))
}

/// Whether `secret` hashes to `stored_hex`, compared in constant time. A
/// stored hash that is not 32 bytes of hex never matches.
pub fn secret_matches(secret: &str, stored_hex: &str) -> bool {
    let Ok(stored) = data_encoding::HEXLOWER_PERMISSIVE.decode(stored_hex.as_bytes()) else {
        return false;
    };
    constant_time_eq(&digest(secret), &stored)
}

/// Compares two secrets without leaking where they differ through timing.
/// Different lengths are simply unequal.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A token just made: the plaintext exists only here, and only its hash is
/// stored.
#[derive(Debug)]
pub struct Minted {
    pub id: String,
    pub hash_hex: String,
    pub plaintext: String,
}

/// Makes a token: a random UUID and 32 random bytes.
pub fn mint() -> Result<Minted, getrandom::Error> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes)?;
    let secret = data_encoding::BASE64URL_NOPAD.encode(&bytes);
    let id = uuid::Uuid::new_v4().to_string();
    Ok(Minted {
        hash_hex: hash_hex(&secret),
        plaintext: format!("{PREFIX}{id}_{secret}"),
        id,
    })
}

/// `Authorization: Bearer <token>` to the token.
pub fn bearer_from(authorization: &str) -> Option<&str> {
    let value = authorization.strip_prefix("Bearer ")?.trim();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_the_first_separator_after_the_id() {
        assert_eq!(
            parse("vorn_1234_ab_cd"),
            Some(Parsed {
                id: "1234",
                secret: "ab_cd"
            })
        );
        assert_eq!(parse("vorn__x"), None);
        assert_eq!(parse("vorn_id_"), None);
        assert_eq!(parse("vorn_id"), None);
        assert_eq!(parse("nope_id_x"), None);
    }

    #[test]
    fn a_minted_token_verifies_and_a_changed_one_does_not() {
        let minted = mint().unwrap();
        let parsed = parse(&minted.plaintext).unwrap();
        assert_eq!(parsed.id, minted.id);
        assert_eq!(parsed.secret.len(), 43);
        assert!(secret_matches(parsed.secret, &minted.hash_hex));
        assert!(!secret_matches("other", &minted.hash_hex));
        assert!(!secret_matches(parsed.secret, "abcd"));
        assert!(!secret_matches(parsed.secret, "not hex"));
    }

    #[test]
    fn hashes_as_the_server_does() {
        // sha256("secret") in hex.
        assert_eq!(
            hash_hex("secret"),
            "2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b"
        );
    }

    #[test]
    fn reads_a_bearer_credential() {
        assert_eq!(bearer_from("Bearer abc"), Some("abc"));
        assert_eq!(bearer_from("Bearer  abc "), Some("abc"));
        assert_eq!(bearer_from("Bearer "), None);
        assert_eq!(bearer_from("Basic abc"), None);
        assert_eq!(bearer_from("bearer abc"), None);
    }
}
