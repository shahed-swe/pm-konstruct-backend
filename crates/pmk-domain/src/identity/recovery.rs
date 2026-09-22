//! Password recovery: the two ways back into an account.
//!
//! A manager issues a short code for a colleague who is locked out. A
//! one-person company has no manager to ask, so it can request a link by
//! email instead. Both end at the same endpoint and are distinguished by how
//! the stored token is shaped.

use crate::error::{DomainError, DomainResult};
use crate::identity::accounts::{validate_password, RECOVERY_CODE_ALPHABET};

/// The message `forgot-password` always returns.
///
/// Identical whether or not the address exists, whether or not the company is
/// eligible, and whether or not delivery succeeded. Anything that varied
/// would tell an unauthenticated caller which addresses are real.
pub const FORGOT_PASSWORD_RESPONSE: &str = "If an account exists and email delivery is available, \
    a reset link will be sent. If it does not arrive, contact your site manager.";

/// What `reset-password` says when the token does not work.
///
/// One message for expired, already-used, and never-existed: distinguishing
/// them would let someone probe which codes had been issued.
pub const INVALID_TOKEN_MESSAGE: &str = "This reset link or recovery code is invalid, expired, \
    or has already been used. Ask your Manager for a new code.";

/// How long an emailed reset link lasts.
pub const RESET_LINK_TTL_MINUTES: i64 = 60;

/// Is this company allowed to reset by email?
///
/// Only when it has exactly one active user. With anyone else on the account
/// there is a manager to ask, and a manager-issued code cannot be intercepted
/// by whoever controls the mailbox. The narrower path is the safer one, and
/// this is the deliberate exception for a sole trader.
#[must_use]
pub const fn may_reset_by_email(active_users_in_company: i64) -> bool {
    active_users_in_company == 1
}

/// Normalises a manager-issued code before it is hashed.
///
/// Uppercased with everything outside the alphabet removed, so a code read
/// aloud and typed back with spaces or dashes still matches.
#[must_use]
pub fn normalise_recovery_code(raw: &str) -> String {
    raw.chars()
        .flat_map(char::to_uppercase)
        .filter(|c| RECOVERY_CODE_ALPHABET.contains(&(*c as u8)))
        .collect()
}

/// How a submitted secret might be stored.
///
/// A reset endpoint is handed one string and has to try both shapes: an
/// emailed link carries a random token stored verbatim, while a manager code
/// is stored only as a prefixed hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetCandidates {
    /// The value as given, for an emailed link.
    pub raw: String,
    /// The normalised code, for a manager-issued one. Empty when the input
    /// has no usable characters.
    pub normalised_code: String,
}

impl ResetCandidates {
    #[must_use]
    pub fn of(submitted: &str) -> Self {
        Self {
            raw: submitted.trim().to_string(),
            normalised_code: normalise_recovery_code(submitted),
        }
    }
}

/// Checks a submitted reset before anything is looked up.
pub fn validate_reset(token: &str, password: &str) -> DomainResult<()> {
    if token.trim().is_empty() {
        return Err(DomainError::invalid("token", "Reset token is required"));
    }
    validate_password(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_one_person_company_may_reset_by_email() {
        assert!(may_reset_by_email(1));
        assert!(!may_reset_by_email(2));
        assert!(!may_reset_by_email(18));
        // Zero should not happen, and is refused rather than allowed.
        assert!(!may_reset_by_email(0));
    }

    #[test]
    fn a_code_read_aloud_and_typed_back_still_matches() {
        // Spaces, dashes and case are how a code survives a phone call.
        let issued = "RNK5GZATKB";
        for typed in [
            "RNK5GZATKB",
            "rnk5gzatkb",
            "RNK5 GZAT KB",
            "RNK5-GZAT-KB",
            "  rnk5 gzat kb  ",
        ] {
            assert_eq!(normalise_recovery_code(typed), issued, "{typed:?}");
        }
    }

    #[test]
    fn characters_outside_the_alphabet_are_dropped() {
        // The alphabet excludes 0, O, 1, I and L precisely because they get
        // misread; dropping them is better than matching the wrong code.
        assert_eq!(normalise_recovery_code("AB0CD"), "ABCD");
        assert_eq!(normalise_recovery_code("AB1CD"), "ABCD");
        assert_eq!(normalise_recovery_code("ABOCD"), "ABCD");
        assert_eq!(normalise_recovery_code("!@#$%"), "");
    }

    #[test]
    fn both_shapes_are_offered_for_lookup() {
        let c = ResetCandidates::of("  abc-123  ");
        assert_eq!(c.raw, "abc-123");
        // 1 is not in the alphabet, so it is dropped.
        assert_eq!(c.normalised_code, "ABC23");
    }

    #[test]
    fn a_hex_link_token_survives_as_the_raw_candidate() {
        let token = "9f8e7d6c5b4a39281706f5e4d3c2b1a0";
        let c = ResetCandidates::of(token);
        assert_eq!(c.raw, token);
    }

    #[test]
    fn a_reset_needs_a_token_and_a_long_enough_password() {
        assert!(validate_reset("", "longenough1").is_err());
        assert!(validate_reset("   ", "longenough1").is_err());
        assert!(validate_reset("tok", "short").is_err());
        assert!(validate_reset("tok", "longenough1").is_ok());
    }

    #[test]
    fn the_failure_messages_do_not_distinguish_the_reasons() {
        // Expired, used and never-existed all read the same, so nobody can
        // probe which codes were issued.
        assert!(INVALID_TOKEN_MESSAGE.contains("invalid, expired, or has already been used"));
        assert!(!FORGOT_PASSWORD_RESPONSE
            .to_lowercase()
            .contains("not found"));
    }
}
