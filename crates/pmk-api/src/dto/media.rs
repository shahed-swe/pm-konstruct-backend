//! Media DTOs.
//!
//! Uploads are a two-step handshake: `prepare` returns presigned URLs, the
//! client PUTs directly to storage, then `confirm` records the rows after the
//! server has verified what actually arrived.

use pmk_domain::ids::{DiaryNoteId, MediaId, UserId};
use pmk_domain::media::UploadRequest;
use pmk_ports::repository::{Media, MediaOwner};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadFileRequest {
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

impl From<UploadFileRequest> for UploadRequest {
    fn from(r: UploadFileRequest) -> Self {
        Self {
            original_name: r.file_name,
            mime: r.mime_type,
            size_bytes: r.size_bytes,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadRequest {
    /// Attaches the upload to a specific note within the diary entry.
    pub note_id: Option<i32>,
    pub files: Vec<UploadFileRequest>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedUploadDto {
    /// PUT the bytes here. Expires; not stored anywhere.
    pub upload_url: String,
    pub expires_in_secs: u64,
    /// Echo this back to `confirm`.
    pub stored_name: String,
    pub original_name: String,
    pub mime_type: String,
}

impl From<pmk_app::media::PreparedUpload> for PreparedUploadDto {
    fn from(p: pmk_app::media::PreparedUpload) -> Self {
        Self {
            upload_url: p.upload_url,
            expires_in_secs: p.expires_in_secs,
            stored_name: p.stored_name,
            original_name: p.original_name,
            mime_type: p.mime_type,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmUploadRequest {
    pub note_id: Option<i32>,
    pub stored_name: String,
    pub original_name: String,
    pub mime_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDto {
    pub id: i32,
    pub diary_entry_id: Option<i32>,
    pub note_id: Option<i32>,
    pub job_id: Option<i32>,
    pub file_type: String,
    pub mime_type: String,
    pub original_name: String,
    pub stored_name: String,
    pub file_size: i64,
    pub uploaded_by: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<Media> for MediaDto {
    fn from(m: Media) -> Self {
        // The object key is deliberately not exposed: clients fetch a
        // short-lived signed URL from the download endpoint instead, so a URL
        // captured from a response cannot be replayed indefinitely.
        let (entry, note, job) = match m.owner {
            MediaOwner::Diary { entry, note } => {
                (Some(entry.get()), note.map(DiaryNoteId::get), None)
            }
            MediaOwner::Job { job } => (None, None, Some(job.get())),
        };
        Self {
            id: m.id.get(),
            diary_entry_id: entry,
            note_id: note,
            job_id: job,
            file_type: m.file_type.as_str().to_string(),
            mime_type: m.mime_type,
            original_name: m.original_name,
            stored_name: m.stored_name,
            file_size: m.file_size,
            uploaded_by: m.uploaded_by.map(UserId::get),
            created_at: m.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadUrlDto {
    pub url: String,
}

/// Identifies which table a media id belongs to.
///
/// `diary_media` and `job_media` have independent sequences, so an id alone is
/// ambiguous. The legacy API had separate routes per table; this keeps that
/// distinction explicit rather than guessing.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaScopeQuery {
    #[serde(default)]
    pub job_media: bool,
}

#[must_use]
pub fn media_id(raw: i32) -> MediaId {
    MediaId(raw)
}
