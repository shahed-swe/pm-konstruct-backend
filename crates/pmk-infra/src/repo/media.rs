//! `MediaRepository` over Postgres.
//!
//! Two tables with near-identical shapes: `diary_media` hangs off a diary entry
//! (and optionally a note), `job_media` hangs directly off a job. The legacy
//! code kept them separate so importing a photo to a job did not require a
//! synthetic diary entry, and that split is preserved.
//!
//! Both are tenant-scoped through their parent by the RLS policies in 0004.

use async_trait::async_trait;
use pmk_domain::ids::{DiaryEntryId, DiaryNoteId, JobId, MediaId, UserId};
use pmk_domain::media::FileType;
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::{Media, MediaOwner, MediaRecord, MediaRepository};
use pmk_ports::PortResult;
use sqlx::{PgPool, Postgres, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

#[derive(Debug, sqlx::FromRow)]
struct DiaryMediaRow {
    id: i32,
    diary_entry_id: i32,
    note_id: Option<i32>,
    file_type: String,
    mime_type: String,
    original_name: String,
    stored_name: String,
    file_size: i32,
    url: String,
    uploaded_by: Option<i32>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct JobMediaRow {
    id: i32,
    job_id: i32,
    file_type: String,
    mime_type: String,
    original_name: String,
    stored_name: String,
    file_size: i32,
    url: String,
    uploaded_by: Option<i32>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<DiaryMediaRow> for Media {
    fn from(r: DiaryMediaRow) -> Self {
        Self {
            id: MediaId(r.id),
            owner: MediaOwner::Diary {
                entry: DiaryEntryId(r.diary_entry_id),
                note: r.note_id.map(DiaryNoteId),
            },
            // Constrained by `diary_media_file_type_check`; default rather than
            // fail an entire gallery on one corrupt row.
            file_type: FileType::parse(&r.file_type).unwrap_or(FileType::Document),
            mime_type: r.mime_type,
            original_name: r.original_name,
            stored_name: r.stored_name,
            file_size: i64::from(r.file_size),
            url: r.url,
            uploaded_by: r.uploaded_by.map(UserId),
            created_at: r.created_at,
        }
    }
}

impl From<JobMediaRow> for Media {
    fn from(r: JobMediaRow) -> Self {
        Self {
            id: MediaId(r.id),
            owner: MediaOwner::Job { job: JobId(r.job_id) },
            file_type: FileType::parse(&r.file_type).unwrap_or(FileType::Document),
            mime_type: r.mime_type,
            original_name: r.original_name,
            stored_name: r.stored_name,
            file_size: i64::from(r.file_size),
            url: r.url,
            uploaded_by: r.uploaded_by.map(UserId),
            created_at: r.created_at,
        }
    }
}

const DIARY_COLS: &str = "id, diary_entry_id, note_id, file_type, mime_type, original_name, \
     stored_name, file_size, url, uploaded_by, created_at";
const JOB_COLS: &str = "id, job_id, file_type, mime_type, original_name, stored_name, \
     file_size, url, uploaded_by, created_at";

#[derive(Debug, Clone)]
pub struct PgMediaRepository {
    pool: PgPool,
}

impl PgMediaRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin(&self, scope: TenantScope) -> PortResult<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        Ok(tx)
    }

    /// Queues an object for removal from storage.
    ///
    /// Runs in the same transaction as the row delete, so a committed delete
    /// always has a corresponding sweep entry. `ON CONFLICT DO NOTHING` because
    /// `uq_media_deletion_queue_stored_name` is unique and a re-delete is not
    /// an error.
    async fn enqueue_deletion(
        tx: &mut Transaction<'_, Postgres>,
        stored_name: &str,
    ) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO media_deletion_queue (stored_name) VALUES ($1) \
             ON CONFLICT (stored_name) DO NOTHING",
        )
        .bind(stored_name)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }
}

