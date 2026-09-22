//! Site diary DTOs.

use pmk_domain::diary::{
    ActionStatus, DiaryEntry, DiaryEntryInput, DiaryNote, DiaryNoteComment, DiaryNoteInput,
    NoteCategory,
};
use pmk_domain::ids::UserId;
use pmk_domain::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherStampDto {
    pub location_name: Option<String>,
    pub location_lat: Option<rust_decimal::Decimal>,
    pub location_lng: Option<rust_decimal::Decimal>,
    pub temperature: Option<rust_decimal::Decimal>,
    pub weather_condition: Option<String>,
    pub weather_icon: Option<String>,
    pub wind_speed_kmh: Option<rust_decimal::Decimal>,
    pub rainfall_mm: Option<rust_decimal::Decimal>,
    pub sunrise_time: Option<String>,
    pub sunset_time: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryEntryDto {
    pub id: i32,
    pub job_id: i32,
    pub date: chrono::NaiveDate,
    pub time: Option<String>,
    pub author_id: Option<i32>,
    pub weather: Option<String>,
    pub workforce: Option<i32>,
    pub work_completed: String,
    pub materials: Option<String>,
    pub trades_on_site: Option<String>,
    pub safety_notes: Option<String>,
    pub client_instructions: Option<String>,
    pub equipment: Option<String>,
    pub visitors: Option<String>,
    pub issues: Option<String>,
    pub notes: Option<String>,
    pub action_status: Option<String>,
    pub action_raised_by: Option<i32>,
    #[serde(flatten)]
    pub weather_stamp: WeatherStampDto,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<DiaryEntry> for DiaryEntryDto {
    fn from(e: DiaryEntry) -> Self {
        Self {
            id: e.id.get(),
            job_id: e.job_id.get(),
            date: e.date,
            time: e.time,
            author_id: e.author_id.map(UserId::get),
            weather: e.weather,
            workforce: e.workforce,
            work_completed: e.work_completed,
            materials: e.materials,
            trades_on_site: e.trades_on_site,
            safety_notes: e.safety_notes,
            client_instructions: e.client_instructions,
            equipment: e.equipment,
            visitors: e.visitors,
            issues: e.issues,
            notes: e.notes,
            action_status: e.action_status.map(|s| s.as_str().to_string()),
            action_raised_by: e.action_raised_by.map(UserId::get),
            weather_stamp: WeatherStampDto {
                location_name: e.weather_stamp.location_name,
                location_lat: e.weather_stamp.location_lat,
                location_lng: e.weather_stamp.location_lng,
                temperature: e.weather_stamp.temperature,
                weather_condition: e.weather_stamp.weather_condition,
                weather_icon: e.weather_stamp.weather_icon,
                wind_speed_kmh: e.weather_stamp.wind_speed_kmh,
                rainfall_mm: e.weather_stamp.rainfall_mm,
                sunrise_time: e.weather_stamp.sunrise_time,
                sunset_time: e.weather_stamp.sunset_time,
            },
            created_at: e.created_at,
            updated_at: e.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryEntryRequest {
    pub job_id: Option<i32>,
    pub date: Option<chrono::NaiveDate>,
    pub time: Option<String>,
    pub weather: Option<String>,
    pub workforce: Option<i32>,
    pub work_completed: Option<String>,
    pub materials: Option<String>,
    pub trades_on_site: Option<String>,
    pub safety_notes: Option<String>,
    pub client_instructions: Option<String>,
    pub equipment: Option<String>,
    pub visitors: Option<String>,
    pub issues: Option<String>,
    pub notes: Option<String>,
    pub action_status: Option<String>,
}

impl DiaryEntryRequest {
    pub fn into_input(self) -> Result<DiaryEntryInput, DomainError> {
        Ok(DiaryEntryInput {
            job_id: self.job_id.unwrap_or_default(),
            date: self.date,
            time: self.time,
            weather: self.weather,
            workforce: self.workforce,
            work_completed: self.work_completed.unwrap_or_default(),
            materials: self.materials,
            trades_on_site: self.trades_on_site,
            safety_notes: self.safety_notes,
            client_instructions: self.client_instructions,
            equipment: self.equipment,
            visitors: self.visitors,
            issues: self.issues,
            notes: self.notes,
            action_status: parse_action(self.action_status.as_deref())?,
        })
    }
}

/// Shared by the entry and note action-status endpoints.
///
/// An explicit `null` clears the status; an unrecognised value is a 400 rather
/// than reaching `site_diary_action_status_check`.
pub fn parse_action(raw: Option<&str>) -> Result<Option<ActionStatus>, DomainError> {
    match raw {
        None | Some("") => Ok(None),
        Some(s) => ActionStatus::parse(s).map(Some).ok_or_else(|| {
            DomainError::invalid(
                "actionStatus",
                "must be one of action, processing, completed",
            )
        }),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryNoteDto {
    pub id: i32,
    pub diary_entry_id: i32,
    pub category: String,
    pub content: String,
    pub action_status: Option<String>,
    pub action_raised_by: Option<i32>,
    pub sort_order: i32,
    pub archived: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<DiaryNote> for DiaryNoteDto {
    fn from(n: DiaryNote) -> Self {
        Self {
            id: n.id.get(),
            diary_entry_id: n.diary_entry_id.get(),
            category: n.category.as_str().to_string(),
            content: n.content,
            action_status: n.action_status.map(|s| s.as_str().to_string()),
            action_raised_by: n.action_raised_by.map(UserId::get),
            sort_order: n.sort_order,
            archived: n.archived,
            created_at: n.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryNoteRequest {
    pub category: Option<String>,
    pub content: String,
    pub action_status: Option<String>,
    pub sort_order: Option<i32>,
}

impl DiaryNoteRequest {
    pub fn into_input(self) -> Result<DiaryNoteInput, DomainError> {
        let category = match self.category.as_deref() {
            None | Some("") => NoteCategory::General,
            Some(c) => NoteCategory::parse(c).ok_or_else(|| {
                DomainError::invalid("category", format!("must be one of {}", NoteCategory::ALL))
            })?,
        };
        Ok(DiaryNoteInput {
            category,
            content: self.content,
            action_status: parse_action(self.action_status.as_deref())?,
            sort_order: self.sort_order,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionStatusRequest {
    pub action_status: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryCommentDto {
    pub id: i32,
    pub note_id: i32,
    pub author_id: Option<i32>,
    pub author_name: Option<String>,
    pub content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<DiaryNoteComment> for DiaryCommentDto {
    fn from(c: DiaryNoteComment) -> Self {
        Self {
            id: c.id,
            note_id: c.note_id.get(),
            author_id: c.author_id.map(UserId::get),
            author_name: c.author_name,
            content: c.content,
            created_at: c.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentRequest {
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryListQuery {
    pub job_id: Option<i32>,
    pub from: Option<chrono::NaiveDate>,
    pub to: Option<chrono::NaiveDate>,
    pub action_status: Option<String>,
    #[serde(default)]
    pub include_archived: bool,
}

/// Emailing a diary entry to colleagues on the job.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailEntryRequest {
    pub to: Vec<String>,
    #[serde(default)]
    pub subject: String,
    pub custom_message: Option<String>,
}

/// What was actually sent, and to whom.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSentDto {
    pub success: bool,
    pub message: String,
}

impl EmailSentDto {
    #[must_use]
    pub fn to(recipients: &[String]) -> Self {
        Self {
            success: true,
            message: format!("Email sent to {}", recipients.join(", ")),
        }
    }
}
