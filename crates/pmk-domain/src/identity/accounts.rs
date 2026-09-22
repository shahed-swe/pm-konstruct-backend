//! Managing user accounts: the rules a manager's changes have to satisfy.

use crate::access::{Permission, Role};
use crate::error::{DomainError, DomainResult};
use crate::identity::looks_like_email;
use crate::ids::UserId;

/// The shortest password a manager may set for someone.
///
/// The legacy had no minimum at all. Eight is the floor the OWASP guidance
/// gives for a password backed by a slow hash, which Argon2 is.
pub const MIN_PASSWORD_LEN: usize = 8;

/// Guards against a paste accident filling the column.
pub const MAX_PASSWORD_LEN: usize = 200;

/// A new or edited account.
#[derive(Debug, Clone)]
pub struct UserInput {
    pub name: String,
    pub email: String,
    pub role: Role,
    pub phone: Option<String>,
    pub active: bool,
    /// Required on create, optional on update. `None` leaves it alone.
    pub password: Option<String>,
}

impl UserInput {
    pub fn validate(&self, creating: bool) -> DomainResult<()> {
        if self.name.trim().is_empty() {
            return Err(DomainError::invalid("name", "a name is required"));
        }
        if self.name.chars().count() > 120 {
            return Err(DomainError::invalid(
                "name",
                "must be 120 characters or fewer",
            ));
        }
        if !looks_like_email(&self.email) {
            return Err(DomainError::invalid("email", "must be a valid address"));
        }

        match (&self.password, creating) {
            (None, true) => Err(DomainError::invalid(
                "password",
                "A password is required when creating a user",
            )),
            // An explicitly empty password on update means "leave it alone" in
            // the legacy UI, and must not be taken as "set it to empty".
            (Some(p), _) if p.is_empty() => Ok(()),
            (Some(p), _) => validate_password(p),
            (None, false) => Ok(()),
        }
    }

    /// The address as it will be stored and compared.
    #[must_use]
    pub fn normalised_email(&self) -> String {
        crate::identity::User::normalise_email(&self.email)
    }
}

pub fn validate_password(password: &str) -> DomainResult<()> {
    // Counted in characters, not bytes: a passphrase in a non-Latin script
    // would otherwise clear the bar on byte length alone.
    let len = password.chars().count();
    if len < MIN_PASSWORD_LEN {
        return Err(DomainError::invalid(
            "password",
            format!("must be at least {MIN_PASSWORD_LEN} characters"),
        ));
    }
    if len > MAX_PASSWORD_LEN {
        return Err(DomainError::invalid(
            "password",
            format!("must be {MAX_PASSWORD_LEN} characters or fewer"),
        ));
    }
    Ok(())
}

/// Whether a manager may act on this account at all.
///
/// A manager cannot deactivate or delete themselves: a company whose only
/// manager locks themselves out has no way back in without support.
pub fn assert_not_self(actor: UserId, target: UserId, what: &'static str) -> DomainResult<()> {
    if actor == target {
        return Err(DomainError::invalid("id", what));
    }
    Ok(())
}

/// Seat accounting for a company.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seats {
    /// `None` when the subscription does not cap seats.
    pub limit: Option<i64>,
    pub in_use: i64,
}

impl Seats {
    /// Is there room for one more active user?
    #[must_use]
    pub fn has_room(&self) -> bool {
        self.limit.is_none_or(|limit| self.in_use < limit)
    }

    /// The message shown when there is not.
    #[must_use]
    pub fn full_message(&self, activating: bool) -> String {
        let limit = self.limit.unwrap_or(0);
        let verb = if activating {
            "activating another user"
        } else {
            "adding another user"
        };
        format!("All {limit} paid seats are in use. Increase your seats in Billing before {verb}.")
    }
}

/// The alphabet a manager-issued recovery code is drawn from.
///
/// No `0`, `O`, `1`, `I` or `L`: the code gets read aloud over the phone or
/// copied off a screen, and those are the pairs people confuse.
pub const RECOVERY_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

/// How long a manager-issued recovery code lasts.
pub const RECOVERY_CODE_TTL_MINUTES: i64 = 15;

/// How many characters the code has.
///
/// Ten characters from a 31-symbol alphabet is about 49 bits, which is far
/// more than a 15-minute window needs.
pub const RECOVERY_CODE_LEN: usize = 10;

/// Builds a recovery code from random bytes.
///
/// Taking the bytes as an argument keeps this testable and keeps the choice of
/// random source in the adapter that has one.
#[must_use]
pub fn recovery_code_from(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| RECOVERY_CODE_ALPHABET[*b as usize % RECOVERY_CODE_ALPHABET.len()] as char)
        .collect()
}

/// Managers hold everything, so their permission list is empty rather than
/// exhaustive, and cannot be edited.
pub fn assert_permissions_editable(role: Role) -> DomainResult<()> {
    if role == Role::Manager {
        return Err(DomainError::invalid(
            "role",
            "Manager access cannot be restricted",
        ));
    }
    Ok(())
}

/// The sentinel that makes a stored permission list take effect (R5).
#[must_use]
pub fn managed_marker() -> Permission {
    Permission {
        resource: "access-rules".to_string(),
        action: "managed".to_string(),
    }
}

