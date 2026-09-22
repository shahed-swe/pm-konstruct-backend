use std::sync::Arc;

use pmk_domain::ids::{DiaryEntryId, JobId, MediaId};
use pmk_domain::media::{
    signature_matches, validate_batch_size, validate_upload, UploadRequest, ValidatedUpload,
    MAX_FILES_PER_UPLOAD, SIGNATURE_PROBE_BYTES,
};
use pmk_domain::DomainError;
use pmk_ports::repository::{
    DiaryRepository, JobRepository, Media, MediaOwner, MediaRecord, MediaRepository,
};
use pmk_ports::ObjectStore;

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

/// A presigned slot the client uploads into, then confirms.
#[derive(Debug, Clone)]
pub struct PreparedUpload {
    pub upload_url: String,
    pub expires_in_secs: u64,
    pub stored_name: String,
    pub object_key: String,
    pub original_name: String,
    pub mime_type: String,
}

pub struct MediaService {
    media: Arc<dyn MediaRepository>,
    store: Arc<dyn ObjectStore>,
    jobs: Arc<dyn JobRepository>,
    diary: Arc<dyn DiaryRepository>,
}

impl std::fmt::Debug for MediaService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaService").finish_non_exhaustive()
    }
}

impl MediaService {
    #[must_use]
    pub fn new(
        media: Arc<dyn MediaRepository>,
        store: Arc<dyn ObjectStore>,
        jobs: Arc<dyn JobRepository>,
        diary: Arc<dyn DiaryRepository>,
    ) -> Self {
        Self { media, store, jobs, diary }
    }

    /// Confirms the caller can reach the job a diary entry belongs to.
    async fn assert_diary_access(
        &self,
        s: &SessionUser,
        entry: DiaryEntryId,
    ) -> AppResult<()> {
        let visible = if s.user.role.sees_all_company_jobs() {
            None
        } else {
            Some(
                self.jobs
                    .visible_job_ids(s.principal.scope(), s.user.id)
                    .await?,
            )
        };
        self.diary
            .find(s.principal.scope(), visible.as_deref(), entry)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Diary entry")))?;
        Ok(())
    }

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

    fn key_for(&self, s: &SessionUser, owner: MediaOwner, stored_name: &str) -> String {
        let (entity, id) = match owner {
            MediaOwner::Diary { entry, .. } => ("diary", entry.get()),
            MediaOwner::Job { job } => ("job", job.get()),
        };
        pmk_infra_object_key(s.principal.company_id().get(), entity, id, stored_name)
    }

    /// Step one: validate the declared type and size, then hand back a
    /// presigned PUT URL.
    ///
    /// Nothing is recorded yet. A client that presigns and never uploads leaves
    /// no row, only an unused signature that expires.
    pub async fn prepare_uploads(
        &self,
        s: &SessionUser,
        owner: MediaOwner,
        files: Vec<UploadRequest>,
    ) -> AppResult<Vec<PreparedUpload>> {
        validate_batch_size(files.len(), MAX_FILES_PER_UPLOAD).map_err(AppError::Domain)?;

        match owner {
            MediaOwner::Diary { entry, .. } => self.assert_diary_access(s, entry).await?,
            MediaOwner::Job { job } => self.assert_job_access(s, job).await?,
        }

        let mut out = Vec::with_capacity(files.len());
        for f in &files {
            let v: ValidatedUpload = validate_upload(f).map_err(AppError::Domain)?;
            let key = self.key_for(s, owner, &v.stored_name);
            let presigned = self
                .store
                .presign_put(&key, v.kind.mime, v.size_bytes)
                .await?;
            out.push(PreparedUpload {
                upload_url: presigned.url,
                expires_in_secs: presigned.expires_in_secs,
                stored_name: v.stored_name,
                object_key: key,
                original_name: v.original_name,
                mime_type: v.kind.mime.to_string(),
            });
        }
        Ok(out)
    }

