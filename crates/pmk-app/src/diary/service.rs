use std::sync::Arc;

use pmk_domain::diary::{
    ActionStatus, DiaryEntry, DiaryEntryInput, DiaryNote, DiaryNoteComment, DiaryNoteInput,
    WeatherStamp,
};
use pmk_domain::ids::{DiaryEntryId, DiaryNoteId, JobId};
use pmk_domain::DomainError;
use pmk_ports::repository::{DiaryFilter, DiaryRepository, JobRepository, WeatherProvider};
use pmk_ports::Clock;
use pmk_ports::{BroadcastEvent, EventBus};

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

pub struct DiaryService {
    diary: Arc<dyn DiaryRepository>,
    jobs: Arc<dyn JobRepository>,
    weather: Option<Arc<dyn WeatherProvider>>,
    clock: Arc<dyn Clock>,
    /// Optional so the service is constructible in tests without a database
    /// listener behind it.
    events: Option<Arc<dyn EventBus>>,
}

impl std::fmt::Debug for DiaryService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiaryService").finish_non_exhaustive()
    }
}

impl DiaryService {
    #[must_use]
    pub fn new(
        diary: Arc<dyn DiaryRepository>,
        jobs: Arc<dyn JobRepository>,
        weather: Option<Arc<dyn WeatherProvider>>,
        clock: Arc<dyn Clock>,
        events: Option<Arc<dyn EventBus>>,
    ) -> Self {
        Self {
            diary,
            jobs,
            weather,
            clock,
            events,
        }
    }

    /// The job ids a caller may see, or `None` for company-wide roles.
    ///
    /// Resolved once per request rather than per row. `Some(vec![])` for a
    /// supervisor with no assignments correctly matches nothing -- the legacy
    /// `null` in that position meant "everything".
    async fn visible_jobs(&self, s: &SessionUser) -> AppResult<Option<Vec<JobId>>> {
        if s.user.role.sees_all_company_jobs() {
            return Ok(None);
        }
        Ok(Some(
            self.jobs
                .visible_job_ids(s.principal.scope(), s.user.id)
                .await?,
        ))
    }

    pub async fn list(&self, s: &SessionUser, filter: DiaryFilter) -> AppResult<Vec<DiaryEntry>> {
        let visible = self.visible_jobs(s).await?;
        Ok(self
            .diary
            .list(s.principal.scope(), visible.as_deref(), &filter)
            .await?)
    }

    pub async fn get(&self, s: &SessionUser, id: DiaryEntryId) -> AppResult<DiaryEntry> {
        let visible = self.visible_jobs(s).await?;
        self.diary
            .find(s.principal.scope(), visible.as_deref(), id)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Diary entry")))
    }

    pub async fn create(&self, s: &SessionUser, input: DiaryEntryInput) -> AppResult<DiaryEntry> {
        input.validate().map_err(AppError::Domain)?;

        // The caller must be able to see the job it is filing against.
        self.jobs
            .find(
                s.principal.scope(),
                s.user.id,
                s.user.role.sees_all_company_jobs(),
                JobId(input.job_id),
            )
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Job")))?;

        let weather = self.stamp_weather().await;

        Ok(self
            .diary
            .create(
                s.principal.scope(),
                s.user.id,
                &input,
                &weather,
                self.clock.today(),
            )
            .await?)
    }