/// Adds the sentinel to a list a manager has just set.
///
/// Without it the stored rows are inert and the user silently keeps their role
/// defaults -- the counter-intuitive half of R5. Saving an empty list is how a
/// manager revokes everything, and that only works if the marker is there.
#[must_use]
pub fn mark_managed(mut permissions: Vec<Permission>) -> Vec<Permission> {
    let marker = managed_marker();
    if !permissions.contains(&marker) {
        permissions.push(marker);
    }
    permissions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> UserInput {
        UserInput {
            name: "Sarah Johnson".into(),
            email: "sarah@example.com".into(),
            role: Role::Supervisor,
            phone: None,
            active: true,
            password: Some("correct horse".into()),
        }
    }

    #[test]
    fn creating_a_user_requires_a_password() {
        let no_password = UserInput {
            password: None,
            ..input()
        };
        let e = no_password.validate(true).unwrap_err();
        assert!(matches!(e, DomainError::Invalid { field, .. } if field == "password"));
        // On update it simply means "leave it alone".
        assert!(no_password.validate(false).is_ok());
    }

    #[test]
    fn an_empty_password_on_update_leaves_it_alone() {
        // The legacy UI sends "" for "unchanged"; taking that literally would
        // set an empty password.
        let blank = UserInput {
            password: Some(String::new()),
            ..input()
        };
        assert!(blank.validate(false).is_ok());
    }

    #[test]
    fn a_short_password_is_rejected() {
        let short = UserInput {
            password: Some("short".into()),
            ..input()
        };
        assert!(short.validate(true).is_err());
        assert!(validate_password("1234567").is_err());
        assert!(validate_password("12345678").is_ok());
    }

    #[test]
    fn password_length_is_counted_in_characters_not_bytes() {
        // Seven characters, twenty-one bytes: long enough by bytes, not by
        // the rule that actually matters.
        let seven = "パスワードだよ";
        assert_eq!(seven.chars().count(), 7);
        assert!(seven.len() > MIN_PASSWORD_LEN);
        assert!(validate_password(seven).is_err());
    }

    #[test]
    fn an_absurdly_long_password_is_rejected() {
        assert!(validate_password(&"x".repeat(MAX_PASSWORD_LEN)).is_ok());
        assert!(validate_password(&"x".repeat(MAX_PASSWORD_LEN + 1)).is_err());
    }

    #[test]
    fn a_bad_email_is_rejected() {
        for bad in ["", "nope", "a b@c.com", "@c.com", "a@c", "a@.com"] {
            let u = UserInput {
                email: bad.into(),
                ..input()
            };
            assert!(u.validate(true).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn the_email_is_normalised_the_same_way_login_does_it() {
        let u = UserInput {
            email: "  Sarah@Example.COM ".into(),
            ..input()
        };
        assert_eq!(u.normalised_email(), "sarah@example.com");
    }

    #[test]
    fn a_manager_cannot_act_on_their_own_account() {
        assert!(assert_not_self(UserId(1), UserId(1), "nope").is_err());
        assert!(assert_not_self(UserId(1), UserId(2), "nope").is_ok());
    }

    #[test]
    fn an_uncapped_subscription_always_has_room() {
        let s = Seats {
            limit: None,
            in_use: 10_000,
        };
        assert!(s.has_room());
    }

    #[test]
    fn a_full_plan_has_no_room_and_says_so() {
        let s = Seats {
            limit: Some(5),
            in_use: 5,
        };
        assert!(!s.has_room());
        assert!(s.full_message(false).contains("All 5 paid seats"));
        assert!(s.full_message(false).contains("adding another user"));
        assert!(s.full_message(true).contains("activating another user"));
    }

    #[test]
    fn one_seat_below_the_limit_still_has_room() {
        assert!(Seats {
            limit: Some(5),
            in_use: 4
        }
        .has_room());
        // And being over it, which a downgrade can cause, does not.
        assert!(!Seats {
            limit: Some(5),
            in_use: 9
        }
        .has_room());
    }

    #[test]
    fn a_recovery_code_avoids_the_characters_people_misread() {
        let code = recovery_code_from(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(code.chars().count(), 10);
        for c in code.chars() {
            assert!(
                !"01OIL".contains(c),
                "{c} is easy to misread over the phone"
            );
            assert!(RECOVERY_CODE_ALPHABET.contains(&(c as u8)));
        }
    }

    #[test]
    fn every_byte_maps_into_the_alphabet() {
        // Including the ones past its length, where the modulo wraps.
        let all: Vec<u8> = (0..=255).collect();
        let code = recovery_code_from(&all);
        assert_eq!(code.chars().count(), 256);
        assert!(code
            .chars()
            .all(|c| RECOVERY_CODE_ALPHABET.contains(&(c as u8))));
    }

    #[test]
    fn manager_permissions_cannot_be_edited() {
        assert!(assert_permissions_editable(Role::Manager).is_err());
        assert!(assert_permissions_editable(Role::Supervisor).is_ok());
        assert!(assert_permissions_editable(Role::Office).is_ok());
    }

    #[test]
    fn saving_permissions_always_adds_the_sentinel() {
        // Without it the rows are inert and the user keeps their role
        // defaults -- the counter-intuitive half of R5.
        let out = mark_managed(vec![Permission {
            resource: "jobs".into(),
            action: "read".into(),
        }]);
        assert!(out.contains(&managed_marker()));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn revoking_everything_is_an_empty_list_plus_the_sentinel() {
        let out = mark_managed(Vec::new());
        assert_eq!(out, vec![managed_marker()]);
    }

    #[test]
    fn the_sentinel_is_not_added_twice() {
        let out = mark_managed(vec![managed_marker()]);
        assert_eq!(out.len(), 1);
    }
}
