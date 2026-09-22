//! User-administration shapes.
//!
//! No response here ever carries a password hash. The domain `User` does not
//! hold one, so that is structural rather than a habit.
//!
//! `UserDto` and `PermissionDto` are the ones the auth routes already define:
//! the admin screen and the session endpoint describe the same user, and two
//! shapes for it would drift.

use pmk_app::users::IssuedRecoveryCode;
use pmk_domain::access::Role;
use pmk_domain::identity::accounts::UserInput;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRequest {
    pub name: String,
    pub email: String,
    pub role: String,
    pub phone: Option<String>,
    /// Defaults to active: the admin form's checkbox starts ticked.
    #[serde(default = "default_true")]
    pub active: bool,
    /// Required on create. On update, absent or empty means "unchanged".
    pub password: Option<String>,
}

const fn default_true() -> bool {
    true
}

impl UserRequest {
    /// Parses the role, which is a closed set rather than free text.
    pub fn into_input(self) -> Result<UserInput, crate::error::ApiError> {
        let role = Role::parse(&self.role).ok_or_else(|| {
            crate::error::ApiError::new(
                axum::http::StatusCode::BAD_REQUEST,
                "must be MANAGER, SUPERVISOR or OFFICE",
            )
            .with_field("role")
        })?;
        Ok(UserInput {
            name: self.name,
            email: self.email,
            role,
            phone: self.phone,
            active: self.active,
            password: self.password,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCodeDto {
    /// Shown once, then unrecoverable: only its hash is stored.
    pub recovery_code: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub user: RecoveryUserDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryUserDto {
    pub id: i32,
    pub name: String,
}

impl From<IssuedRecoveryCode> for RecoveryCodeDto {
    fn from(c: IssuedRecoveryCode) -> Self {
        Self {
            recovery_code: c.code,
            expires_at: c.expires_at,
            user: RecoveryUserDto {
                id: c.user_id.get(),
                name: c.user_name,
            },
        }
    }
}

/// What `resend-invite` returns.
///
/// The legacy endpoint sent no email -- it only echoed the address back, and
/// the frontend showed it for the manager to pass on by hand. Reproduced
/// as-is; sending a real invitation is Phase 14's email work.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteDto {
    pub email: String,
    pub name: String,
}
