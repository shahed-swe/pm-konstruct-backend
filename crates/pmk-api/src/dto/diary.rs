//! Site diary DTOs.

use pmk_domain::diary::{
    ActionStatus, DiaryEntry, DiaryEntryInput, DiaryNote, DiaryNoteComment, DiaryNoteInput,
    NoteCategory,
};
use pmk_domain::ids::UserId;
use pmk_domain::DomainError;
use serde::{Deserialize, Serialize};

use crate::dto::weather::WeatherDto;

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
    /// Flattened onto the entry, the way the legacy sent it.
    ///
    /// The legacy read these through drizzle's `numeric`, which hands back
    /// strings, so its own reports service `parseFloat`ed every reading. We
    /// send the same numbers `/weather` sends, so a temperature has one type
    /// wherever it appears.
    #[serde(flatten)]
    pub weather_stamp: WeatherDto,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,

    /// The job's three names, so the client can label the entry by whichever
    /// one the company's branding selects. Sent by the legacy under these
    /// names; absent on a create or update reply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_address: Option<String>,

    /// Who wrote it. Null when the author's user row has been deleted; the
    /// entry itself stays, because it is a site record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,

    /// The first note, and how many there are.
    ///
    /// An entry written through the diary form leaves `workCompleted` empty
    /// and puts everything in its notes, so a list built from the entry's own
    /// fields alone shows "Diary entry" on every row -- which is what the
    /// legacy list did. These let it show the entry's actual first line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_count: Option<i64>,
}

impl From<pmk_app::diary::DiaryEntryView> for DiaryEntryDto {
    fn from(v: pmk_app::diary::DiaryEntryView) -> Self {
        Self {
            job_name: v.context.job_name,
            job_number: v.context.job_number,
            job_address: v.context.job_address,
            author_name: v.context.author_name,
            first_note: v.context.first_note,
            note_count: Some(v.context.note_count),
            ..Self::from(v.entry)
        }
    }
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
            weather_stamp: WeatherDto::from(e.weather_stamp),
            created_at: e.created_at,
            updated_at: e.updated_at,
            job_name: None,
            job_number: None,
            job_address: None,
            author_name: None,
            first_note: None,
            note_count: None,
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

    /// The weather where the supervisor is standing, captured by the browser
    /// when the entry is written.
    ///
    /// Sent by the client rather than looked up by the server, because the
    /// server has no way to know which site somebody is on -- a job's address
    /// is a postal address, not a set of coordinates, and a supervisor may be
    /// standing on any of several lots. The browser has the location; the
    /// server has the API key. Both are needed.
    ///
    /// Frozen once written (domain-rules R8): it is a record of the
    /// conditions that day, never refreshed.
    #[serde(flatten)]
    pub weather_stamp: WeatherStampRequest,
}

/// The reading the client captured, if it managed to.
///
/// Every field is optional: location can be refused, the weather service can
/// be unconfigured or down, and none of that may stop a supervisor filing
/// the day's diary.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherStampRequest {
    pub location_name: Option<String>,
    pub location_lat: Option<f64>,
    pub location_lng: Option<f64>,
    pub temperature: Option<f64>,
    pub weather_condition: Option<String>,
    pub weather_icon: Option<String>,
    pub wind_speed_kmh: Option<f64>,
    pub rainfall_mm: Option<f64>,
    pub sunrise_time: Option<String>,
    pub sunset_time: Option<String>,
}

impl WeatherStampRequest {
    /// Whether the client managed to capture anything at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.location_name.is_none()
            && self.temperature.is_none()
            && self.weather_condition.is_none()
            && self.location_lat.is_none()
    }

    /// The domain stamp. Decimals are stored, so the f64s convert here.
    #[must_use]
    pub fn into_stamp(self) -> pmk_domain::diary::WeatherStamp {
        use rust_decimal::prelude::FromPrimitive;
        let dec = |v: Option<f64>| v.and_then(rust_decimal::Decimal::from_f64);

        pmk_domain::diary::WeatherStamp {
            location_name: self.location_name,
            location_lat: dec(self.location_lat),
            location_lng: dec(self.location_lng),
            temperature: dec(self.temperature),
            weather_condition: self.weather_condition,
            weather_icon: self.weather_icon,
            wind_speed_kmh: dec(self.wind_speed_kmh),
            rainfall_mm: dec(self.rainfall_mm),
            sunrise_time: self.sunrise_time,
            sunset_time: self.sunset_time,
        }
    }
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

    /// Splits the body into the entry's own fields and its weather stamp.
    pub fn split(
        mut self,
    ) -> Result<(DiaryEntryInput, Option<pmk_domain::diary::WeatherStamp>), DomainError> {
        let captured = std::mem::take(&mut self.weather_stamp);
        let stamp = if captured.is_empty() {
            None
        } else {
            Some(captured.into_stamp())
        };
        Ok((self.into_input()?, stamp))
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
