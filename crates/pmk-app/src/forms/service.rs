use std::sync::Arc;

use pmk_domain::email::{
    assert_recipients_allowed, normalise_recipients, redact_internal_eto_reason,
};
use pmk_domain::forms::{
    inspection_action_status, inspection_entry_summary, inspection_note, stale_revision, EtoInput,
    InspectionDraftInput,
};
use pmk_domain::ids::{InspectionFormId, JobId};
use pmk_domain::media::{
    validate_batch_size, validate_upload, UploadRequest, MAX_FILES_PER_UPLOAD,
};
use pmk_domain::DomainError;
use pmk_ports::repository::{
    EtoListItem, EtoRaised, FormsRepository, InspectionForm, InspectionPhotoRecord, JobRepository,
    MediaOwner, MediaRecord, RenderedInspection, UserRepository,
};
use pmk_ports::{Clock, Message, ObjectStore};

use crate::identity::SessionUser;
use crate::media::upload;
use crate::{AppError, AppResult};

/// A presigned slot for one inspection photo.
#[derive(Debug, Clone)]
pub struct PreparedInspectionUpload {
    pub stored_name: String,
    pub upload_url: String,
    pub expires_in_secs: u64,
    pub max_bytes: u64,
}

/// A photo the client says it has uploaded.
#[derive(Debug, Clone)]
pub struct InspectionPhotoUpload {
    pub stored_name: String,
    pub original_name: String,
    pub mime_type: String,
}

pub struct FormsService {
    forms: Arc<dyn FormsRepository>,
    /// For the recipient allowlist: who may be emailed a form.
    users: Arc<dyn UserRepository>,
    mail: Arc<crate::mail::Mailer>,
    jobs: Arc<dyn JobRepository>,
    store: Arc<dyn ObjectStore>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for FormsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FormsService").finish_non_exhaustive()
    }
}

impl FormsService {
    #[must_use]
    pub fn new(
        forms: Arc<dyn FormsRepository>,
        jobs: Arc<dyn JobRepository>,
        store: Arc<dyn ObjectStore>,
        clock: Arc<dyn Clock>,
        users: Arc<dyn UserRepository>,
        mail: Arc<crate::mail::Mailer>,
    ) -> Self {
        Self {
            forms,
            jobs,
            store,
            clock,
            users,
            mail,
        }
    }

    /// A job the caller can actually see, or a 404.
    ///
    /// A supervisor with no assignment to a job must not be able to raise an
    /// ETO against it, and must not learn that it exists by being told 403.
    async fn assert_job_access(&self, s: &SessionUser, job: JobId) -> AppResult<()> {
        self.jobs
            .find(
                s.principal.scope(),
                s.user.id,
                s.user.role.sees_all_company_jobs(),
                job,
            )
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Job")))?;
        Ok(())
    }

    /// Date and `HH:MM`, both in the company's timezone.
    fn stamp(&self) -> (chrono::NaiveDate, String) {
        (self.clock.today(), self.clock.time_hm())
    }

    // ── extras to order ─────────────────────────────────────────────────────

    /// Raises an ETO, allocating the next gap-free number for the job (R7).
    pub async fn raise_eto(&self, s: &SessionUser, input: &EtoInput) -> AppResult<EtoRaised> {
        input.validate().map_err(AppError::Domain)?;
        self.assert_job_access(s, input.job_id).await?;
        let (date, time) = self.stamp();
        Ok(self
            .forms
            .raise_eto(
                s.principal.scope(),
                s.user.id,
                &s.user.name,
                input,
                (date, &time),
            )
            .await?)
    }

    pub async fn etos_for_job(&self, s: &SessionUser, job: JobId) -> AppResult<Vec<EtoListItem>> {
        self.assert_job_access(s, job).await?;
        Ok(self.forms.etos_for_job(s.principal.scope(), job).await?)
    }

