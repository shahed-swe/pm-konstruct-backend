//! `DiaryRepository` over Postgres.
//!
//! `site_diary` carries no `company_id`; it is tenant-scoped through its job by
//! the RLS policy in migration 0004, so setting the GUC is what constrains it.
//! Job-level visibility (domain-rules R3) is layered on top via `visible_jobs`.

use async_trait::async_trait;
use pmk_domain::diary::{
    ActionStatus, DiaryEntry, DiaryEntryInput, DiaryNote, DiaryNoteComment, DiaryNoteInput,
    NoteCategory, WeatherStamp,
};
use pmk_domain::ids::{DiaryEntryId, DiaryNoteId, JobId, UserId};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::{DiaryContext, DiaryFilter, DiaryRepository, WeatherSnapshot};
use pmk_ports::{PortError, PortResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

const ENTRY_COLUMNS: &str = "id, job_id, date, time, author_id, weather, workforce, \
     work_completed, materials, trades_on_site, safety_notes, client_instructions, equipment, \
     visitors, issues, notes, location_name, location_lat, location_lng, temperature, \
     weather_condition, weather_icon, wind_speed_kmh, rainfall_mm, sunrise_time, sunset_time, \
     action_status, action_raised_by, created_at, updated_at";

#[derive(Debug, sqlx::FromRow)]
struct EntryRow {
    id: i32,
    job_id: i32,
    date: chrono::NaiveDate,
    time: Option<String>,
    author_id: Option<i32>,
    weather: Option<String>,
    workforce: Option<i32>,
    work_completed: String,
    materials: Option<String>,
    trades_on_site: Option<String>,
    safety_notes: Option<String>,
    client_instructions: Option<String>,
    equipment: Option<String>,
    visitors: Option<String>,
    issues: Option<String>,
    notes: Option<String>,
    location_name: Option<String>,
    location_lat: Option<Decimal>,
    location_lng: Option<Decimal>,
    temperature: Option<Decimal>,
    weather_condition: Option<String>,
    weather_icon: Option<String>,
    wind_speed_kmh: Option<Decimal>,
    rainfall_mm: Option<Decimal>,
    sunrise_time: Option<String>,
    sunset_time: Option<String>,
    action_status: Option<String>,
    action_raised_by: Option<i32>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<EntryRow> for DiaryEntry {
    fn from(r: EntryRow) -> Self {
        Self {
            id: DiaryEntryId(r.id),
            job_id: JobId(r.job_id),
            date: r.date,
            time: r.time,
            author_id: r.author_id.map(UserId),
            weather: r.weather,
            workforce: r.workforce,
            work_completed: r.work_completed,
            materials: r.materials,
            trades_on_site: r.trades_on_site,
            safety_notes: r.safety_notes,
            client_instructions: r.client_instructions,
            equipment: r.equipment,
            visitors: r.visitors,
            issues: r.issues,
            notes: r.notes,
            weather_stamp: WeatherStamp {
                location_name: r.location_name,
                location_lat: r.location_lat,
                location_lng: r.location_lng,
                temperature: r.temperature,
                weather_condition: r.weather_condition,
                weather_icon: r.weather_icon,
                wind_speed_kmh: r.wind_speed_kmh,
                rainfall_mm: r.rainfall_mm,
                sunrise_time: r.sunrise_time,
                sunset_time: r.sunset_time,
            },
            // Constrained by `site_diary_action_status_check`, so an unknown
            // value means corrupt data; treated as absent rather than failing
            // the whole read.
            action_status: r.action_status.as_deref().and_then(ActionStatus::parse),
            action_raised_by: r.action_raised_by.map(UserId),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct NoteRow {
    id: i32,
    diary_entry_id: i32,
    category: String,
    content: String,
    action_status: Option<String>,
    action_raised_by: Option<i32>,
    sort_order: i32,
    archived: bool,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<NoteRow> for DiaryNote {
    fn from(r: NoteRow) -> Self {
        Self {
            id: DiaryNoteId(r.id),
            diary_entry_id: DiaryEntryId(r.diary_entry_id),
            category: NoteCategory::parse(&r.category).unwrap_or(NoteCategory::General),
            content: r.content,
            action_status: r.action_status.as_deref().and_then(ActionStatus::parse),
            action_raised_by: r.action_raised_by.map(UserId),
            sort_order: r.sort_order,
            archived: r.archived,
            created_at: r.created_at,
        }
    }
}

const NOTE_COLUMNS: &str = "id, diary_entry_id, category, content, action_status, \
     action_raised_by, sort_order, archived, created_at";

#[derive(Debug, sqlx::FromRow)]
struct CommentRow {
    id: i32,
    note_id: i32,
    author_id: Option<i32>,
    author_name: Option<String>,
    content: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<CommentRow> for DiaryNoteComment {
    fn from(r: CommentRow) -> Self {
        Self {
            id: r.id,
            note_id: DiaryNoteId(r.note_id),
            author_id: r.author_id.map(UserId),
            author_name: r.author_name,
            content: r.content,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PgDiaryRepository {
    pool: PgPool,
}

impl PgDiaryRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin(&self, scope: TenantScope) -> PortResult<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        Ok(tx)
    }

    /// `NULL` means "no job restriction" (manager/office); otherwise the array
    /// bounds the query. An **empty** slice is passed through as an empty array,
    /// which correctly matches nothing -- unlike the legacy
    /// `getAuthorizedJobIds`, where a null meant "everything".
    fn job_ids(visible: Option<&[JobId]>) -> Option<Vec<i32>> {
        visible.map(|ids| ids.iter().map(|j| j.get()).collect())
    }
}

#[async_trait]
impl DiaryRepository for PgDiaryRepository {
    async fn list(
        &self,
        scope: TenantScope,
        visible_jobs: Option<&[JobId]>,
        filter: &DiaryFilter,
    ) -> PortResult<Vec<DiaryEntry>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM site_diary \
             WHERE ($1::int[] IS NULL OR job_id = ANY($1)) \
               AND ($2::int IS NULL OR job_id = $2) \
               AND ($3::date IS NULL OR date >= $3) \
               AND ($4::date IS NULL OR date <= $4) \
               AND ($5::text IS NULL OR action_status = $5) \
             ORDER BY date DESC, id DESC"
        );
        let rows: Vec<EntryRow> = sqlx::query_as(&sql)
            .bind(Self::job_ids(visible_jobs))
            .bind(filter.job_id.map(JobId::get))
            .bind(filter.from)
            .bind(filter.to)
            .bind(filter.action_status.as_deref())
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn find(
        &self,
        scope: TenantScope,
        visible_jobs: Option<&[JobId]>,
        id: DiaryEntryId,
    ) -> PortResult<Option<DiaryEntry>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM site_diary \
             WHERE id = $1 AND ($2::int[] IS NULL OR job_id = ANY($2))"
        );
        let row: Option<EntryRow> = sqlx::query_as(&sql)
            .bind(id.get())
            .bind(Self::job_ids(visible_jobs))
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn context(
        &self,
        scope: TenantScope,
        ids: &[DiaryEntryId],
    ) -> PortResult<std::collections::HashMap<i32, DiaryContext>> {
        use std::collections::HashMap;

        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let raw: Vec<i32> = ids.iter().map(|i| i.get()).collect();
        let mut tx = self.begin(scope).await?;

        // Left joins throughout: an entry whose author has since been deleted
        // is still an entry, and dropping the row would make it vanish from
        // the list rather than merely lose a name.
        let rows: Vec<(
            i32,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT d.id, j.name, j.job_number, j.address, u.name \
                 FROM site_diary d \
                 LEFT JOIN jobs j ON j.id = d.job_id \
                 LEFT JOIN users u ON u.id = d.author_id \
                 WHERE d.id = ANY($1)",
        )
        .bind(&raw)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        // The earliest note, and the count. `DISTINCT ON` picks one row per
        // entry in the same order the notes are displayed, so the summary is
        // the note a reader would see first rather than an arbitrary one.
        let notes: Vec<(i32, String, i64)> = sqlx::query_as(
            "SELECT DISTINCT ON (n.diary_entry_id) n.diary_entry_id, n.content, \
                    count(*) OVER (PARTITION BY n.diary_entry_id) AS note_count \
             FROM diary_notes n \
             WHERE n.diary_entry_id = ANY($1) AND NOT n.archived \
             ORDER BY n.diary_entry_id, n.sort_order, n.id",
        )
        .bind(&raw)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;

        let mut out: std::collections::HashMap<i32, DiaryContext> = rows
            .into_iter()
            .map(|(id, job_name, job_number, job_address, author_name)| {
                (
                    id,
                    DiaryContext {
                        job_name,
                        job_number,
                        job_address,
                        author_name,
                        first_note: None,
                        note_count: 0,
                    },
                )
            })
            .collect();

        for (entry_id, content, count) in notes {
            if let Some(context) = out.get_mut(&entry_id) {
                context.first_note = Some(content);
                context.note_count = count;
            }
        }

        Ok(out)
    }

    async fn create(
        &self,
        scope: TenantScope,
        author: UserId,
        input: &DiaryEntryInput,
        weather: &WeatherStamp,
        today: chrono::NaiveDate,
    ) -> PortResult<DiaryEntry> {
        let mut tx = self.begin(scope).await?;
        // The date defaults to "today" in the company's configured timezone,
        // supplied by the caller -- never the database's now().
        let sql = format!(
            "INSERT INTO site_diary (job_id, date, time, author_id, weather, workforce, \
                work_completed, materials, trades_on_site, safety_notes, client_instructions, \
                equipment, visitors, issues, notes, location_name, location_lat, location_lng, \
                temperature, weather_condition, weather_icon, wind_speed_kmh, rainfall_mm, \
                sunrise_time, sunset_time, action_status, action_raised_by) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20, \
                     $21,$22,$23,$24,$25,$26,$27) \
             RETURNING {ENTRY_COLUMNS}"
        );
        let row: EntryRow = sqlx::query_as(&sql)
            .bind(input.job_id)
            .bind(input.date.unwrap_or(today))
            .bind(input.time.as_deref())
            .bind(author.get())
            .bind(input.weather.as_deref())
            .bind(input.workforce)
            .bind(&input.work_completed)
            .bind(input.materials.as_deref())
            .bind(input.trades_on_site.as_deref())
            .bind(input.safety_notes.as_deref())
            .bind(input.client_instructions.as_deref())
            .bind(input.equipment.as_deref())
            .bind(input.visitors.as_deref())
            .bind(input.issues.as_deref())
            .bind(input.notes.as_deref())
            .bind(weather.location_name.as_deref())
            .bind(weather.location_lat)
            .bind(weather.location_lng)
            .bind(weather.temperature)
            .bind(weather.weather_condition.as_deref())
            .bind(weather.weather_icon.as_deref())
            .bind(weather.wind_speed_kmh)
            .bind(weather.rainfall_mm)
            .bind(weather.sunrise_time.as_deref())
            .bind(weather.sunset_time.as_deref())
            .bind(input.action_status.map(|s| s.as_str()))
            .bind(input.action_status.map(|_| author.get()))
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.into())
    }

    async fn update(
        &self,
        scope: TenantScope,
        id: DiaryEntryId,
        input: &DiaryEntryInput,
    ) -> PortResult<Option<DiaryEntry>> {
        let mut tx = self.begin(scope).await?;
        // The weather stamp is deliberately absent: it records conditions at
        // creation and is never rewritten (domain-rules R8).
        let sql = format!(
            "UPDATE site_diary SET date = COALESCE($2, date), time = $3, weather = $4, \
                workforce = $5, work_completed = $6, materials = $7, trades_on_site = $8, \
                safety_notes = $9, client_instructions = $10, equipment = $11, visitors = $12, \
                issues = $13, notes = $14, updated_at = NOW() \
             WHERE id = $1 RETURNING {ENTRY_COLUMNS}"
        );
        let row: Option<EntryRow> = sqlx::query_as(&sql)
            .bind(id.get())
            .bind(input.date)
            .bind(input.time.as_deref())
            .bind(input.weather.as_deref())
            .bind(input.workforce)
            .bind(&input.work_completed)
            .bind(input.materials.as_deref())
            .bind(input.trades_on_site.as_deref())
            .bind(input.safety_notes.as_deref())
            .bind(input.client_instructions.as_deref())
            .bind(input.equipment.as_deref())
            .bind(input.visitors.as_deref())
            .bind(input.issues.as_deref())
            .bind(input.notes.as_deref())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn delete(&self, scope: TenantScope, id: DiaryEntryId) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query("DELETE FROM site_diary WHERE id = $1")
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }

    async fn set_action_status(
        &self,
        scope: TenantScope,
        id: DiaryEntryId,
        status: Option<ActionStatus>,
        raised_by: Option<UserId>,
    ) -> PortResult<Option<DiaryEntry>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE site_diary SET action_status = $2, \
                action_raised_by = CASE WHEN $2::text IS NULL THEN NULL ELSE $3 END, \
                updated_at = NOW() \
             WHERE id = $1 RETURNING {ENTRY_COLUMNS}"
        );
        let row: Option<EntryRow> = sqlx::query_as(&sql)
            .bind(id.get())
            .bind(status.map(|s| s.as_str()))
            .bind(raised_by.map(UserId::get))
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn notes(
        &self,
        scope: TenantScope,
        entry: DiaryEntryId,
        include_archived: bool,
    ) -> PortResult<Vec<DiaryNote>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "SELECT {NOTE_COLUMNS} FROM diary_notes \
             WHERE diary_entry_id = $1 AND ($2 OR NOT archived) \
             ORDER BY sort_order, id"
        );
        let rows: Vec<NoteRow> = sqlx::query_as(&sql)
            .bind(entry.get())
            .bind(include_archived)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn add_note(
        &self,
        scope: TenantScope,
        entry: DiaryEntryId,
        input: &DiaryNoteInput,
        raised_by: Option<UserId>,
    ) -> PortResult<DiaryNote> {
        let mut tx = self.begin(scope).await?;

        // The parent must exist and be visible under the tenant policy;
        // otherwise the insert would fail on the foreign key with a confusing
        // conflict instead of a clean 404.
        let parent: Option<(i32,)> = sqlx::query_as("SELECT id FROM site_diary WHERE id = $1")
            .bind(entry.get())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        if parent.is_none() {
            return Err(PortError::NotFound);
        }

        let sql = format!(
            "INSERT INTO diary_notes (diary_entry_id, category, content, action_status, \
                action_raised_by, sort_order) \
             VALUES ($1,$2,$3,$4,$5, \
               COALESCE($6, (SELECT COALESCE(MAX(sort_order), -1) + 1 \
                             FROM diary_notes WHERE diary_entry_id = $1))) \
             RETURNING {NOTE_COLUMNS}"
        );
        let row: NoteRow = sqlx::query_as(&sql)
            .bind(entry.get())
            .bind(input.category.as_str())
            .bind(&input.content)
            .bind(input.action_status.map(|s| s.as_str()))
            .bind(input.action_status.and_then(|_| raised_by.map(UserId::get)))
            .bind(input.sort_order)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.into())
    }

    async fn update_note(
        &self,
        scope: TenantScope,
        note: DiaryNoteId,
        input: &DiaryNoteInput,
    ) -> PortResult<Option<DiaryNote>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE diary_notes SET category = $2, content = $3, \
                sort_order = COALESCE($4, sort_order) \
             WHERE id = $1 RETURNING {NOTE_COLUMNS}"
        );
        let row: Option<NoteRow> = sqlx::query_as(&sql)
            .bind(note.get())
            .bind(input.category.as_str())
            .bind(&input.content)
            .bind(input.sort_order)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn set_note_action_status(
        &self,
        scope: TenantScope,
        note: DiaryNoteId,
        status: Option<ActionStatus>,
        raised_by: Option<UserId>,
    ) -> PortResult<Option<DiaryNote>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE diary_notes SET action_status = $2, \
                action_raised_by = CASE WHEN $2::text IS NULL THEN NULL ELSE $3 END \
             WHERE id = $1 RETURNING {NOTE_COLUMNS}"
        );
        let row: Option<NoteRow> = sqlx::query_as(&sql)
            .bind(note.get())
            .bind(status.map(|s| s.as_str()))
            .bind(raised_by.map(UserId::get))
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn set_note_archived(
        &self,
        scope: TenantScope,
        note: DiaryNoteId,
        archived: bool,
    ) -> PortResult<Option<DiaryNote>> {
        let mut tx = self.begin(scope).await?;
        let sql =
            format!("UPDATE diary_notes SET archived = $2 WHERE id = $1 RETURNING {NOTE_COLUMNS}");
        let row: Option<NoteRow> = sqlx::query_as(&sql)
            .bind(note.get())
            .bind(archived)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn delete_note(&self, scope: TenantScope, note: DiaryNoteId) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        // Comments and media cascade from the note's foreign keys.
        let r = sqlx::query("DELETE FROM diary_notes WHERE id = $1")
            .bind(note.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }

    async fn comments(
        &self,
        scope: TenantScope,
        note: DiaryNoteId,
    ) -> PortResult<Vec<DiaryNoteComment>> {
        let mut tx = self.begin(scope).await?;
        let rows: Vec<CommentRow> = sqlx::query_as(
            "SELECT c.id, c.note_id, c.author_id, u.name AS author_name, c.content, c.created_at \
             FROM diary_note_comments c LEFT JOIN users u ON u.id = c.author_id \
             WHERE c.note_id = $1 ORDER BY c.created_at, c.id",
        )
        .bind(note.get())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn add_comment(
        &self,
        scope: TenantScope,
        note: DiaryNoteId,
        author: UserId,
        content: &str,
    ) -> PortResult<DiaryNoteComment> {
        let mut tx = self.begin(scope).await?;

        let parent: Option<(i32,)> = sqlx::query_as("SELECT id FROM diary_notes WHERE id = $1")
            .bind(note.get())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        if parent.is_none() {
            return Err(PortError::NotFound);
        }

        let row: CommentRow = sqlx::query_as(
            "WITH ins AS (\
               INSERT INTO diary_note_comments (note_id, author_id, content) \
               VALUES ($1, $2, $3) RETURNING id, note_id, author_id, content, created_at) \
             SELECT ins.id, ins.note_id, ins.author_id, u.name AS author_name, ins.content, \
                    ins.created_at \
             FROM ins LEFT JOIN users u ON u.id = ins.author_id",
        )
        .bind(note.get())
        .bind(author.get())
        .bind(content)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(row.into())
    }

    // -- weather snapshot ----------------------------------------------------

    async fn weather_snapshot(
        &self,
        scope: TenantScope,
        entry: DiaryEntryId,
    ) -> PortResult<Option<WeatherSnapshot>> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query(&format!(
            "SELECT {SNAPSHOT_COLS} FROM weather_snapshots WHERE diary_entry_id = $1"
        ))
        .bind(entry.get())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.as_ref().map(weather_snapshot_row))
    }

    async fn set_weather_snapshot(
        &self,
        scope: TenantScope,
        snapshot: &WeatherSnapshot,
    ) -> PortResult<WeatherSnapshot> {
        let mut tx = self.begin(scope).await?;
        // One row per entry, by the unique constraint from migration 0012.
        // Re-reading conditions later replaces the reading rather than
        // appending a second one.
        let r = sqlx::query(&format!(
            "INSERT INTO weather_snapshots \
               (diary_entry_id, temperature_c, conditions, rain, wind_description, \
                wind_speed_kmh, humidity_pct, source, snapshot_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
             ON CONFLICT (diary_entry_id) DO UPDATE \
               SET temperature_c = EXCLUDED.temperature_c, \
                   conditions = EXCLUDED.conditions, \
                   rain = EXCLUDED.rain, \
                   wind_description = EXCLUDED.wind_description, \
                   wind_speed_kmh = EXCLUDED.wind_speed_kmh, \
                   humidity_pct = EXCLUDED.humidity_pct, \
                   source = EXCLUDED.source, \
                   snapshot_at = EXCLUDED.snapshot_at \
             RETURNING {SNAPSHOT_COLS}"
        ))
        .bind(snapshot.diary_entry_id.get())
        .bind(snapshot.temperature_c)
        .bind(snapshot.conditions.as_deref())
        .bind(snapshot.rain.as_deref())
        .bind(snapshot.wind_description.as_deref())
        .bind(snapshot.wind_speed_kmh)
        .bind(snapshot.humidity_pct)
        .bind(&snapshot.source)
        .bind(snapshot.snapshot_at)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(weather_snapshot_row(&r))
    }
}

/// Maps a `weather_snapshots` row.
fn weather_snapshot_row(r: &sqlx::postgres::PgRow) -> WeatherSnapshot {
    use sqlx::Row as _;
    WeatherSnapshot {
        diary_entry_id: DiaryEntryId(r.get("diary_entry_id")),
        temperature_c: r.get("temperature_c"),
        conditions: r.get("conditions"),
        rain: r.get("rain"),
        wind_description: r.get("wind_description"),
        wind_speed_kmh: r.get("wind_speed_kmh"),
        humidity_pct: r.get("humidity_pct"),
        source: r.get("source"),
        snapshot_at: r.get("snapshot_at"),
    }
}

const SNAPSHOT_COLS: &str = "diary_entry_id, temperature_c, conditions, rain, \
                             wind_description, wind_speed_kmh, humidity_pct, source, snapshot_at";
