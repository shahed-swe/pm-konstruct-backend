use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use pmk_app::identity::SessionUser;
use pmk_domain::access::Role;

use crate::error::ApiError;
use crate::state::AppState;

/// Name of the cookie that carries the access token.
///
/// A cookie as well as a header because browsers cannot attach an
/// `Authorization` header to `<img>` or `<video>` requests, and media is served
/// behind auth. The legacy API did the same via `auth_token`.
pub const ACCESS_COOKIE: &str = "pmk_access";
pub const REFRESH_COOKIE: &str = "pmk_refresh";

fn bearer_from(parts: &Parts) -> Option<String> {
    if let Some(v) = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(rest) = v.strip_prefix("Bearer ") {
            let t = rest.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    cookie_value(parts, ACCESS_COOKIE)
}

pub(crate) fn cookie_value(parts: &Parts, name: &str) -> Option<String> {
    parts
        .headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|raw| raw.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim().to_string())
}

/// A verified caller. Rejects with 401 when absent or invalid.
#[derive(Debug, Clone)]
pub struct AuthUser(pub SessionUser);

impl std::ops::Deref for AuthUser {
    type Target = SessionUser;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Resolved once per request and cached in extensions, so several
        // extractors on one handler cost a single lookup.
        if let Some(cached) = parts.extensions.get::<SessionUser>() {
            return Ok(Self(cached.clone()));
        }
        let token = bearer_from(parts).ok_or_else(ApiError::unauthenticated)?;
        let session = state
            .auth
            .resolve_session(&token)
            .await
            .map_err(ApiError::from)?;
        parts.extensions.insert(session.clone());
        Ok(Self(session))
    }
}

/// A caller whose subscription permits using the application.
///
/// Returns **402 with `BILLING_REQUIRED`**, which the frontend converts into a
/// redirect to `/billing` (domain-rules R6).
#[derive(Debug, Clone)]
pub struct Entitled(pub SessionUser);

impl std::ops::Deref for Entitled {
    type Target = SessionUser;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromRequestParts<AppState> for Entitled {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let AuthUser(session) = AuthUser::from_request_parts(parts, state).await?;
        if !session.entitlement.can_access_application {
            return Err(ApiError::billing_required());
        }
        Ok(Self(session))
    }
}

/// Requires one of a fixed set of roles.
#[derive(Debug, Clone)]
pub struct RequireRole<const MANAGER: bool, const SUPERVISOR: bool, const OFFICE: bool>(
    pub SessionUser,
);

impl<const M: bool, const S: bool, const O: bool> FromRequestParts<AppState>
    for RequireRole<M, S, O>
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Entitled(session) = Entitled::from_request_parts(parts, state).await?;
        let allowed = match session.user.role {
            Role::Manager => M,
            Role::Supervisor => S,
            Role::Office => O,
        };
        if allowed {
            Ok(Self(session))
        } else {
            Err(ApiError::forbidden())
        }
    }
}

/// `requireManager`.
pub type ManagerOnly = RequireRole<true, false, false>;
/// `requireManagerOrSupervisor`.
pub type ManagerOrSupervisor = RequireRole<true, true, false>;

/// A `resource:action` pair, named at the type level.
///
/// Rust forbids `&'static str` as a const generic parameter, so the pair is
/// carried by a marker type instead. The upside is that call sites read as
/// `RequirePermission<JobsWrite>` rather than a pair of string literals.
pub trait PermissionSpec: Send + Sync + 'static {
    const RESOURCE: &'static str;
    const ACTION: &'static str;
}

/// Declares a permission marker.
#[macro_export]
macro_rules! permission {
    ($name:ident, $resource:literal, $action:literal) => {
        #[derive(Debug, Clone, Copy)]
        pub struct $name;
        impl $crate::extract::auth::PermissionSpec for $name {
            const RESOURCE: &'static str = $resource;
            const ACTION: &'static str = $action;
        }
    };
}

// The permission vocabulary in use, from docs/contract/rbac-matrix.csv.
permission!(JobsRead, "jobs", "read");
permission!(JobsWrite, "jobs", "write");
permission!(SiteDiaryRead, "site-diary", "read");
permission!(SiteDiaryWrite, "site-diary", "write");
permission!(CallForwardRead, "call-forward", "read");
permission!(CallForwardWrite, "call-forward", "write");
permission!(ReportsRead, "reports", "read");
permission!(TradeSchedulerRead, "trade-scheduler", "read");
permission!(TradeSchedulerWrite, "trade-scheduler", "write");

/// Managers pass unconditionally; every other role needs `P`'s permission.
///
/// Mirrors `requireManagerOrPermission` and
/// `requireManagerSupervisorOrPermission`, which despite their names behave
/// identically in the legacy code.
#[derive(Debug, Clone)]
pub struct RequirePermission<P: PermissionSpec>(pub SessionUser, std::marker::PhantomData<P>);

impl<P: PermissionSpec> std::ops::Deref for RequirePermission<P> {
    type Target = SessionUser;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<P: PermissionSpec> FromRequestParts<AppState> for RequirePermission<P> {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Entitled(session) = Entitled::from_request_parts(parts, state).await?;
        if session.can(P::RESOURCE, P::ACTION) {
            Ok(Self(session, std::marker::PhantomData))
        } else {
            Err(ApiError::forbidden())
        }
    }
}
