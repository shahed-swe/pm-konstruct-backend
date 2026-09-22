//! User-administration routes.
//!
//! Manager-only throughout, matching the contract: a supervisor cannot even
//! list their colleagues.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use pmk_domain::ids::UserId;

use crate::dto::{
    InviteDto, PermissionDto, RecoveryCodeDto, UserDto, UserPatchRequest, UserRequest,
};
use crate::error::ApiError;
use crate::extract::{ManagerOnly, RequireRole};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(remove))
        .route("/{id}/resend-invite", post(resend_invite))
        .route("/{id}/password-recovery-code", post(issue_recovery_code))
        .route("/{id}/permissions", get(permissions).put(set_permissions))
}

async fn list(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
) -> Result<Json<Vec<UserDto>>, ApiError> {
    let users = state.users.list(&session).await?;
    Ok(Json(users.iter().map(Into::into).collect()))
}

async fn get_one(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Path(id): Path<i32>,
) -> Result<Json<UserDto>, ApiError> {
    let user = state.users.get(&session, UserId(id)).await?;
    Ok(Json((&user).into()))
}

async fn create(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Json(req): Json<UserRequest>,
) -> Result<(StatusCode, Json<UserDto>), ApiError> {
    let user = state.users.create(&session, &req.into_input()?).await?;
    Ok((StatusCode::CREATED, Json((&user).into())))
}

async fn update(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Path(id): Path<i32>,
    Json(req): Json<UserPatchRequest>,
) -> Result<Json<UserDto>, ApiError> {
    // Read first, then merge: the users page deactivates someone by sending
    // `{"active": false}` alone.
    let current = state.users.get(&session, UserId(id)).await?;
    let user = state
        .users
        .update(&session, UserId(id), &req.apply(&current)?)
        .await?;
    Ok(Json((&user).into()))
}

async fn remove(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
    state.users.delete(&session, UserId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Echoes the invited address back for the manager to pass on.
///
/// Sends nothing: neither did the legacy endpoint, despite its name. Real
/// invitations arrive with Phase 14's email work.
async fn resend_invite(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Path(id): Path<i32>,
) -> Result<Json<InviteDto>, ApiError> {
    let user = state.users.get(&session, UserId(id)).await?;
    Ok(Json(InviteDto {
        email: user.email,
        name: user.name,
    }))
}

/// Issues a one-time code, shown once.
async fn issue_recovery_code(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Path(id): Path<i32>,
) -> Result<(StatusCode, Json<RecoveryCodeDto>), ApiError> {
    let code = state
        .users
        .issue_recovery_code(&session, UserId(id))
        .await?;
    Ok((StatusCode::CREATED, Json(code.into())))
}

async fn permissions(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Path(id): Path<i32>,
) -> Result<Json<Vec<PermissionDto>>, ApiError> {
    let perms = state.users.permissions(&session, UserId(id)).await?;
    Ok(Json(perms.into_iter().map(Into::into).collect()))
}

/// Replaces the user's permissions with exactly this list.
///
/// The body is the array itself, not an object wrapping it, because that is
/// what the existing admin UI sends.
async fn set_permissions(
    State(state): State<AppState>,
    RequireRole(session): ManagerOnly,
    Path(id): Path<i32>,
    Json(req): Json<Vec<PermissionDto>>,
) -> Result<Json<crate::dto::OkDto>, ApiError> {
    state
        .users
        .set_permissions(
            &session,
            UserId(id),
            req.into_iter().map(Into::into).collect(),
        )
        .await?;
    Ok(Json(crate::dto::OkDto::yes()))
}
