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
use pmk_domain::media::{object_key, FileType};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::{
    BulkDeletePlan, JobFile, Media, MediaOwner, MediaRecord, MediaRepository, MediaSelection,
};
use pmk_ports::PortResult;
use sqlx::{PgPool, Postgres, Row, Transaction};

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
            owner: MediaOwner::Job {
                job: JobId(r.job_id),
            },
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

    /// Queues objects for removal from storage.
    ///
    /// Takes **object keys**, not stored names. The object lives at
    /// `{company}/{entity}/{entity_id}/{stored_name}`, and queueing the bare
    /// filename asks storage to delete a key that has never existed -- which
    /// it reports as success, so the queue drains and the object stays.
    /// See migration 0015.
    ///
    /// Runs in the same transaction as the row delete, so a committed delete
    /// always has a corresponding sweep entry.
    async fn enqueue_deletions(
        tx: &mut Transaction<'_, Postgres>,
        object_keys: &[String],
    ) -> PortResult<()> {
        if object_keys.is_empty() {
            return Ok(());
        }
        // `ON CONFLICT DO NOTHING` without a conflict target, deliberately.
        // Naming the target (`ON CONFLICT (object_key)`) makes Postgres
        // inspect the conflicting row, which requires SELECT on the table --
        // and granting the API SELECT here would let it enumerate every
        // tenant's object names. The untargeted form needs only INSERT and
        // swallows the duplicate just the same.
        sqlx::query(
            "INSERT INTO media_deletion_queue (object_key) \
             SELECT unnest($1::text[]) ON CONFLICT DO NOTHING",
        )
        .bind(object_keys)
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
        let sql =
            format!("SELECT {DIARY_COLS} FROM diary_media WHERE diary_entry_id = $1 ORDER BY id");
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
        let table = if job_media {
            "job_media"
        } else {
            "diary_media"
        };

        // RETURNING gives the stored name and the owner id, so the object key
        // can be built and queued in the same transaction; a separate SELECT
        // would race with a concurrent delete and could leave the object
        // orphaned in storage.
        let (owner_column, entity) = if job_media {
            ("job_id", "job")
        } else {
            ("diary_entry_id", "diary")
        };
        let deleted: Option<(String, i32)> = sqlx::query_as(&format!(
            "DELETE FROM {table} WHERE id = $1 RETURNING stored_name, {owner_column}"
        ))
        .bind(id.get())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let Some((stored_name, owner_id)) = deleted else {
            tx.commit().await.map_err(map_sqlx)?;
            return Ok(false);
        };

        let key = object_key(scope.company_id().get(), entity, owner_id, &stored_name);
        Self::enqueue_deletions(&mut tx, std::slice::from_ref(&key)).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(true)
    }

    async fn pending_deletions(&self, limit: i64) -> PortResult<Vec<String>> {
        // Runs in the worker, which is cross-tenant by design: the queue keys
        // on the object key alone and has no tenant column. RLS denies pmk_app
        // this table entirely (migration 0004).
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT object_key FROM media_deletion_queue \
             WHERE attempts < 10 ORDER BY created_at LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(rows.into_iter().map(|(n,)| n).collect())
    }

    async fn mark_deleted(&self, object_key: &str) -> PortResult<()> {
        sqlx::query("DELETE FROM media_deletion_queue WHERE object_key = $1")
            .bind(object_key)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn job_files(&self, scope: TenantScope, job: JobId) -> PortResult<Vec<JobFile>> {
        let mut tx = self.begin(scope).await?;
        // The UNION is the legacy shape: the Files tab shows documents and
        // inspection drafts together, newest first, and the drafts are
        // synthesised rather than stored as files.
        let rows = sqlx::query(
            "SELECT jm.id, jm.file_type, jm.mime_type, jm.original_name, jm.stored_name, \
                    jm.file_size::bigint AS file_size, jm.url, jm.created_at, \
                    u.name AS uploader_name \
             FROM job_media jm \
             LEFT JOIN users u ON u.id = jm.uploaded_by \
             WHERE jm.job_id = $1 AND jm.file_type = 'document' \
             UNION ALL \
             SELECT f.id, 'form', 'application/x-pm-konstruct-form', \
                    'Site Inspection — ' || to_char(f.inspection_date, 'DD Mon YYYY') || ' (Draft)', \
                    'inspection-form:' || f.id, 0::bigint, \
                    '/forms/property-inspection?jobId=' || f.job_id, f.created_at, \
                    u.name \
             FROM inspection_forms f \
             LEFT JOIN users u ON u.id = f.created_by \
             WHERE f.job_id = $1 \
             ORDER BY created_at DESC",
        )
        .bind(job.get())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| JobFile {
                id: r.get("id"),
                file_type: r.get("file_type"),
                mime_type: r.get("mime_type"),
                original_name: r.get("original_name"),
                stored_name: r.get("stored_name"),
                file_size: r.get("file_size"),
                url: r.get("url"),
                created_at: r.get("created_at"),
                uploader_name: r.get("uploader_name"),
            })
            .collect())
    }

    async fn delete_many(
        &self,
        scope: TenantScope,
        job: JobId,
        selections: &[MediaSelection],
        uploaded_by: Option<UserId>,
    ) -> PortResult<BulkDeletePlan> {
        let mut plan = BulkDeletePlan::default();
        if selections.is_empty() {
            return Ok(plan);
        }

        let job_ids: Vec<i32> = selections
            .iter()
            .filter(|s| s.job_media)
            .map(|s| s.id.get())
            .collect();
        let diary_ids: Vec<i32> = selections
            .iter()
            .filter(|s| !s.job_media)
            .map(|s| s.id.get())
            .collect();

        let mut tx = self.begin(scope).await?;

        // FOR UPDATE: two people clearing the same gallery must not both see
        // the rows and both try to delete them.
        let job_rows = sqlx::query(
            "SELECT id, stored_name, uploaded_by, job_id FROM job_media \
             WHERE job_id = $1 AND id = ANY($2) FOR UPDATE",
        )
        .bind(job.get())
        .bind(&job_ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        // Diary media reaches the job through its entry, so the join is what
        // stops an id from another job being deleted by guessing.
        let diary_rows = sqlx::query(
            "SELECT dm.id, dm.stored_name, dm.uploaded_by, dm.diary_entry_id FROM diary_media dm \
             JOIN site_diary sd ON sd.id = dm.diary_entry_id \
             WHERE sd.job_id = $1 AND dm.id = ANY($2) FOR UPDATE",
        )
        .bind(job.get())
        .bind(&diary_ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let found: std::collections::HashMap<MediaSelection, Option<i32>> = job_rows
            .iter()
            .map(|r| {
                (
                    MediaSelection {
                        job_media: true,
                        id: MediaId(r.get("id")),
                    },
                    r.get::<Option<i32>, _>("uploaded_by"),
                )
            })
            .chain(diary_rows.iter().map(|r| {
                (
                    MediaSelection {
                        job_media: false,
                        id: MediaId(r.get("id")),
                    },
                    r.get::<Option<i32>, _>("uploaded_by"),
                )
            }))
            .collect();

        for sel in selections {
            match found.get(sel) {
                None => plan.missing.push(*sel),
                Some(owner) => {
                    if uploaded_by.is_some_and(|u| *owner != Some(u.get())) {
                        plan.forbidden.push(*sel);
                    }
                }
            }
        }

        // All or nothing: a partial delete would leave the user unable to tell
        // which of their selections survived.
        if !plan.missing.is_empty() || !plan.forbidden.is_empty() {
            tx.rollback().await.map_err(map_sqlx)?;
            return Ok(plan);
        }

        // Object keys, not stored names: the object lives under a
        // company/entity prefix and the bare filename names nothing.
        let company = scope.company_id().get();
        let mut keys: Vec<String> = Vec::with_capacity(selections.len());
        for r in &job_rows {
            let stored: String = r.get("stored_name");
            keys.push(object_key(company, "job", r.get("job_id"), &stored));
        }
        for r in &diary_rows {
            let stored: String = r.get("stored_name");
            keys.push(object_key(
                company,
                "diary",
                r.get("diary_entry_id"),
                &stored,
            ));
        }

        if !job_ids.is_empty() {
            sqlx::query("DELETE FROM job_media WHERE job_id = $1 AND id = ANY($2)")
                .bind(job.get())
                .bind(&job_ids)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        }
        if !diary_ids.is_empty() {
            sqlx::query(
                "DELETE FROM diary_media WHERE id = ANY($1) AND diary_entry_id IN \
                 (SELECT id FROM site_diary WHERE job_id = $2)",
            )
            .bind(&diary_ids)
            .bind(job.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        // Queued in the same transaction as the deletes, so a commit always
        // leaves the objects reachable by the sweeper.
        Self::enqueue_deletions(&mut tx, &keys).await?;

        tx.commit().await.map_err(map_sqlx)?;
        plan.deleted = selections.to_vec();
        Ok(plan)
    }

    async fn mark_deletion_failed(&self, object_key: &str, error: &str) -> PortResult<()> {
        // Attempts are capped at 10 by `pending_deletions`, so a permanently
        // failing object stops being retried but stays visible for triage.
        sqlx::query(
            "UPDATE media_deletion_queue \
             SET attempts = attempts + 1, last_error = $2, last_attempt_at = NOW() \
             WHERE object_key = $1",
        )
        .bind(object_key)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }
}
