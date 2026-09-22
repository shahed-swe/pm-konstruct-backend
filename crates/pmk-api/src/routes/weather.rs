//! Current conditions for a coordinate.
//!
//! Used to stamp a diary entry with the weather at the moment it was written.
//! Rate-limited per user, because the upstream plan is metered and billed to
//! the company: a page stuck in a refresh loop must not be able to spend the
//! month's quota.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use pmk_domain::weather::Coordinates;

use crate::dto::WeatherDto;
use crate::error::ApiError;
use crate::extract::Entitled;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(current))
}

/// Coordinates arrive as strings so the decimal-degrees rule can reject the
/// spellings `f64` would otherwise accept -- `NaN`, `inf`, `1e400`.
#[derive(Debug, serde::Deserialize)]
pub struct CoordQuery {
    pub lat: Option<String>,
    pub lon: Option<String>,
}

async fn current(
    State(state): State<AppState>,
    Entitled(session): Entitled,
    Query(q): Query<CoordQuery>,
) -> Result<Json<WeatherDto>, ApiError> {
    let (Some(lat), Some(lon)) = (q.lat, q.lon) else {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "lat and lon query params are required",
        ));
    };
    let coords = Coordinates::parse(&lat, &lon)?;
    let stamp = state.weather.current(&session, coords).await?;
    Ok(Json(stamp.into()))
}
