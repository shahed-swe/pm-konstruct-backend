//! `FormsRepository` over Postgres.
//!
//! Two features share this file because they share a shape: both write a
//! site-diary entry plus a categorised note, and keep their structured state
//! in a side table. Neither `eto_job_sequences` nor `inspection_forms` is
//! reachable without the parent job, so RLS scopes both through `jobs`.

use async_trait::async_trait;
use pmk_domain::forms::{
    eto_entry_summary, eto_note, parse_eto_number, parse_eto_raised_by, EtoInput, EtoNumber,
    InspectionDraftInput, JobHeader,
};
use pmk_domain::ids::{
    DiaryEntryId, DiaryNoteId, InspectionFormId, InspectionFormItemId, JobId, MediaId, UserId,
};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::{
    EtoListItem, EtoRaised, FormsRepository, InspectionForm, InspectionItem, InspectionPhoto,
    InspectionPhotoRecord, RenderedInspection,
};
use pmk_ports::{PortError, PortResult};
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::collections::HashMap;

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

#[derive(Debug, Clone)]
pub struct PgFormsRepository {
    pool: PgPool,
}

impl PgFormsRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin(&self, scope: TenantScope) -> PortResult<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        Ok(tx)
    }

    /// Loads a form the caller owns, with its items and photos.
    ///
    /// Ownership is part of the WHERE clause rather than a check afterwards:
    /// a draft belongs to the person who started it, and another supervisor
    /// asking for it by id must get the same 404 as a stranger.
    async fn load(
        tx: &mut Transaction<'_, Postgres>,
        id: InspectionFormId,
        user: UserId,
    ) -> PortResult<Option<InspectionForm>> {
        let row = sqlx::query(
            "SELECT id, company_id, job_id, created_by, diary_entry_id, diary_note_id, \
                    inspection_date, inspector, inspection_type, stage, observations, \
                    weather_data, revision, created_at, updated_at \
             FROM inspection_forms WHERE id = $1 AND created_by = $2",
        )
        .bind(id.get())
        .bind(user.get())
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_sqlx)?;

        let Some(row) = row else { return Ok(None) };

        let items = sqlx::query(
            "SELECT id, client_key, room, description, actioned, sort_order \
             FROM inspection_form_items WHERE form_id = $1 ORDER BY sort_order, id",
        )
        .bind(id.get())
        .fetch_all(&mut **tx)
        .await
        .map_err(map_sqlx)?;

        // One join rather than a query per item: a 40-row inspection with
        // photos on each would otherwise be 40 round-trips.
        let photos = sqlx::query(
            "SELECT m.id, m.item_id, m.diary_media_id, m.sort_order, \
                    dm.original_name, dm.mime_type, dm.file_size, dm.url \
             FROM inspection_form_media m \
             JOIN diary_media dm ON dm.id = m.diary_media_id \
             WHERE m.form_id = $1 ORDER BY m.sort_order, m.id",
        )
        .bind(id.get())
        .fetch_all(&mut **tx)
        .await
        .map_err(map_sqlx)?;

        let mut by_item: HashMap<i32, Vec<InspectionPhoto>> = HashMap::new();
        for p in photos {
            let item_id: i32 = p.get("item_id");
            by_item.entry(item_id).or_default().push(InspectionPhoto {
                id: p.get("id"),
                item_id: InspectionFormItemId(item_id),
                diary_media_id: MediaId(p.get("diary_media_id")),
                sort_order: p.get("sort_order"),
                original_name: p.get("original_name"),
                mime_type: p.get("mime_type"),
                file_size: i64::from(p.get::<i32, _>("file_size")),
                url: p.get("url"),
            });
        }

        Ok(Some(InspectionForm {
            id: InspectionFormId(row.get("id")),
            company_id: row.get("company_id"),
            job_id: JobId(row.get("job_id")),
            created_by: UserId(row.get("created_by")),
            diary_entry_id: DiaryEntryId(row.get("diary_entry_id")),
            diary_note_id: DiaryNoteId(row.get("diary_note_id")),
            inspection_date: row.get("inspection_date"),
            inspector: row.get("inspector"),
            inspection_type: row.get("inspection_type"),
            stage: row.get("stage"),
            observations: row.get("observations"),
            weather_data: row.get("weather_data"),
            revision: row.get("revision"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            items: items
                .into_iter()
                .map(|i| {
                    let item_id: i32 = i.get("id");
                    InspectionItem {
                        id: InspectionFormItemId(item_id),
                        client_key: i.get("client_key"),
                        room: i.get("room"),
                        description: i.get("description"),
                        actioned: i.get("actioned"),
                        sort_order: i.get("sort_order"),
                        photos: by_item.remove(&item_id).unwrap_or_default(),
                    }
                })
                .collect(),
        }))
    }

    /// Bumps `revision`, but only from `expected`.
    ///
    /// This is the whole concurrency control: the UPDATE matching zero rows
    /// *is* the conflict, so two clients saving the same draft cannot both
    /// succeed no matter how their transactions interleave.
    async fn claim_revision(
        tx: &mut Transaction<'_, Postgres>,
        id: InspectionFormId,
        expected: i32,
    ) -> PortResult<bool> {
        let claimed = sqlx::query(
            "UPDATE inspection_forms SET revision = revision + 1, updated_at = NOW() \
             WHERE id = $1 AND revision = $2 RETURNING id",
        )
        .bind(id.get())
        .bind(expected)
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_sqlx)?;
        Ok(claimed.is_some())
    }

    /// Deletes diary media rows and queues their objects for sweeping.
    ///
    /// Same transaction as the row delete, so a committed delete always has a
    /// matching queue entry and no object is orphaned in storage.
    async fn detach_media(
        tx: &mut Transaction<'_, Postgres>,
        company_id: i32,
        media_ids: &[i32],
    ) -> PortResult<Vec<String>> {
        if media_ids.is_empty() {
            return Ok(Vec::new());
        }
        // The entry id comes back with the name because the object lives
        // under `{company}/diary/{entry}/`, and the bare filename names
        // nothing in the bucket -- see migration 0015.
        let rows: Vec<(String, i32)> = sqlx::query_as(
            "DELETE FROM diary_media WHERE id = ANY($1) \
             RETURNING stored_name, diary_entry_id",
        )
        .bind(media_ids)
        .fetch_all(&mut **tx)
        .await
        .map_err(map_sqlx)?;

        let keys: Vec<String> = rows
            .iter()
            .map(|(stored, entry)| {
                pmk_domain::media::object_key(company_id, "diary", *entry, stored)
            })
            .collect();

        if !keys.is_empty() {
            // Untargeted ON CONFLICT: naming the target would make Postgres
            // read the conflicting row, which needs SELECT on a table the API
            // role is deliberately denied.
            sqlx::query(
                "INSERT INTO media_deletion_queue (object_key) \
                 SELECT unnest($1::text[]) ON CONFLICT DO NOTHING",
            )
            .bind(&keys)
            .execute(&mut **tx)
            .await
            .map_err(map_sqlx)?;
        }
        Ok(rows.into_iter().map(|(stored, _)| stored).collect())
    }
}

