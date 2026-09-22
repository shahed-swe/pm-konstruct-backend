//! Authenticated user shape and password-hash policy.

use crate::access::Role;
use crate::ids::UserId;
use crate::tenant::{AuthenticatedPrincipal, CompanyId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub company_id: CompanyId,
    pub name: String,
    pub email: String,
    pub role: Role,
    pub phone: Option<String>,
    pub active: bool,
    pub password_changed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl User {
    #[must_use]
    pub fn principal(&self) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal::new(self.id, self.company_id, self.role)
    }

    /// Emails are stored lower-cased and trimmed; the legacy login did the same
    /// normalisation on the way in, so the comparison must match.
    #[must_use]
    pub fn normalise_email(raw: &str) -> String {
        raw.trim().to_lowercase()
    }
}

/// Which algorithm produced a stored hash.
///
/// Production holds 18 bcrypt hashes at cost 12. Those must keep working, and
/// upgrade to Argon2id transparently on the next successful login -- nobody is
/// asked to reset a password because of the rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashKind {
    Argon2id,
    Bcrypt,
    Unknown,
}

impl HashKind {
    #[must_use]
    pub fn detect(hash: &str) -> Self {
        if hash.starts_with("$argon2id$") || hash.starts_with("$argon2i$") {
            Self::Argon2id
        } else if hash.starts_with("$2a$") || hash.starts_with("$2b$") || hash.starts_with("$2y$") {
            Self::Bcrypt
        } else {
            Self::Unknown
        }
    }

    /// Whether a successful verification should trigger a re-hash.
    #[must_use]
    pub const fn needs_rehash(self) -> bool {
        matches!(self, Self::Bcrypt)
    }
}

/// Deliberately minimal: a full RFC 5322 parser is not the point, and the
/// legacy system accepted anything with an @ in it.
///
/// Shared by job contacts and user accounts so the two cannot drift: an
/// address good enough to invite someone with must be good enough to record
/// as a site contact.
#[must_use]
pub fn looks_like_email(s: &str) -> bool {
    let s = s.trim();
    // Whitespace anywhere is invalid, including in the local part -- an earlier
    // version only checked the domain and accepted "a b@c.com".
    if s.chars().any(char::is_whitespace) {
        return false;
    }
    match s.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !domain.contains('@')
        }
        None => false,
    }
}

pub mod accounts;
pub mod recovery;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_legacy_bcrypt_prefixes() {
        for h in ["$2a$12$abc", "$2b$12$abc", "$2y$12$abc"] {
            assert_eq!(HashKind::detect(h), HashKind::Bcrypt, "{h}");
            assert!(HashKind::detect(h).needs_rehash());
        }
    }

    #[test]
    fn detects_argon2_and_does_not_rehash_it() {
        let h = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA";
        assert_eq!(HashKind::detect(h), HashKind::Argon2id);
        assert!(!HashKind::detect(h).needs_rehash());
    }

    #[test]
    fn unknown_hashes_are_not_silently_accepted() {
        assert_eq!(HashKind::detect("plaintext"), HashKind::Unknown);
        assert_eq!(HashKind::detect(""), HashKind::Unknown);
    }

    #[test]
    fn email_normalisation_matches_the_legacy_login() {
        assert_eq!(
            User::normalise_email("  Bob@Example.COM "),
            "bob@example.com"
        );
    }
}
