//! Call-forward DTOs.
//!
//! `delayStatus` and `delayDays` are computed at read time (domain-rules R1)
//! and returned alongside the stored fields, exactly as the legacy API did.

use pmk_domain::call_forward::{CallForwardInput, CallForwardItemWithDelay, CfStatus, ItemType};
use pmk_domain::ids::CallForwardItemId;
use pmk_domain::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallForwardDto {
    pub id: i32,
    pub job_id: i32,
    pub title: String,
    pub item_type: String,
    pub supplier_trade: Option<String>,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_start: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub status: String,
    pub notes: Option<String>,
    pub sort_order: i32,
    pub parent_id: Option<i32>,
    /// Computed, never stored.
    pub delay_status: String,
    /// Only ever set when `delayStatus` is `delayed`.
    pub delay_days: Option<i64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<CallForwardItemWithDelay> for CallForwardDto {
    fn from(w: CallForwardItemWithDelay) -> Self {
        let i = w.item;
        Self {
            id: i.id.get(),
            job_id: i.job_id.get(),
            title: i.title,
            item_type: i.item_type.as_str().to_string(),
            supplier_trade: i.supplier_trade,
            est_start: i.est_start,
            est_finish: i.est_finish,
            actual_start: i.actual_start,
            actual_finish: i.actual_finish,
            // The stored status; an unrecognised one reads back as
            // `not_started`, matching the legacy default.
            status: i.status.map_or("not_started", CfStatus::as_str).to_string(),
            notes: i.notes,
            sort_order: i.sort_order,
            parent_id: i.parent_id.map(CallForwardItemId::get),
            delay_status: w.delay.delay_status.as_str().to_string(),
            delay_days: w.delay.delay_days,
            created_at: i.created_at,
            updated_at: i.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallForwardRequest {
    pub job_id: Option<i32>,
    pub title: String,
    pub item_type: Option<String>,
    pub supplier_trade: Option<String>,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_start: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub status: Option<String>,
    pub notes: Option<String>,
    pub sort_order: Option<i32>,
    pub parent_id: Option<i32>,
    /// Index into the same list for bulk creates, so a hierarchy can be
    /// described before any ids exist.
    pub local_parent: Option<usize>,
}

impl CallForwardRequest {
    pub fn into_input(self) -> Result<(CallForwardInput, Option<usize>), DomainError> {
        let item_type = match self.item_type.as_deref() {
            None | Some("") => None,
            Some(t) => Some(ItemType::parse(t).ok_or_else(|| {
                DomainError::invalid("itemType", "must be one of HEADER, STAGE_CLAIM, TASK")
            })?),
        };
        let status = match self.status.as_deref() {
            None | Some("") => None,
            Some(s) => Some(CfStatus::parse(s).ok_or_else(|| {
                DomainError::invalid(
                    "status",
                    "must be one of not_started, in_progress, completed, on_hold",
                )
            })?),
        };
        Ok((
            CallForwardInput {
                title: self.title,
                item_type,
                supplier_trade: self.supplier_trade,
                est_start: self.est_start,
                est_finish: self.est_finish,
                actual_start: self.actual_start,
                actual_finish: self.actual_finish,
                status,
                notes: self.notes,
                sort_order: self.sort_order,
                parent_id: self.parent_id,
            },
            self.local_parent,
        ))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkCreateRequest {
    pub job_id: i32,
    pub items: Vec<CallForwardRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderItem {
    pub id: i32,
    pub sort_order: i32,
    /// Absent leaves parentage untouched; an explicit `null` detaches to root.
    #[serde(default, deserialize_with = "double_option")]
    pub parent_id: Option<Option<i32>>,
}

/// Distinguishes an absent key from an explicit JSON `null`.
fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderRequest {
    pub items: Vec<ReorderItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderResponse {
    pub updated: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallForwardListQuery {
    pub job_id: Option<i32>,
    pub status: Option<String>,
    pub parent_id: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingQuery {
    pub days: Option<i64>,
}

// ── templates ───────────────────────────────────────────────────────────────

use pmk_domain::call_forward::templates::{Template, TemplateItem};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateItemDto {
    pub local_id: i32,
    pub local_parent_id: Option<i32>,
    pub title: String,
    pub item_type: String,
    pub supplier_trade: Option<String>,
    pub sort_order: i32,
}

impl From<TemplateItem> for TemplateItemDto {
    fn from(i: TemplateItem) -> Self {
        Self {
            local_id: i.local_id,
            local_parent_id: i.local_parent_id,
            title: i.title,
            item_type: i.item_type,
            supplier_trade: i.supplier_trade,
            sort_order: i.sort_order,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateDto {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub items: Vec<TemplateItemDto>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Template> for TemplateDto {
    fn from(t: Template) -> Self {
        Self {
            id: t.id.get(),
            name: t.name,
            description: t.description,
            items: t.items.into_iter().map(Into::into).collect(),
            created_at: t.created_at,
            updated_at: t.updated_at,
        }
    }
}

/// A template is captured from a job, never authored by hand: that way it
/// always reflects a programme that really existed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTemplateRequest {
    pub name: String,
    pub description: Option<String>,
    pub job_id: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameTemplateRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyTemplateRequest {
    pub job_id: i32,
    /// Clears the job's existing programme first. Destructive, so it is off
    /// unless explicitly asked for.
    #[serde(default)]
    pub replace: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedDto {
    pub applied: usize,
}