    /// Step two: confirm the object is really there, check its content, and
    /// record the row.
    ///
    /// The client's claims are not trusted. The size is read back from storage
    /// rather than taken from the request, and the magic bytes are checked
    /// against the declared MIME type (domain-rules R10) -- which is the whole
    /// point of validating after the upload rather than before it.
    pub async fn confirm_upload(
        &self,
        s: &SessionUser,
        owner: MediaOwner,
        stored_name: &str,
        original_name: &str,
        mime_type: &str,
    ) -> AppResult<Media> {
        match owner {
            MediaOwner::Diary { entry, .. } => self.assert_diary_access(s, entry).await?,
            MediaOwner::Job { job } => self.assert_job_access(s, job).await?,
        }

        let kind = pmk_domain::media::kind_for(mime_type).ok_or_else(|| {
            AppError::Domain(DomainError::invalid("mimeType", "is not an accepted type"))
        })?;

        let key = self.key_for(s, owner, stored_name);

        let head = self
            .store
            .head(&key)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::invalid("storedName", "was not uploaded")))?;

        if head.size_bytes == 0 {
            self.discard(&key).await;
            return Err(AppError::Domain(DomainError::invalid("size", "the file is empty")));
        }
        if head.size_bytes > kind.max_bytes() {
            self.discard(&key).await;
            return Err(AppError::Domain(DomainError::invalid(
                "size",
                "Each photo must be 20 MB or smaller, or each video must be 500 MB or smaller.",
            )));
        }

        let prefix = self.store.read_prefix(&key, SIGNATURE_PROBE_BYTES).await?;
        if !signature_matches(kind.mime, &prefix) {
            // Remove it: an object whose content does not match its declared
            // type must not linger in the bucket.
            self.discard(&key).await;
            return Err(AppError::Domain(DomainError::invalid(
                "file",
                format!("File content does not match the declared type for: {original_name}"),
            )));
        }

        let record = MediaRecord {
            owner,
            file_type: kind.file_type,
            mime_type: kind.mime.to_string(),
            original_name: original_name.to_string(),
            stored_name: stored_name.to_string(),
            file_size: i64::try_from(head.size_bytes).unwrap_or(i64::MAX),
            // The row stores the key, not a signed URL: signed URLs expire,
            // and a stored one would be useless within the hour.
            url: key.clone(),
            uploaded_by: s.user.id,
        };
        Ok(self.media.record(s.principal.scope(), &record).await?)
    }

    /// Best-effort cleanup of a rejected upload. A failure here is logged, not
    /// propagated: the caller's error is the one that matters.
    async fn discard(&self, key: &str) {
        if let Err(e) = self.store.delete(key).await {
            tracing::warn!(key, error = %e, "could not remove a rejected upload");
        }
    }

    pub async fn list_for_diary(
        &self,
        s: &SessionUser,
        entry: DiaryEntryId,
    ) -> AppResult<Vec<Media>> {
        self.assert_diary_access(s, entry).await?;
        Ok(self.media.list_for_diary(s.principal.scope(), entry).await?)
    }

    pub async fn list_for_job(&self, s: &SessionUser, job: JobId) -> AppResult<Vec<Media>> {
        self.assert_job_access(s, job).await?;
        Ok(self.media.list_for_job(s.principal.scope(), job).await?)
    }

    /// A short-lived URL for viewing one file.
    ///
    /// Ownership is re-checked on every request rather than trusting that the
    /// caller got the id from a list they were allowed to see.
    pub async fn download_url(
        &self,
        s: &SessionUser,
        id: MediaId,
        job_media: bool,
    ) -> AppResult<String> {
        let media = self
            .media
            .find(s.principal.scope(), id, job_media)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Media")))?;

        match media.owner {
            MediaOwner::Diary { entry, .. } => self.assert_diary_access(s, entry).await?,
            MediaOwner::Job { job } => self.assert_job_access(s, job).await?,
        }

        Ok(self.store.presign_get(&media.url).await?.url)
    }

    pub async fn delete(&self, s: &SessionUser, id: MediaId, job_media: bool) -> AppResult<()> {
        let media = self
            .media
            .find(s.principal.scope(), id, job_media)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Media")))?;

        match media.owner {
            MediaOwner::Diary { entry, .. } => self.assert_diary_access(s, entry).await?,
            MediaOwner::Job { job } => self.assert_job_access(s, job).await?,
        }

        // The row goes now; the object is swept by the worker. The two cannot
        // be atomic, and an orphaned object costs storage whereas an orphaned
        // row shows the user a broken image.
        if self.media.delete(s.principal.scope(), id, job_media).await? {
            Ok(())
        } else {
            Err(AppError::Domain(DomainError::not_found("Media")))
        }
    }
}

/// Re-exported so the service does not depend on `pmk-infra` directly.
fn pmk_infra_object_key(company_id: i32, entity: &str, entity_id: i32, stored: &str) -> String {
    format!("{company_id}/{entity}/{entity_id}/{stored}")
}
