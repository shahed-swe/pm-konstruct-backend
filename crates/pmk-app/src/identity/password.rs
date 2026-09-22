//! Password hashing.
//!
//! New hashes are Argon2id. Existing hashes are bcrypt at cost 12 -- production
//! holds 18 of them -- and must keep working, then upgrade silently on the next
//! successful login. Nobody resets a password because of the rewrite.

use argon2::password_hash::{rand_core::OsRng, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Argon2, PasswordHash};
use pmk_domain::HashKind;

use crate::{AppError, AppResult};

/// Verification result, including whether the stored hash should be replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verification {
    pub matched: bool,
    pub needs_rehash: bool,
}

pub fn hash(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(AppError::internal)
}

/// Verifies against whichever algorithm produced the stored hash.
///
/// An unrecognised hash format is treated as a non-match rather than an error:
/// it must not become a way to log in, and it must not leak which accounts have
/// malformed credentials.
pub fn verify(password: &str, stored: &str) -> Verification {
    match HashKind::detect(stored) {
        HashKind::Argon2id => {
            let matched = PasswordHash::new(stored)
                .map(|parsed| {
                    Argon2::default()
                        .verify_password(password.as_bytes(), &parsed)
                        .is_ok()
                })
                .unwrap_or(false);
            Verification { matched, needs_rehash: false }
        }
        HashKind::Bcrypt => {
            let matched = bcrypt::verify(password, stored).unwrap_or(false);
            Verification { matched, needs_rehash: matched }
        }
        HashKind::Unknown => Verification { matched: false, needs_rehash: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argon2_round_trips() {
        let h = hash("correct horse battery staple").unwrap();
        assert!(h.starts_with("$argon2id$"));
        let v = verify("correct horse battery staple", &h);
        assert!(v.matched);
        assert!(!v.needs_rehash, "a fresh argon2 hash needs no upgrade");
    }

    #[test]
    fn argon2_rejects_the_wrong_password() {
        let h = hash("right").unwrap();
        assert!(!verify("wrong", &h).matched);
    }

    #[test]
    fn the_same_password_hashes_differently_each_time() {
        // Distinct salts; identical hashes would leak equal passwords.
        assert_ne!(hash("same").unwrap(), hash("same").unwrap());
    }

    #[test]
    fn legacy_bcrypt_verifies_and_asks_for_rehash() {
        // Cost 4 keeps the test fast; production is cost 12.
        let legacy = bcrypt::hash("buildsmart2024", 4).unwrap();
        let v = verify("buildsmart2024", &legacy);
        assert!(v.matched, "existing production passwords must keep working");
        assert!(v.needs_rehash, "bcrypt must upgrade to argon2id on login");
    }

    #[test]
    fn wrong_password_against_bcrypt_does_not_request_rehash() {
        let legacy = bcrypt::hash("real", 4).unwrap();
        let v = verify("guess", &legacy);
        assert!(!v.matched);
        assert!(!v.needs_rehash);
    }

    #[test]
    fn malformed_hashes_never_authenticate() {
        for stored in ["", "plaintext", "$unknown$abc", "$2b$notreallybcrypt"] {
            let v = verify("anything", stored);
            assert!(!v.matched, "{stored:?} must not authenticate");
            assert!(!v.needs_rehash);
        }
    }
}
