//! Access and refresh tokens.
//!
//! Access tokens are short-lived JWTs (15 min). Refresh tokens are opaque
//! random strings, stored only as SHA-256 hashes, rotated on every use, with
//! family-wide revocation on reuse.
//!
//! `password_changed_at` is embedded in the access token and re-checked on
//! every request, so changing a password invalidates outstanding tokens
//! immediately. The legacy system did the same; it is preserved.

use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use pmk_domain::access::Role;
use pmk_domain::ids::UserId;
use pmk_domain::tenant::CompanyId;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessClaims {
    /// Subject: the user id, as a string per the JWT spec.
    pub sub: String,
    pub company_id: i32,
    pub role: String,
    pub email: String,
    pub name: String,
    /// Milliseconds since the epoch, or null. Matches the legacy claim so a
    /// password change invalidates tokens the same way.
    pub password_changed_at_ms: Option<i64>,
    pub iat: i64,
    pub exp: i64,
}

impl AccessClaims {
    pub fn user_id(&self) -> AppResult<UserId> {
        self.sub
            .parse::<i32>()
            .map(UserId)
            .map_err(|_| AppError::InvalidToken)
    }

    #[must_use]
    pub fn company(&self) -> CompanyId {
        CompanyId(self.company_id)
    }

    pub fn role(&self) -> AppResult<Role> {
        Role::parse(&self.role).ok_or(AppError::InvalidToken)
    }
}

#[derive(Clone)]
pub struct TokenCodec {
    encoding: EncodingKey,
    decoding: DecodingKey,
    access_ttl: chrono::Duration,
}

impl std::fmt::Debug for TokenCodec {
    // jsonwebtoken's key types are not Debug, and printing a signing key would
    // be a leak even if they were.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenCodec")
            .field("access_ttl", &self.access_ttl)
            .finish_non_exhaustive()
    }
}

impl TokenCodec {
    pub fn new(secret: &str, access_ttl: std::time::Duration) -> AppResult<Self> {
        let ttl = chrono::Duration::from_std(access_ttl).map_err(AppError::internal)?;
        Ok(Self {
            encoding: EncodingKey::from_secret(secret.as_bytes()),
            decoding: DecodingKey::from_secret(secret.as_bytes()),
            access_ttl: ttl,
        })
    }

    pub fn issue_access(
        &self,
        user: &pmk_domain::User,
        now: DateTime<Utc>,
    ) -> AppResult<(String, AccessClaims)> {
        let claims = AccessClaims {
            sub: user.id.get().to_string(),
            company_id: user.company_id.get(),
            role: user.role.as_str().to_string(),
            email: user.email.clone(),
            name: user.name.clone(),
            password_changed_at_ms: user.password_changed_at.map(|t| t.timestamp_millis()),
            iat: now.timestamp(),
            exp: (now + self.access_ttl).timestamp(),
        };
        let token = jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .map_err(AppError::internal)?;
        Ok((token, claims))
    }

    pub fn verify_access(&self, token: &str) -> AppResult<AccessClaims> {
        let mut v = Validation::new(Algorithm::HS256);
        v.set_required_spec_claims(&["exp", "sub"]);
        // Only HS256 is accepted, so an `alg: none` or RS256 confusion attack
        // cannot get a forged token past verification.
        jsonwebtoken::decode::<AccessClaims>(token, &self.decoding, &v)
            .map(|d| d.claims)
            .map_err(|_| AppError::InvalidToken)
    }
}

/// A freshly minted refresh token: the value to hand the client, and the hash
/// to store. The plaintext is never persisted.
#[derive(Debug, Clone)]
pub struct RefreshToken {
    pub value: String,
    pub hash: String,
}

impl RefreshToken {
    /// 32 bytes from the OS CSPRNG, hex-encoded.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let value = hex::encode(bytes);
        let hash = hash_refresh(&value);
        Self { value, hash }
    }
}

/// SHA-256, not a password hash: the input is 256 bits of entropy, so there is
/// nothing to brute-force and a slow KDF would only add latency to every
/// refresh.
#[must_use]
pub fn hash_refresh(value: &str) -> String {
    let mut h = Sha256::new();
    h.update(value.as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pmk_domain::User;

    fn codec() -> TokenCodec {
        TokenCodec::new(&"s".repeat(48), std::time::Duration::from_secs(900)).unwrap()
    }

    fn user() -> User {
        User {
            id: UserId(7),
            company_id: CompanyId(3),
            name: "Alpha Manager".into(),
            email: "am@a.test".into(),
            role: Role::Manager,
            phone: None,
            active: true,
            password_changed_at: None,
        }
    }

    #[test]
    fn access_token_round_trips_with_its_claims() {
        let c = codec();
        let (t, issued) = c.issue_access(&user(), Utc::now()).unwrap();
        let back = c.verify_access(&t).unwrap();
        assert_eq!(back.user_id().unwrap(), UserId(7));
        assert_eq!(back.company(), CompanyId(3));
        assert_eq!(back.role().unwrap(), Role::Manager);
        assert_eq!(back.exp, issued.exp);
    }

    #[test]
    fn a_token_from_another_secret_is_rejected() {
        let (t, _) = codec().issue_access(&user(), Utc::now()).unwrap();
        let other = TokenCodec::new(&"x".repeat(48), std::time::Duration::from_secs(900)).unwrap();
        assert!(other.verify_access(&t).is_err());
    }

    #[test]
    fn an_expired_token_is_rejected() {
        let c = codec();
        let (t, _) = c
            .issue_access(&user(), Utc::now() - chrono::Duration::hours(2))
            .unwrap();
        assert!(c.verify_access(&t).is_err());
    }

    #[test]
    fn garbage_is_rejected_rather_than_panicking() {
        for t in ["", "not.a.jwt", "a.b.c"] {
            assert!(codec().verify_access(t).is_err(), "{t:?}");
        }
    }

    #[test]
    fn password_change_time_is_carried_so_tokens_can_be_invalidated() {
        let mut u = user();
        u.password_changed_at = Some(Utc::now());
        let c = codec();
        let (t, _) = c.issue_access(&u, Utc::now()).unwrap();
        assert!(c
            .verify_access(&t)
            .unwrap()
            .password_changed_at_ms
            .is_some());
    }

    #[test]
    fn refresh_tokens_are_unique_and_only_the_hash_is_storable() {
        let a = RefreshToken::generate();
        let b = RefreshToken::generate();
        assert_ne!(a.value, b.value);
        assert_eq!(a.hash, hash_refresh(&a.value));
        assert_ne!(a.hash, a.value, "the stored form must not be the token");
        assert_eq!(a.value.len(), 64, "32 bytes hex-encoded");
    }
}