#[async_trait]
impl FormsRepository for PgFormsRepository {
    async fn raise_eto(
        &self,
        scope: TenantScope,
        author: UserId,
        author_name: &str,
        input: &EtoInput,
        stamp: (chrono::NaiveDate, &str),
    ) -> PortResult<EtoRaised> {
        let mut tx = self.begin(scope).await?;

        let job = sqlx::query("SELECT job_number, name, address FROM jobs WHERE id = $1")
            .bind(input.job_id.get())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        let Some(job) = job else {
            return Err(PortError::NotFound);
        };
        let header = JobHeader {
            job_number: job
                .try_get::<Option<String>, _>("job_number")
                .ok()
                .flatten()
                .unwrap_or_default(),
            name: job
                .try_get::<Option<String>, _>("name")
                .ok()
                .flatten()
                .unwrap_or_default(),
            address: job
                .try_get::<Option<String>, _>("address")
                .ok()
                .flatten()
                .unwrap_or_default(),
        };

        // R7. The upsert *is* the allocation, so it is atomic without a lock:
        // a second submission blocks on the row until this transaction ends,
        // then reads the committed value. A rollback returns the number.
        let number: i32 = sqlx::query_scalar(
            "INSERT INTO eto_job_sequences (job_id, next_number) VALUES ($1, 2) \
             ON CONFLICT (job_id) DO UPDATE \
               SET next_number = eto_job_sequences.next_number + 1 \
             RETURNING next_number - 1",
        )
        .bind(input.job_id.get())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let po_number = EtoNumber(number).po_number(&header.job_number, input.job_id);
        let (date, time) = stamp;

        let entry_id: i32 = sqlx::query_scalar(
            "INSERT INTO site_diary (job_id, author_id, date, time, work_completed) \
             VALUES ($1,$2,$3,$4,$5) RETURNING id",
        )
        .bind(input.job_id.get())
        .bind(author.get())
        .bind(date)
        .bind(time)
        .bind(eto_entry_summary(&po_number))
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let content = eto_note(&po_number, author_name, &header, input);
        let note_id: i32 = sqlx::query_scalar(
            "INSERT INTO diary_notes \
               (diary_entry_id, category, content, action_status, action_raised_by, sort_order) \
             VALUES ($1,'eto',$2,'action',$3,0) RETURNING id",
        )
        .bind(entry_id)
        .bind(&content)
        .bind(author.get())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(EtoRaised {
            entry_id: DiaryEntryId(entry_id),
            note_id: DiaryNoteId(note_id),
            eto_number: number,
            po_number,
            raised_by: author_name.to_string(),
        })
    }

