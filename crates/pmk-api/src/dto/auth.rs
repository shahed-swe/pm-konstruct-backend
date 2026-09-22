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

#[derive(Debug, Serialize)]
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

/// `GET /auth/permissions` -- the effective set after the managed sentinel is
/// applied, not the raw stored rows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsResponse {
    pub permissions: Vec<PermissionDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingStatusDto {
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
    pub billing: BillingStatusDto,
}

impl From<&SessionUser> for MeResponse {
    fn from(s: &SessionUser) -> Self {
        Self {
            user: UserDto::from(&s.user),
            permissions: s.effective().into_iter().map(Into::into).collect(),
            billing: BillingStatusDto {
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
