use pmk_app::identity::SessionUser;
use pmk_domain::access::Permission;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Matches the legacy `{ token, user }` response so existing clients and the
/// captured fixtures need no changes.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub token: String,
    pub user: UserDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
    pub id: i32,
    pub company_id: i32,
    pub name: String,
    pub email: String,
    pub role: String,
    pub phone: Option<String>,
    pub active: bool,
}

impl From<&pmk_domain::User> for UserDto {
    fn from(u: &pmk_domain::User) -> Self {
        Self {
            id: u.id.get(),
            company_id: u.company_id.get(),
            name: u.name.clone(),
            email: u.email.clone(),
            role: u.role.as_str().to_string(),
            phone: u.phone.clone(),
            active: u.active,
        }
    }
}

// Deserialize as well: the admin screen sends this shape back when a manager
// edits someone's access.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDto {
    pub resource: String,
    pub action: String,
}

impl From<Permission> for PermissionDto {
    fn from(p: Permission) -> Self {
        Self {
            resource: p.resource,
            action: p.action,
        }
    }
}

/// The admin screen sends permissions back in the same shape it received them.
impl From<PermissionDto> for Permission {
    fn from(p: PermissionDto) -> Self {
        Self {
            resource: p.resource,
            action: p.action,
        }
    }
}

/// `GET /auth/permissions` -- the effective set after the managed sentinel is
/// applied, not the raw stored rows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsResponse {
    pub permissions: Vec<PermissionDto>,
}

/// The entitlement summary carried on `/auth/me`, so the client knows
/// whether to show the application or redirect to billing.
///
/// Not the billing page's own status, which is `dto::billing::BillingStatusDto`
/// and reads the subscription from Stripe. This one is derived from what the
/// session already resolved and costs nothing extra.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntitlementDto {
    pub access_allowed: bool,
    pub onboarding_complete: bool,
    pub can_configure_account: bool,
}

/// `GET /auth/me`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub user: UserDto,
    pub permissions: Vec<PermissionDto>,
    pub billing: EntitlementDto,
}

impl From<&SessionUser> for MeResponse {
    fn from(s: &SessionUser) -> Self {
        Self {
            user: UserDto::from(&s.user),
            permissions: s.effective().into_iter().map(Into::into).collect(),
            billing: EntitlementDto {
                access_allowed: s.entitlement.can_access_application,
                onboarding_complete: s.entitlement.onboarding_complete,
                can_configure_account: s.entitlement.can_configure_account,
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: &'static str,
    pub ready: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyResponse {
    pub status: &'static str,
    pub database: bool,
    pub migrations: bool,
}

// ── first run, registration and recovery ────────────────────────────────────

/// Whether this installation still needs its first account.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatusDto {
    pub needs_setup: bool,
}

/// The first-run form, plus the shared secret that authorises it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupRequest {
    pub setup_key: String,
    pub name: String,
    pub email: String,
    pub password: String,
    pub phone: Option<String>,
    pub company_name: Option<String>,
}

/// The public sign-up form.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    pub company_name: String,
    pub name: String,
    pub email: String,
    pub password: String,
    pub phone: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgotPasswordRequest {
    pub email: String,
}

/// Always the same wording, whatever happened.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDto {
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPasswordRequest {
    pub token: String,
    pub password: String,
}