#[async_trait]
impl MediaRepository for PgMediaRepository {
    async fn list_for_diary(
        &self,
        scope: TenantScope,
        entry: DiaryEntryId,
    ) -> PortResult<Vec<Media>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "SELECT {DIARY_COLS} FROM diary_media WHERE diary_entry_id = $1 ORDER BY id"
        );
        let rows: Vec<DiaryMediaRow> = sqlx::query_as(&sql)
            .bind(entry.get())
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_for_job(&self, scope: TenantScope, job: JobId) -> PortResult<Vec<Media>> {
        let mut tx = self.begin(scope).await?;
        // Matches `idx_job_media_job_created`.
        let sql = format!(
            "SELECT {JOB_COLS} FROM job_media WHERE job_id = $1 ORDER BY created_at DESC, id DESC"
        );
        let rows: Vec<JobMediaRow> = sqlx::query_as(&sql)
            .bind(job.get())
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn find(
        &self,
        scope: TenantScope,
        id: MediaId,
        job_media: bool,
    ) -> PortResult<Option<Media>> {
        let mut tx = self.begin(scope).await?;
        let media = if job_media {
            let sql = format!("SELECT {JOB_COLS} FROM job_media WHERE id = $1");
            let row: Option<JobMediaRow> = sqlx::query_as(&sql)
                .bind(id.get())
                .fetch_optional(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            row.map(Into::into)
        } else {
            let sql = format!("SELECT {DIARY_COLS} FROM diary_media WHERE id = $1");
            let row: Option<DiaryMediaRow> = sqlx::query_as(&sql)
                .bind(id.get())
                .fetch_optional(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            row.map(Into::into)
        };
        tx.commit().await.map_err(map_sqlx)?;
        Ok(media)
    }

    async fn record(&self, scope: TenantScope, rec: &MediaRecord) -> PortResult<Media> {
        let mut tx = self.begin(scope).await?;
        // `file_size` is `integer` in both tables, so anything over 2 GB would
        // wrap. The 500 MB video ceiling keeps it in range, and this makes the
        // truncation explicit rather than silent.
        let size = i32::try_from(rec.file_size).unwrap_or(i32::MAX);

        let media: Media = match rec.owner {
            MediaOwner::Diary { entry, note } => {
                let sql = format!(
                    "INSERT INTO diary_media (diary_entry_id, note_id, file_type, mime_type, \
                        original_name, stored_name, file_size, url, uploaded_by) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING {DIARY_COLS}"
                );
                let row: DiaryMediaRow = sqlx::query_as(&sql)
                    .bind(entry.get())
                    .bind(note.map(DiaryNoteId::get))
                    .bind(rec.file_type.as_str())
                    .bind(&rec.mime_type)
                    .bind(&rec.original_name)
                    .bind(&rec.stored_name)
                    .bind(size)
                    .bind(&rec.url)
                    .bind(rec.uploaded_by.get())
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(map_sqlx)?;
                row.into()
            }
            MediaOwner::Job { job } => {
                let sql = format!(
                    "INSERT INTO job_media (job_id, file_type, mime_type, original_name, \
                        stored_name, file_size, url, uploaded_by, source) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'upload') RETURNING {JOB_COLS}"
                );
                let row: JobMediaRow = sqlx::query_as(&sql)
                    .bind(job.get())
                    .bind(rec.file_type.as_str())
                    .bind(&rec.mime_type)
                    .bind(&rec.original_name)
                    .bind(&rec.stored_name)
                    .bind(size)
                    .bind(&rec.url)
                    .bind(rec.uploaded_by.get())
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(map_sqlx)?;
                row.into()
            }
        };

        tx.commit().await.map_err(map_sqlx)?;
        Ok(media)
    }

    async fn delete(&self, scope: TenantScope, id: MediaId, job_media: bool) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let table = if job_media { "job_media" } else { "diary_media" };

        // RETURNING gives the stored name so the object can be queued in the
        // same transaction; a separate SELECT would race with a concurrent
        // delete and could leave the object orphaned in storage.
        let deleted: Option<(String,)> = sqlx::query_as(&format!(
            "DELETE FROM {table} WHERE id = $1 RETURNING stored_name"
        ))
        .bind(id.get())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let Some((stored_name,)) = deleted else {
            tx.commit().await.map_err(map_sqlx)?;
            return Ok(false);
        };

        Self::enqueue_deletion(&mut tx, &stored_name).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(true)
    }

    async fn pending_deletions(&self, limit: i64) -> PortResult<Vec<String>> {
        // Runs in the worker, which is cross-tenant by design: the queue keys
        // on stored_name alone and has no tenant column. RLS denies pmk_app
        // this table entirely (migration 0004).
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT stored_name FROM media_deletion_queue \
             WHERE attempts < 10 ORDER BY created_at LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(rows.into_iter().map(|(n,)| n).collect())
    }

    async fn mark_deleted(&self, stored_name: &str) -> PortResult<()> {
        sqlx::query("DELETE FROM media_deletion_queue WHERE stored_name = $1")
            .bind(stored_name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn mark_deletion_failed(&self, stored_name: &str, error: &str) -> PortResult<()> {
        // Attempts are capped at 10 by `pending_deletions`, so a permanently
        // failing object stops being retried but stays visible for triage.
        sqlx::query(
            "UPDATE media_deletion_queue \
             SET attempts = attempts + 1, last_error = $2, last_attempt_at = NOW() \
             WHERE stored_name = $1",
        )
        .bind(stored_name)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }
}