    async fn etos_for_job(&self, scope: TenantScope, job: JobId) -> PortResult<Vec<EtoListItem>> {
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(
            "SELECT dn.id AS note_id, dn.diary_entry_id AS entry_id, dn.content, \
                    raiser.name AS raised_by, sd.date AS diary_date, \
                    dn.created_at AS approved_at, \
                    j.job_number, j.name AS job_name, j.address AS job_address \
             FROM diary_notes dn \
             JOIN site_diary sd ON sd.id = dn.diary_entry_id \
             JOIN jobs j ON j.id = sd.job_id \
             LEFT JOIN users raiser ON raiser.id = dn.action_raised_by \
             WHERE sd.job_id = $1 AND dn.category = 'eto' \
               AND dn.action_status = 'completed' AND dn.archived = false \
             ORDER BY dn.created_at DESC",
        )
        .bind(job.get())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let note_id: i32 = r.get("note_id");
                let content: String = r.get("content");
                let raised_by: Option<String> = r.get("raised_by");
                EtoListItem {
                    note_id: DiaryNoteId(note_id),
                    entry_id: DiaryEntryId(r.get("entry_id")),
                    eto_number: parse_eto_number(&content, note_id),
                    // The name column wins; notes predating it carry the name
                    // only in their text.
                    raised_by: raised_by
                        .or_else(|| parse_eto_raised_by(&content))
                        .unwrap_or_else(|| "User".to_string()),
                    content,
                    diary_date: r.get("diary_date"),
                    approved_at: r.get("approved_at"),
                    job_number: r.get("job_number"),
                    job_name: r.get("job_name"),
                    job_address: r.get("job_address"),
                }
            })
            .collect())
    }

    async fn get_or_create_draft(
        &self,
        scope: TenantScope,
        job: JobId,
        user: UserId,
        stamp: (chrono::NaiveDate, &str),
        empty_note: &str,
    ) -> PortResult<InspectionForm> {
        let mut tx = self.begin(scope).await?;

        let existing: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM inspection_forms WHERE job_id = $1 AND created_by = $2",
        )
        .bind(job.get())
        .bind(user.get())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        if let Some(id) = existing {
            let form = Self::load(&mut tx, InspectionFormId(id), user).await?;
            tx.commit().await.map_err(map_sqlx)?;
            return form.ok_or(PortError::NotFound);
        }

        let (date, time) = stamp;
        let entry_id: i32 = sqlx::query_scalar(
            "INSERT INTO site_diary (job_id, author_id, date, time, work_completed) \
             VALUES ($1,$2,$3,$4,'Site Inspection — Draft') RETURNING id",
        )
        .bind(job.get())
        .bind(user.get())
        .bind(date)
        .bind(time)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let note_id: i32 = sqlx::query_scalar(
            "INSERT INTO diary_notes (diary_entry_id, category, content) \
             VALUES ($1,'property-inspection',$2) RETURNING id",
        )
        .bind(entry_id)
        .bind(empty_note)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        // `company_id` is written explicitly because the unique constraint
        // spans it; RLS checks it rather than supplying it.
        let form_id: i32 = sqlx::query_scalar(
            "INSERT INTO inspection_forms \
               (company_id, job_id, created_by, diary_entry_id, diary_note_id, inspection_date) \
             VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
        )
        .bind(scope.company_id().get())
        .bind(job.get())
        .bind(user.get())
        .bind(entry_id)
        .bind(note_id)
        .bind(date)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let form = Self::load(&mut tx, InspectionFormId(form_id), user).await?;
        tx.commit().await.map_err(map_sqlx)?;
        form.ok_or(PortError::NotFound)
    }

    async fn find_draft(
        &self,
        scope: TenantScope,
        id: InspectionFormId,
        user: UserId,
    ) -> PortResult<Option<InspectionForm>> {
        let mut tx = self.begin(scope).await?;
        let form = Self::load(&mut tx, id, user).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(form)
    }

    async fn save_draft(
        &self,
        scope: TenantScope,
        id: InspectionFormId,
        user: UserId,
        input: &InspectionDraftInput,
        rendered: &RenderedInspection,
    ) -> PortResult<Option<InspectionForm>> {
        let mut tx = self.begin(scope).await?;

        let Some(form) = Self::load(&mut tx, id, user).await? else {
            return Err(PortError::NotFound);
        };

        if !Self::claim_revision(&mut tx, id, input.revision).await? {
            return Ok(None);
        }

        sqlx::query(
            "UPDATE inspection_forms \
             SET inspector = $2, inspection_type = $3, stage = $4, observations = $5, \
                 weather_data = $6 \
             WHERE id = $1",
        )
        .bind(id.get())
        .bind(&input.inspector)
        .bind(&input.inspection_type)
        .bind(&input.stage)
        .bind(&input.observations)
        .bind(&input.weather_data)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let items = input.retained();
        let keys: Vec<String> = items
            .iter()
            .map(|i| i.client_key.trim().to_string())
            .collect();

        // Dropping an item takes its photos with it, so their storage objects
        // must be queued before the cascade removes the rows that name them.
        let orphaned: Vec<i32> = sqlx::query_scalar(
            "SELECT m.diary_media_id FROM inspection_form_media m \
             JOIN inspection_form_items i ON i.id = m.item_id \
             WHERE i.form_id = $1 AND NOT (i.client_key = ANY($2))",
        )
        .bind(id.get())
        .bind(&keys)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        Self::detach_media(&mut tx, scope.company_id().get(), &orphaned).await?;

        sqlx::query(
            "DELETE FROM inspection_form_items WHERE form_id = $1 AND NOT (client_key = ANY($2))",
        )
        .bind(id.get())
        .bind(&keys)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        // One statement for every item: unnest turns the arrays into rows so a
        // 40-item checklist is one round-trip, not forty.
        if !items.is_empty() {
            let rooms: Vec<String> = items.iter().map(|i| i.room.clone()).collect();
            let descriptions: Vec<String> = items.iter().map(|i| i.description.clone()).collect();
            let actioned: Vec<bool> = items.iter().map(|i| i.actioned).collect();
            let orders: Vec<i32> = items
                .iter()
                .enumerate()
                .map(|(idx, i)| {
                    i.sort_order
                        .unwrap_or_else(|| i32::try_from(idx).unwrap_or(i32::MAX))
                })
                .collect();

            sqlx::query(
                "INSERT INTO inspection_form_items \
                   (form_id, client_key, room, description, actioned, sort_order) \
                 SELECT $1, * FROM unnest($2::text[], $3::text[], $4::text[], \
                                          $5::boolean[], $6::int[]) \
                 ON CONFLICT (form_id, client_key) DO UPDATE \
                   SET room = EXCLUDED.room, description = EXCLUDED.description, \
                       actioned = EXCLUDED.actioned, sort_order = EXCLUDED.sort_order",
            )
            .bind(id.get())
            .bind(&keys)
            .bind(&rooms)
            .bind(&descriptions)
            .bind(&actioned)
            .bind(&orders)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        sqlx::query(
            "UPDATE diary_notes SET content = $2, action_status = $3, action_raised_by = $4 \
             WHERE id = $1",
        )
        .bind(form.diary_note_id.get())
        .bind(&rendered.note_content)
        .bind(rendered.action_status)
        .bind(action_raiser(rendered, user))
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        sqlx::query(
            "UPDATE site_diary \
             SET work_completed = $2, action_status = $3, action_raised_by = $4, updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(form.diary_entry_id.get())
        .bind(&rendered.entry_summary)
        .bind(rendered.action_status)
        .bind(action_raiser(rendered, user))
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let saved = Self::load(&mut tx, id, user).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(saved)
    }

    async fn attach_photos(
        &self,
        scope: TenantScope,
        id: InspectionFormId,
        user: UserId,
        expected_revision: i32,
        photos: &[InspectionPhotoRecord],
    ) -> PortResult<Option<InspectionForm>> {
        let mut tx = self.begin(scope).await?;

        let Some(form) = Self::load(&mut tx, id, user).await? else {
            return Err(PortError::NotFound);
        };
        if !Self::claim_revision(&mut tx, id, expected_revision).await? {
            return Ok(None);
        }

        for photo in photos {
            let key = photo.client_key.trim();
            let item: Option<i32> = sqlx::query_scalar(
                "SELECT id FROM inspection_form_items WHERE form_id = $1 AND client_key = $2",
            )
            .bind(id.get())
            .bind(key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;

            // 409 rather than 404: the item existed when the client presigned
            // the upload, so this is a lost race, not a bad request.
            let Some(item_id) = item else {
                return Err(PortError::Conflict {
                    constraint: Some("inspection_item_missing".into()),
                });
            };

            let media_id: i32 = sqlx::query_scalar(
                "INSERT INTO diary_media \
                   (diary_entry_id, note_id, file_type, mime_type, original_name, \
                    stored_name, file_size, url, uploaded_by) \
                 VALUES ($1,$2,'photo',$3,$4,$5,$6,$7,$8) RETURNING id",
            )
            .bind(form.diary_entry_id.get())
            .bind(form.diary_note_id.get())
            .bind(&photo.media.mime_type)
            .bind(&photo.media.original_name)
            .bind(&photo.media.stored_name)
            .bind(i32::try_from(photo.media.file_size).unwrap_or(i32::MAX))
            .bind(&photo.media.url)
            .bind(photo.media.uploaded_by.get())
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;

            // Appends to the end of the item's existing photos.
            sqlx::query(
                "INSERT INTO inspection_form_media (form_id, item_id, diary_media_id, sort_order) \
                 SELECT $1, $2, $3, COALESCE(MAX(sort_order), -1) + 1 \
                 FROM inspection_form_media WHERE item_id = $2",
            )
            .bind(id.get())
            .bind(item_id)
            .bind(media_id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        let saved = Self::load(&mut tx, id, user).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(saved)
    }

    async fn delete_photo(
        &self,
        scope: TenantScope,
        id: InspectionFormId,
        user: UserId,
        expected_revision: i32,
        photo_id: i32,
    ) -> PortResult<Option<InspectionForm>> {
        let mut tx = self.begin(scope).await?;

        if Self::load(&mut tx, id, user).await?.is_none() {
            return Err(PortError::NotFound);
        }
        if !Self::claim_revision(&mut tx, id, expected_revision).await? {
            return Ok(None);
        }

        // Scoped to the form as well as the photo: an id from another
        // inspection must not be deletable by guessing.
        let media_id: Option<i32> = sqlx::query_scalar(
            "SELECT diary_media_id FROM inspection_form_media WHERE id = $1 AND form_id = $2",
        )
        .bind(photo_id)
        .bind(id.get())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let Some(media_id) = media_id else {
            return Err(PortError::NotFound);
        };

        // Deleting the diary_media row cascades to inspection_form_media.
        Self::detach_media(&mut tx, scope.company_id().get(), &[media_id]).await?;

        let saved = Self::load(&mut tx, id, user).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(saved)
    }

    async fn photo_counts(
        &self,
        scope: TenantScope,
        id: InspectionFormId,
    ) -> PortResult<HashMap<String, usize>> {
        let mut tx = self.begin(scope).await?;
        let rows = sqlx::query(
            "SELECT i.client_key, COUNT(*) AS n FROM inspection_form_media m \
             JOIN inspection_form_items i ON i.id = m.item_id \
             WHERE m.form_id = $1 GROUP BY i.client_key",
        )
        .bind(id.get())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let n: i64 = r.get("n");
                (
                    r.get::<String, _>("client_key"),
                    usize::try_from(n).unwrap_or(0),
                )
            })
            .collect())
    }
}

/// Only an outstanding action names who raised it.
fn action_raiser(rendered: &RenderedInspection, user: UserId) -> Option<i32> {
    (rendered.action_status == Some("action")).then(|| user.get())
}