    /// Fetches the weather stamp, swallowing any failure.
    ///
    /// Domain-rules R8: a weather outage must never stop a supervisor filing
    /// the day's diary. The entry saves with null weather fields instead.
    async fn stamp_weather(&self) -> WeatherStamp {
        let Some(provider) = &self.weather else {
            return WeatherStamp::default();
        };
        // Coordinates come from the job site in Phase 14; until then the
        // provider is absent and this is a no-op.
        match provider.current(0.0, 0.0).await {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "weather lookup failed; entry saved without a stamp");
                WeatherStamp::default()
            }
        }
    }

    pub async fn update(
        &self,
        s: &SessionUser,
        id: DiaryEntryId,
        input: DiaryEntryInput,
    ) -> AppResult<DiaryEntry> {
        self.get(s, id).await?;
        input.validate().map_err(AppError::Domain)?;
        self.diary
            .update(s.principal.scope(), id, &input)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Diary entry")))
    }

    pub async fn delete(&self, s: &SessionUser, id: DiaryEntryId) -> AppResult<()> {
        self.get(s, id).await?;
        if self.diary.delete(s.principal.scope(), id).await? {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Diary entry")))
        }
    }

    pub async fn set_action_status(
        &self,
        s: &SessionUser,
        id: DiaryEntryId,
        status: Option<ActionStatus>,
    ) -> AppResult<DiaryEntry> {
        self.get(s, id).await?;
        self.diary
            .set_action_status(s.principal.scope(), id, status, Some(s.user.id))
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Diary entry")))
    }

    /// Tells connected clients a job's diary changed.
    ///
    /// Best effort: a failure to broadcast is logged and swallowed, because
    /// the write has already committed and failing the request would tell the
    /// user their entry was not saved when it was. The UI refetches on
    /// reconnect, so a missed event costs a delay rather than data.
    async fn announce(&self, s: &SessionUser, job: JobId, entry: DiaryEntryId) {
        let Some(bus) = &self.events else { return };
        let event = BroadcastEvent::for_job(
            s.principal.company_id(),
            job,
            "site-diary",
            "read",
            "diary_note_updated",
            serde_json::json!({ "entryId": entry.get(), "jobId": job.get() }),
        );
        if let Err(e) = bus.publish(&event).await {
            tracing::warn!(error = %e, "could not broadcast a diary change");
        }
    }

    // ── notes ───────────────────────────────────────────────────────────────

    pub async fn notes(
        &self,
        s: &SessionUser,
        entry: DiaryEntryId,
        include_archived: bool,
    ) -> AppResult<Vec<DiaryNote>> {
        self.get(s, entry).await?;
        Ok(self
            .diary
            .notes(s.principal.scope(), entry, include_archived)
            .await?)
    }

    pub async fn add_note(
        &self,
        s: &SessionUser,
        entry: DiaryEntryId,
        input: DiaryNoteInput,
    ) -> AppResult<DiaryNote> {
        let parent = self.get(s, entry).await?;
        input.validate().map_err(AppError::Domain)?;
        let note = self
            .diary
            .add_note(s.principal.scope(), entry, &input, Some(s.user.id))
            .await?;
        self.announce(s, parent.job_id, entry).await;
        Ok(note)
    }

    pub async fn update_note(
        &self,
        s: &SessionUser,
        entry: DiaryEntryId,
        note: DiaryNoteId,
        input: DiaryNoteInput,
    ) -> AppResult<DiaryNote> {
        let parent = self.get(s, entry).await?;
        input.validate().map_err(AppError::Domain)?;
        let updated = self
            .diary
            .update_note(s.principal.scope(), note, &input)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Note")))?;
        self.announce(s, parent.job_id, entry).await;
        Ok(updated)
    }

    pub async fn set_note_action_status(
        &self,
        s: &SessionUser,
        entry: DiaryEntryId,
        note: DiaryNoteId,
        status: Option<ActionStatus>,
    ) -> AppResult<DiaryNote> {
        let parent = self.get(s, entry).await?;
        let updated = self
            .diary
            .set_note_action_status(s.principal.scope(), note, status, Some(s.user.id))
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Note")))?;
        self.announce(s, parent.job_id, entry).await;
        Ok(updated)
    }

    pub async fn set_note_archived(
        &self,
        s: &SessionUser,
        entry: DiaryEntryId,
        note: DiaryNoteId,
        archived: bool,
    ) -> AppResult<DiaryNote> {
        self.get(s, entry).await?;
        self.diary
            .set_note_archived(s.principal.scope(), note, archived)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Note")))
    }

    pub async fn delete_note(
        &self,
        s: &SessionUser,
        entry: DiaryEntryId,
        note: DiaryNoteId,
    ) -> AppResult<()> {
        self.get(s, entry).await?;
        if self.diary.delete_note(s.principal.scope(), note).await? {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Note")))
        }
    }

    // ── comment threads ─────────────────────────────────────────────────────

    pub async fn comments(
        &self,
        s: &SessionUser,
        entry: DiaryEntryId,
        note: DiaryNoteId,
    ) -> AppResult<Vec<DiaryNoteComment>> {
        self.get(s, entry).await?;
        Ok(self.diary.comments(s.principal.scope(), note).await?)
    }

    pub async fn add_comment(
        &self,
        s: &SessionUser,
        entry: DiaryEntryId,
        note: DiaryNoteId,
        content: String,
    ) -> AppResult<DiaryNoteComment> {
        self.get(s, entry).await?;
        if content.trim().is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "content",
                "is required",
            )));
        }
        Ok(self
            .diary
            .add_comment(s.principal.scope(), note, s.user.id, &content)
            .await?)
    }
}
