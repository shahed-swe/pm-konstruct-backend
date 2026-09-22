//! Site diary: entries, notes, comment threads — domain-rules R8.

use crate::ids::{DiaryEntryId, DiaryNoteId, JobId, UserId};
use crate::{DomainError, DomainResult};

/// Categories a diary note can carry.
///
/// Derived from the production data (`general`, `client`, `trades`,
/// `site_conditions`, `issues`, `safety`, `materials`) plus `eto`, which only
/// appears in development but is created by the ETO form flow (R7).
/// Enforced by `diary_notes_category_check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteCategory {
    General,
    Client,
    Trades,
    SiteConditions,
    Issues,
    Safety,
    Materials,
    Eto,
}

impl NoteCategory {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "general" => Some(Self::General),
            "client" => Some(Self::Client),
            "trades" => Some(Self::Trades),
            "site_conditions" => Some(Self::SiteConditions),
            "issues" => Some(Self::Issues),
            "safety" => Some(Self::Safety),
            "materials" => Some(Self::Materials),
            "eto" => Some(Self::Eto),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Client => "client",
            Self::Trades => "trades",
            Self::SiteConditions => "site_conditions",
            Self::Issues => "issues",
            Self::Safety => "safety",
            Self::Materials => "materials",
            Self::Eto => "eto",
        }
    }

    pub const ALL: &'static str =
        "general, client, trades, site_conditions, issues, safety, materials, eto";
}

/// Action state on an entry or note. NULL means "not an action item", which is
/// the common case: 141 of 175 production entries have no action status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Action,
    Processing,
    Completed,
}

impl ActionStatus {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "action" => Some(Self::Action),
            "processing" => Some(Self::Processing),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Processing => "processing",
            Self::Completed => "completed",
        }
    }
}

/// Weather captured at entry creation and then frozen (domain-rules R8).
///
/// Never refreshed: it is a historical record of conditions on site that day,
/// not a live reading.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WeatherStamp {
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

#[derive(Debug, Clone, PartialEq)]
pub struct DiaryEntry {
    pub id: DiaryEntryId,
    pub job_id: JobId,
    pub date: chrono::NaiveDate,
    pub time: Option<String>,
    pub author_id: Option<UserId>,
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
    pub weather_stamp: WeatherStamp,
    pub action_status: Option<ActionStatus>,
    pub action_raised_by: Option<UserId>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct DiaryEntryInput {
    pub job_id: i32,
    pub date: Option<chrono::NaiveDate>,
    pub time: Option<String>,
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
    pub action_status: Option<ActionStatus>,
}

impl DiaryEntryInput {
    pub fn validate(&self) -> DomainResult<()> {
        if self.job_id <= 0 {
            return Err(DomainError::invalid("jobId", "is required"));
        }
        // `site_diary.work_completed` is NOT NULL with no default.
        if self.work_completed.trim().is_empty() {
            return Err(DomainError::invalid("workCompleted", "is required"));
        }
        if let Some(w) = self.workforce {
            if w < 0 {
                return Err(DomainError::invalid("workforce", "cannot be negative"));
            }
            if w > 10_000 {
                return Err(DomainError::invalid("workforce", "is implausibly large"));
            }
        }
        if let Some(t) = &self.time {
            if !t.trim().is_empty() && !looks_like_time(t) {
                return Err(DomainError::invalid("time", "must be HH:MM or HH:MM:SS"));
            }
        }
        Ok(())
    }
}

/// `site_diary.time` is free text in the schema, but the UI writes 24-hour
/// clock values and reports parse it, so it is validated rather than trusted.
fn looks_like_time(s: &str) -> bool {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return false;
    }
    let Ok(h) = parts[0].parse::<u32>() else {
        return false;
    };
    let Ok(m) = parts[1].parse::<u32>() else {
        return false;
    };
    let sec = match parts.get(2) {
        Some(p) => match p.parse::<u32>() {
            Ok(v) => v,
            Err(_) => return false,
        },
        None => 0,
    };
    h < 24 && m < 60 && sec < 60
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiaryNote {
    pub id: DiaryNoteId,
    pub diary_entry_id: DiaryEntryId,
    pub category: NoteCategory,
    pub content: String,
    pub action_status: Option<ActionStatus>,
    pub action_raised_by: Option<UserId>,
    pub sort_order: i32,
    pub archived: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct DiaryNoteInput {
    pub category: NoteCategory,
    pub content: String,
    pub action_status: Option<ActionStatus>,
    pub sort_order: Option<i32>,
}

impl DiaryNoteInput {
    pub fn validate(&self) -> DomainResult<()> {
        if self.content.trim().is_empty() {
            return Err(DomainError::invalid("content", "is required"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiaryNoteComment {
    pub id: i32,
    pub note_id: DiaryNoteId,
    pub author_id: Option<UserId>,
    pub author_name: Option<String>,
    pub content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> DiaryEntryInput {
        DiaryEntryInput {
            job_id: 1,
            work_completed: "Poured slab".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_minimal_entry_validates() {
        assert!(entry().validate().is_ok());
    }

    #[test]
    fn work_completed_is_required() {
        let mut e = entry();
        e.work_completed = "   ".into();
        match e.validate() {
            Err(DomainError::Invalid { field, .. }) => assert_eq!(field, "workCompleted"),
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn job_id_is_required() {
        let mut e = entry();
        e.job_id = 0;
        assert!(e.validate().is_err());
    }

    #[test]
    fn workforce_bounds_are_enforced() {
        let mut e = entry();
        e.workforce = Some(-1);
        assert!(e.validate().is_err(), "negative workforce");
        e.workforce = Some(99_999);
        assert!(e.validate().is_err(), "implausible workforce");
        e.workforce = Some(0);
        assert!(e.validate().is_ok(), "zero is a legitimate rest day");
        e.workforce = Some(42);
        assert!(e.validate().is_ok());
    }

    #[test]
    fn time_must_be_a_clock_value() {
        let mut e = entry();
        for bad in ["25:00", "12:60", "noon", "1", "12:30:61", "12:aa"] {
            e.time = Some(bad.into());
            assert!(e.validate().is_err(), "{bad} should be rejected");
        }
        for good in ["00:00", "07:30", "23:59", "13:45:30"] {
            e.time = Some(good.into());
            assert!(e.validate().is_ok(), "{good} should be accepted");
        }
        e.time = Some("  ".into());
        assert!(e.validate().is_ok(), "blank is treated as absent");
    }

    #[test]
    fn note_categories_round_trip_including_eto() {
        for s in [
            "general",
            "client",
            "trades",
            "site_conditions",
            "issues",
            "safety",
            "materials",
            "eto",
        ] {
            assert_eq!(NoteCategory::parse(s).map(NoteCategory::as_str), Some(s));
        }
        assert_eq!(NoteCategory::parse("random"), None);
    }

    #[test]
    fn action_statuses_round_trip() {
        for s in ["action", "processing", "completed"] {
            assert_eq!(ActionStatus::parse(s).map(ActionStatus::as_str), Some(s));
        }
        assert_eq!(ActionStatus::parse("done"), None);
    }

    #[test]
    fn note_content_is_required() {
        let n = DiaryNoteInput {
            category: NoteCategory::General,
            content: " ".into(),
            action_status: None,
            sort_order: None,
        };
        assert!(n.validate().is_err());
    }
}
