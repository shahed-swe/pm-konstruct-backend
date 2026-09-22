//! Forms request and response shapes.

use pmk_domain::forms::{EtoInput, InspectionDraftInput, InspectionItemInput};
use pmk_domain::ids::JobId;
use pmk_ports::repository::{
    EtoListItem, EtoRaised, InspectionForm, InspectionItem, InspectionPhoto,
};
use serde::{Deserialize, Serialize};

// ── extras to order ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EtoRequest {
    pub job_id: i32,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub details: String,
}

impl From<EtoRequest> for EtoInput {
    fn from(r: EtoRequest) -> Self {
        Self {
            job_id: JobId(r.job_id),
            reason: r.reason,
            details: r.details,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EtoRaisedDto {
    pub entry_id: i32,
    pub note_id: i32,
    pub eto_number: i32,
    pub po_number: String,
    pub raised_by: String,
    /// Always `action`: a new ETO is pending manager approval by definition.
    pub status: &'static str,
}

impl From<EtoRaised> for EtoRaisedDto {
    fn from(e: EtoRaised) -> Self {
        Self {
            entry_id: e.entry_id.get(),
            note_id: e.note_id.get(),
            eto_number: e.eto_number,
            po_number: e.po_number,
            raised_by: e.raised_by,
            status: "action",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EtoListItemDto {
    pub note_id: i32,
    pub entry_id: i32,
    pub eto_number: String,
    pub content: String,
    pub raised_by: String,
    pub diary_date: chrono::NaiveDate,
    pub approved_at: chrono::DateTime<chrono::Utc>,
    pub job_number: Option<String>,
    pub job_name: Option<String>,
    pub job_address: Option<String>,
}

impl From<EtoListItem> for EtoListItemDto {
    fn from(e: EtoListItem) -> Self {
        Self {
            note_id: e.note_id.get(),
            entry_id: e.entry_id.get(),
            eto_number: e.eto_number,
            content: e.content,
            raised_by: e.raised_by,
            diary_date: e.diary_date,
            approved_at: e.approved_at,
            job_number: e.job_number,
            job_name: e.job_name,
            job_address: e.job_address,
        }
    }
}

// ── property inspection ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRequest {
    pub job_id: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionItemRequest {
    pub client_key: String,
    #[serde(default)]
    pub room: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub actioned: bool,
    #[serde(default)]
    pub sort_order: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionDraftRequest {
    #[serde(default)]
    pub inspector: String,
    #[serde(default)]
    pub inspection_type: String,
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub observations: String,
    #[serde(default)]
    pub weather_data: Option<serde_json::Value>,
    /// The revision the client last read. Required -- defaulting it to 0 would
    /// turn every stale save into a silent overwrite of the first one.
    pub revision: i32,
    #[serde(default)]
    pub items: Vec<InspectionItemRequest>,
}

impl From<InspectionDraftRequest> for InspectionDraftInput {
    fn from(r: InspectionDraftRequest) -> Self {
        Self {
            inspector: r.inspector,
            inspection_type: r.inspection_type,
            stage: r.stage,
            observations: r.observations,
            weather_data: r.weather_data,
            revision: r.revision,
            items: r
                .items
                .into_iter()
                .map(|i| InspectionItemInput {
                    client_key: i.client_key,
                    room: i.room,
                    description: i.description,
                    actioned: i.actioned,
                    sort_order: i.sort_order,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionPhotoDto {
    pub id: i32,
    pub item_id: i32,
    pub diary_media_id: i32,
    pub sort_order: i32,
    pub original_name: String,
    pub mime_type: String,
    pub file_size: i64,
    pub url: String,
}

impl From<InspectionPhoto> for InspectionPhotoDto {
    fn from(p: InspectionPhoto) -> Self {
        Self {
            id: p.id,
            item_id: p.item_id.get(),
            diary_media_id: p.diary_media_id.get(),
            sort_order: p.sort_order,
            original_name: p.original_name,
            mime_type: p.mime_type,
            file_size: p.file_size,
            url: p.url,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionItemDto {
    pub id: i32,
    pub client_key: String,
    pub room: String,
    pub description: String,
    pub actioned: bool,
    pub sort_order: i32,
    pub photos: Vec<InspectionPhotoDto>,
}

impl From<InspectionItem> for InspectionItemDto {
    fn from(i: InspectionItem) -> Self {
        Self {
            id: i.id.get(),
            client_key: i.client_key,
            room: i.room,
            description: i.description,
            actioned: i.actioned,
            sort_order: i.sort_order,
            photos: i.photos.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionFormDto {
    pub id: i32,
    pub company_id: i32,
    pub job_id: i32,
    pub created_by: i32,
    pub diary_entry_id: i32,
    pub diary_note_id: i32,
    pub inspection_date: chrono::NaiveDate,
    pub inspector: String,
    pub inspection_type: String,
    pub stage: String,
    pub observations: String,
    pub weather_data: Option<serde_json::Value>,
    pub revision: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub items: Vec<InspectionItemDto>,
}

impl From<InspectionForm> for InspectionFormDto {
    fn from(f: InspectionForm) -> Self {
        Self {
            id: f.id.get(),
            company_id: f.company_id,
            job_id: f.job_id.get(),
            created_by: f.created_by.get(),
            diary_entry_id: f.diary_entry_id.get(),
            diary_note_id: f.diary_note_id.get(),
            inspection_date: f.inspection_date,
            inspector: f.inspector,
            inspection_type: f.inspection_type,
            stage: f.stage,
            observations: f.observations,
            weather_data: f.weather_data,
            revision: f.revision,
            created_at: f.created_at,
            updated_at: f.updated_at,
            items: f.items.into_iter().map(Into::into).collect(),
        }
    }
}

// ── inspection photo upload ─────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedInspectionUploadDto {
    pub stored_name: String,
    pub upload_url: String,
    pub expires_in_secs: u64,
    pub max_bytes: u64,
}

impl From<pmk_app::forms::PreparedInspectionUpload> for PreparedInspectionUploadDto {
    fn from(p: pmk_app::forms::PreparedInspectionUpload) -> Self {
        Self {
            stored_name: p.stored_name,
            upload_url: p.upload_url,
            expires_in_secs: p.expires_in_secs,
            max_bytes: p.max_bytes,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmInspectionPhoto {
    pub stored_name: String,
    pub original_name: String,
    pub mime_type: String,
}

impl From<ConfirmInspectionPhoto> for pmk_app::forms::InspectionPhotoUpload {
    fn from(c: ConfirmInspectionPhoto) -> Self {
        Self {
            stored_name: c.stored_name,
            original_name: c.original_name,
            mime_type: c.mime_type,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmInspectionPhotos {
    pub files: Vec<ConfirmInspectionPhoto>,
}
