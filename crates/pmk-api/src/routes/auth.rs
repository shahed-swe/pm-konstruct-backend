//! Authentication routes.

use axum::extract::State;
use axum::http::header::{HeaderValue, SET_COOKIE};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use pmk_app::identity::LoginOutcome;

use crate::dto::{
    ForgotPasswordRequest, LoginRequest, LoginResponse, MeResponse, MessageDto, OkDto,
    PermissionsResponse, RegisterRequest, ResetPasswordRequest, SetupRequest, SetupStatusDto,
    UserDto,
};
use crate::error::ApiError;
use crate::extract::auth::{ACCESS_COOKIE, REFRESH_COOKIE};
use crate::extract::AuthUser;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/setup", get(setup_status).post(complete_setup))
        .route("/register", post(register))
        .route("/forgot-password", post(forgot_password))
        .route("/reset-password", post(reset_password))
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

// ── first run, registration and recovery ────────────────────────────────────

/// Whether this installation still needs its first account.
///
/// Unauthenticated by necessity: it is what the login page asks before
/// deciding whether to show a sign-in form or a setup form. It answers only
/// yes or no, and cannot be used to count or enumerate users.
async fn setup_status(State(state): State<AppState>) -> Result<Json<SetupStatusDto>, ApiError> {
    Ok(Json(SetupStatusDto {
        needs_setup: state.auth.needs_setup().await?,
    }))
}

/// Creates the first company and its manager.
///
/// Guarded by a secret held by whoever deploys the server. The endpoint has to
/// be unauthenticated -- before it runs there is nobody to authenticate as --
/// so the secret is the only thing standing in front of it, and a deployment
/// that leaves `SETUP_SECRET` unset has the endpoint disabled entirely.
async fn complete_setup(
    State(state): State<AppState>,
    Json(req): Json<SetupRequest>,
) -> Result<Response, ApiError> {
    let out = state
        .auth
        .complete_setup(
            &req.setup_key,
            &registration(
                req.company_name
                    .unwrap_or_else(|| "PM Konstruct".to_string()),
                req.name,
                req.email,
                req.password,
                req.phone,
            ),
        )
        .await?;
    Ok((StatusCode::CREATED, login_response(&state, out)).into_response())
}

/// Public sign-up: a new company with its first manager.
async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Response, ApiError> {
    let out = state
        .auth
        .register_company(&registration(
            req.company_name,
            req.name,
            req.email,
            req.password,
            req.phone,
        ))
        .await?;
    Ok((StatusCode::CREATED, login_response(&state, out)).into_response())
}

fn registration(
    company_name: String,
    name: String,
    email: String,
    password: String,
    phone: Option<String>,
) -> pmk_ports::repository::CompanyRegistration {
    pmk_ports::repository::CompanyRegistration {
        company_name,
        manager: pmk_domain::identity::accounts::UserInput {
            name,
            email,
            // The first account is always a manager: somebody has to be able
            // to add the others.
            role: pmk_domain::access::Role::Manager,
            phone,
            active: true,
            password: Some(password),
        },
    }
}

/// Starts an emailed password reset.
///
/// Always the same response, whatever happened: unknown address, inactive
/// account, a company with colleagues who should issue a code instead, or a
/// link genuinely sent. Anything that varied would tell an unauthenticated
/// caller which addresses are real.
async fn forgot_password(
    State(state): State<AppState>,
    Json(req): Json<ForgotPasswordRequest>,
) -> Result<Json<MessageDto>, ApiError> {
    let issued = state.auth.begin_password_reset(&req.email).await?;

    if let Some((_user, _token)) = issued {
        // Delivery needs the company's relay, which is reached through a
        // session this caller does not have. Phase 15 gives the worker a way
        // to send it; until then the token is stored and a manager-issued
        // code remains the supported path. The response is unchanged either
        // way, by design.
        tracing::info!("a password reset token was issued");
    }

    Ok(Json(MessageDto {
        message: pmk_domain::identity::recovery::FORGOT_PASSWORD_RESPONSE.to_string(),
    }))
}

/// Sets a new password from an emailed link or a manager-issued code.
async fn reset_password(
    State(state): State<AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<Json<OkDto>, ApiError> {
    state.auth.reset_password(&req.token, &req.password).await?;
    Ok(Json(OkDto::yes()))
}