    /// Emails a form to colleagues on the job.
    ///
    /// The same recipient rule as the diary: active users who can see the job,
    /// so the feature cannot be used to relay mail anywhere else.
    pub async fn email_form(
        &self,
        s: &SessionUser,
        job: JobId,
        to: &[String],
        subject: &str,
        body: &str,
    ) -> AppResult<Vec<String>> {
        self.assert_job_access(s, job).await?;

        let recipients = normalise_recipients(to).map_err(AppError::Domain)?;
        let allowed = self
            .users
            .email_recipients_for_job(s.principal.scope(), job)
            .await?;
        assert_recipients_allowed(&recipients, &allowed).map_err(AppError::Domain)?;

        if subject.trim().is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "subject",
                "a subject is required",
            )));
        }
        if body.trim().is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "body",
                "a message is required",
            )));
        }

        self.mail
            .send(
                s,
                &Message {
                    to: recipients.clone(),
                    subject: subject.trim().to_string(),
                    // Redacted like a diary note: a form can carry an ETO's
                    // internal reason if someone pasted it in.
                    body: redact_internal_eto_reason(body),
                    html: false,
                },
            )
            .await?;

        Ok(recipients)
    }

    // ── property inspection ─────────────────────────────────────────────────

    pub async fn get_or_create_draft(
        &self,
        s: &SessionUser,
        job: JobId,
    ) -> AppResult<InspectionForm> {
        self.assert_job_access(s, job).await?;
        let (date, time) = self.stamp();
        // An empty draft still gets a note, so the entry reads sensibly the
        // moment it appears in the diary rather than after the first save.
        let empty = inspection_note(&empty_draft(), date, &|_| 0);
        Ok(self
            .forms
            .get_or_create_draft(s.principal.scope(), job, s.user.id, (date, &time), &empty)
            .await?)
    }

    pub async fn find_draft(
        &self,
        s: &SessionUser,
        id: InspectionFormId,
    ) -> AppResult<InspectionForm> {
        let form = self
            .forms
            .find_draft(s.principal.scope(), id, s.user.id)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Inspection draft")))?;
        self.assert_job_access(s, form.job_id).await?;
        Ok(form)
    }

    pub async fn save_draft(
        &self,
        s: &SessionUser,
        id: InspectionFormId,
        input: &InspectionDraftInput,
    ) -> AppResult<InspectionForm> {
        input.validate().map_err(AppError::Domain)?;
        let form = self.find_draft(s, id).await?;

        // Photo counts decide which rows count as filled in, which in turn
        // decides the note's action status -- so they are read before the
        // note is rendered, not after.
        let counts = self.forms.photo_counts(s.principal.scope(), id).await?;
        let rendered = render(input, form.inspection_date, &counts);

        self.forms
            .save_draft(s.principal.scope(), id, s.user.id, input, &rendered)
            .await?
            .ok_or_else(|| AppError::Domain(stale_revision()))
    }

    /// Step one of attaching photos: presigned PUT URLs.
    ///
    /// The revision is checked here as well as on confirm, so a client that
    /// has fallen behind finds out before uploading rather than after.
    pub async fn prepare_photos(
        &self,
        s: &SessionUser,
        id: InspectionFormId,
        client_key: &str,
        expected_revision: i32,
        files: Vec<UploadRequest>,
    ) -> AppResult<Vec<PreparedInspectionUpload>> {
        validate_batch_size(files.len(), MAX_FILES_PER_UPLOAD).map_err(AppError::Domain)?;
        let form = self.find_draft(s, id).await?;
        if form.revision != expected_revision {
            return Err(AppError::Domain(stale_revision()));
        }
        self.assert_item_exists(&form, client_key)?;

        let mut out = Vec::with_capacity(files.len());
        for file in files {
            let validated = validate_upload(&file).map_err(AppError::Domain)?;
            // Rejected here as well as on attach. A checklist row takes photos
            // only, and finding that out *after* uploading a 500 MB video --
            // which the presigned URL would otherwise happily accept -- is a
            // waste of the site's mobile data.
            assert_photo(validated.kind.file_type)?;
            let key = self.key_for(s, &form, &validated.stored_name);
            let presigned = self
                .store
                .presign_put(&key, validated.kind.mime, validated.size_bytes)
                .await?;
            out.push(PreparedInspectionUpload {
                stored_name: validated.stored_name,
                upload_url: presigned.url,
                expires_in_secs: presigned.expires_in_secs,
                max_bytes: validated.size_bytes,
            });
        }
        Ok(out)
    }

    /// Step two: verify what actually landed, then attach it to the item.
    pub async fn attach_photos(
        &self,
        s: &SessionUser,
        id: InspectionFormId,
        client_key: &str,
        expected_revision: i32,
        uploads: &[InspectionPhotoUpload],
    ) -> AppResult<InspectionForm> {
        if uploads.is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "files",
                "Choose at least one photo",
            )));
        }
        validate_batch_size(uploads.len(), MAX_FILES_PER_UPLOAD).map_err(AppError::Domain)?;

        let form = self.find_draft(s, id).await?;
        if form.revision != expected_revision {
            return Err(AppError::Domain(stale_revision()));
        }
        self.assert_item_exists(&form, client_key)?;

        let mut records = Vec::with_capacity(uploads.len());
        let mut keys: Vec<String> = Vec::with_capacity(uploads.len());
        for u in uploads {
            let key = self.key_for(s, &form, &u.stored_name);
            let verified =
                match upload::verify(self.store.as_ref(), &key, &u.mime_type, &u.original_name)
                    .await
                {
                    Ok(v) => v,
                    Err(e) => {
                        // One bad file fails the batch, so the objects already
                        // accepted in this call do not linger unreferenced.
                        for k in &keys {
                            upload::discard(self.store.as_ref(), k).await;
                        }
                        return Err(e);
                    }
                };
            // Checked again on the way in: `prepare` refuses a video, but a
            // client can presign a JPEG and upload something else, and only
            // the bytes in storage settle what was really sent.
            if let Err(e) = assert_photo(verified.kind.file_type) {
                for k in &keys {
                    upload::discard(self.store.as_ref(), k).await;
                }
                upload::discard(self.store.as_ref(), &key).await;
                return Err(e);
            }
            records.push(InspectionPhotoRecord {
                media: MediaRecord {
                    owner: MediaOwner::Diary {
                        entry: form.diary_entry_id,
                        note: Some(form.diary_note_id),
                    },
                    file_type: verified.kind.file_type,
                    mime_type: verified.kind.mime.to_string(),
                    original_name: u.original_name.clone(),
                    stored_name: u.stored_name.clone(),
                    file_size: verified.size_bytes,
                    url: key.clone(),
                    uploaded_by: s.user.id,
                },
                client_key: client_key.to_string(),
            });
            keys.push(key);
        }

        let saved = self
            .forms
            .attach_photos(
                s.principal.scope(),
                id,
                s.user.id,
                expected_revision,
                &records,
            )
            .await;

        match saved {
            Ok(Some(form)) => Ok(form),
            Ok(None) => {
                // Lost the revision race after uploading: the objects have no
                // rows pointing at them, so remove them now.
                for k in &keys {
                    upload::discard(self.store.as_ref(), k).await;
                }
                Err(AppError::Domain(stale_revision()))
            }
            Err(e) => {
                for k in &keys {
                    upload::discard(self.store.as_ref(), k).await;
                }
                Err(e.into())
            }
        }
    }

    pub async fn delete_photo(
        &self,
        s: &SessionUser,
        id: InspectionFormId,
        expected_revision: i32,
        photo_id: i32,
    ) -> AppResult<InspectionForm> {
        self.find_draft(s, id).await?;
        self.forms
            .delete_photo(
                s.principal.scope(),
                id,
                s.user.id,
                expected_revision,
                photo_id,
            )
            .await?
            .ok_or_else(|| AppError::Domain(stale_revision()))
    }

    /// Photos hang off a saved item, so an unsaved row is a 409 telling the
    /// client to save first -- not a 404, which would read as "wrong URL".
    fn assert_item_exists(&self, form: &InspectionForm, client_key: &str) -> AppResult<()> {
        let key = client_key.trim();
        if key.is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "clientKey",
                "an inspection item is required",
            )));
        }
        if form.items.iter().any(|i| i.client_key == key) {
            return Ok(());
        }
        Err(AppError::Domain(DomainError::Conflict(
            "Save the inspection item before adding photos".to_string(),
        )))
    }

    /// Inspection photos are diary media, so they share the diary key space.
    fn key_for(&self, s: &SessionUser, form: &InspectionForm, stored_name: &str) -> String {
        pmk_domain::media::object_key(
            s.principal.company_id().get(),
            "diary",
            form.diary_entry_id.get(),
            stored_name,
        )
    }
}

/// An inspection item takes photos and nothing else.
fn assert_photo(file_type: pmk_domain::media::FileType) -> AppResult<()> {
    if file_type == pmk_domain::media::FileType::Photo {
        return Ok(());
    }
    Err(AppError::Domain(DomainError::invalid(
        "mimeType",
        "an inspection item accepts photos only",
    )))
}

fn empty_draft() -> InspectionDraftInput {
    InspectionDraftInput {
        inspector: String::new(),
        inspection_type: String::new(),
        stage: String::new(),
        observations: String::new(),
        weather_data: None,
        revision: 0,
        items: Vec::new(),
    }
}

/// Renders everything a draft save writes into the diary.
fn render(
    input: &InspectionDraftInput,
    inspection_date: chrono::NaiveDate,
    counts: &std::collections::HashMap<String, usize>,
) -> RenderedInspection {
    let photos = |k: &str| *counts.get(k).unwrap_or(&0);
    let items = input.retained();
    RenderedInspection {
        note_content: inspection_note(input, inspection_date, &photos),
        entry_summary: inspection_entry_summary(&input.inspection_type, &input.stage),
        action_status: inspection_action_status(&items, &photos).map(|s| s.as_str()),
    }
}
