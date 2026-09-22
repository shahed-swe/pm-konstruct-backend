//! Authentication routes.

use axum::extract::State;
use axum::http::header::{HeaderValue, SET_COOKIE};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use pmk_app::identity::LoginOutcome;

use crate::dto::{LoginRequest, LoginResponse, MeResponse, PermissionsResponse, UserDto};
use crate::error::ApiError;
use crate::extract::auth::{ACCESS_COOKIE, REFRESH_COOKIE};
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/refresh", post(refresh))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/permissions", get(permissions))
}

/// Builds the session cookies.
///
/// `HttpOnly` so JavaScript cannot read them, which is the point: the legacy
/// client kept its token in `localStorage`, readable by any XSS. `SameSite=Lax`
/// allows top-level navigation while blocking cross-site form posts. The
/// refresh cookie is additionally path-scoped so it is only ever sent to the
/// refresh and logout endpoints.
fn session_cookies(state: &AppState, out: &LoginOutcome) -> Vec<HeaderValue> {
    let secure = if state.config.auth.cookie_secure {
        "; Secure"
    } else {
        ""
    };
    let domain = state
        .config
        .auth
        .cookie_domain
        .as_deref()
        .map(|d| format!("; Domain={d}"))
        .unwrap_or_default();

    [
        format!(
            "{ACCESS_COOKIE}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}{secure}{domain}",
            out.access_token, state.config.auth.access_token_ttl_secs
        ),
        format!(
            "{REFRESH_COOKIE}={}; HttpOnly; SameSite=Lax; Path=/api/auth; Max-Age={}{secure}{domain}",
            out.refresh_token, state.config.auth.refresh_token_ttl_secs
        ),
    ]
    .iter()
    .filter_map(|c| HeaderValue::from_str(c).ok())
    .collect()
}

fn clearing_cookies(state: &AppState) -> Vec<HeaderValue> {
    let secure = if state.config.auth.cookie_secure {
        "; Secure"
    } else {
        ""
    };
    [
        format!("{ACCESS_COOKIE}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0{secure}"),
        format!("{REFRESH_COOKIE}=; HttpOnly; SameSite=Lax; Path=/api/auth; Max-Age=0{secure}"),
    ]
    .iter()
    .filter_map(|c| HeaderValue::from_str(c).ok())
    .collect()
}

fn login_response(state: &AppState, out: LoginOutcome) -> Response {
    let cookies = session_cookies(state, &out);
    // The body still carries `token` so the legacy client and the captured
    // fixtures keep working during migration; new clients should rely on the
    // cookie instead.
    let body = LoginResponse {
        token: out.access_token.clone(),
        user: UserDto::from(&out.user),
    };
    let mut res = (StatusCode::OK, Json(body)).into_response();
    for c in cookies {
        res.headers_mut().append(SET_COOKIE, c);
    }
    res
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    if req.email.trim().is_empty() || req.password.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Email and password are required",
        ));
    }
    let out = state.auth.login(&req.email, &req.password).await?;
    Ok(login_response(&state, out))
}

async fn refresh(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Response, ApiError> {
    let presented = read_cookie(&headers, REFRESH_COOKIE).ok_or_else(ApiError::unauthenticated)?;
    let out = state.auth.refresh(&presented).await?;
    Ok(login_response(&state, out))
}

async fn logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Response, ApiError> {
    // Unlike the legacy stateless logout, this actually revokes the family
    // server-side. Always 200: logout is idempotent and must not reveal
    // whether the token was valid.
    if let Some(presented) = read_cookie(&headers, REFRESH_COOKIE) {
        state.auth.logout(&presented).await?;
    }
    let mut res = (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response();
    for c in clearing_cookies(&state) {
        res.headers_mut().append(SET_COOKIE, c);
    }
    Ok(res)
}

async fn me(AuthUser(session): AuthUser) -> Json<MeResponse> {
    Json(MeResponse::from(&session))
}

async fn permissions(AuthUser(session): AuthUser) -> Json<PermissionsResponse> {
    Json(PermissionsResponse {
        permissions: session.effective().into_iter().map(Into::into).collect(),
    })
}

fn read_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|raw| raw.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim().to_string())
}
