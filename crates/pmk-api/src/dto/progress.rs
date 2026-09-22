//! Progress-record shapes.

use pmk_domain::ids::JobId;
use pmk_domain::progress::{Progress, ProgressInput};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressDto {
    pub id: i32,
    pub job_id: i32,
    pub date: chrono::NaiveDate,
    pub percent_complete: f32,
    pub milestone: Option<String>,
    pub description: Option<String>,
    pub photos: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Progress> for ProgressDto {
    fn from(p: Progress) -> Self {
        Self {
            id: p.id.get(),
            job_id: p.job_id.get(),
            date: p.date,
            percent_complete: p.percent_complete,
            milestone: p.milestone,
            description: p.description,
            photos: p.photos,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressRequest {
    /// Ignored on update: a record stays on the job it was raised against.
    #[serde(default)]
    pub job_id: i32,
    pub date: chrono::NaiveDate,
    pub percent_complete: f32,
    pub milestone: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub photos: Vec<String>,
}

/// A partial update to a progress record.
///
/// The progress page edits one field at a time -- a percentage nudged, a
/// milestone named -- and the legacy service copied across only what
/// arrived.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressPatchRequest {
    pub date: Option<chrono::NaiveDate>,
    pub percent_complete: Option<f32>,
    #[serde(default, deserialize_with = "double_option")]
    pub milestone: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub description: Option<Option<String>>,
    pub photos: Option<Vec<String>>,
}

/// Distinguishes "key absent" from "key present and null".
fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

impl ProgressPatchRequest {
    #[must_use]
    pub fn apply(self, current: &pmk_domain::progress::Progress) -> ProgressInput {
        ProgressInput {
            job_id: current.job_id,
            date: self.date.unwrap_or(current.date),
            percent_complete: self.percent_complete.unwrap_or(current.percent_complete),
            milestone: self.milestone.unwrap_or_else(|| current.milestone.clone()),
            description: self
                .description
                .unwrap_or_else(|| current.description.clone()),
            photos: self.photos.unwrap_or_else(|| current.photos.clone()),
        }
    }
}

impl From<ProgressRequest> for ProgressInput {
    fn from(r: ProgressRequest) -> Self {
        Self {
            job_id: JobId(r.job_id),
            date: r.date,
            percent_complete: r.percent_complete,
            milestone: r.milestone,
            description: r.description,
            photos: r.photos,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProgressListQuery {
    pub job_id: Option<i32>,
}
