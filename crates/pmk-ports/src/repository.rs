//! Repository ports.
//!
//! Every tenant-scoped method takes a `TenantScope`. It cannot be constructed
//! without a verified principal, so omitting tenant scoping is a compile error
//! rather than a cross-tenant leak -- the structural fix for Analysis 4.3.

use async_trait::async_trait;
use pmk_domain::ids::UserId;
use pmk_domain::tenant::{CompanyId, TenantScope};
use pmk_domain::{access::Permission, User};

use crate::PortResult;

/// A stored credential, kept separate from [`User`] so a hash is never carried
/// around inside a value that gets serialised into a response.
#[derive(Debug, Clone)]
pub struct StoredCredential {
    pub user: User,
    pub password_hash: Option<String>,
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Looks a user up by email for login. Deliberately **not** tenant-scoped:
    /// at login time no tenant is known yet. Emails are globally unique.
    async fn find_credential_by_email(&self, email: &str) -> PortResult<Option<StoredCredential>>;

    /// Re-loads a user during token verification. Not tenant-scoped for the
    /// same reason: the scope is derived *from* this result.
    async fn find_by_id_unscoped(&self, id: UserId) -> PortResult<Option<User>>;

    async fn replace_password_hash(&self, id: UserId, hash: &str) -> PortResult<()>;

    async fn permissions_for(&self, scope: TenantScope, id: UserId) -> PortResult<Vec<Permission>>;

    async fn list(&self, scope: TenantScope) -> PortResult<Vec<User>>;
}

/// Refresh-token family, for rotation with reuse detection.
#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub id: i64,
    pub user_id: UserId,
    pub family_id: uuid::Uuid,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub used_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[async_trait]
pub trait RefreshTokenRepository: Send + Sync {
    async fn insert(
        &self,
        user_id: UserId,
        token_hash: &str,
        family_id: uuid::Uuid,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> PortResult<()>;

    async fn find_by_hash(&self, token_hash: &str) -> PortResult<Option<RefreshTokenRecord>>;

    async fn mark_used(&self, id: i64) -> PortResult<()>;

    /// Revokes every token in a family. Called when a used token is presented
    /// again, which means it leaked.
    async fn revoke_family(&self, family_id: uuid::Uuid) -> PortResult<u64>;

    async fn revoke_all_for_user(&self, user_id: UserId) -> PortResult<u64>;

    async fn delete_expired(&self) -> PortResult<u64>;
}

/// Billing entitlement, read on nearly every request (domain-rules R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entitlement {
    pub can_access_application: bool,
    pub can_configure_account: bool,
    pub onboarding_complete: bool,
}

impl Entitlement {
    /// Used until Phase 15 wires Stripe. Every company is entitled, which
    /// matches the legacy default `billing_onboarding_completed = true`.
    #[must_use]
    pub const fn permissive() -> Self {
        Self {
            can_access_application: true,
            can_configure_account: true,
            onboarding_complete: true,
        }
    }
}

#[async_trait]
pub trait BillingRepository: Send + Sync {
    async fn entitlement(&self, company: CompanyId) -> PortResult<Entitlement>;
}

// ── jobs ────────────────────────────────────────────────────────────────────

use pmk_domain::ids::JobId;
use pmk_domain::job::{Job, JobAssignment, JobInput};

/// Filters for the job list, mirroring the legacy query parameters.
#[derive(Debug, Clone, Default)]
pub struct JobFilter {
    pub status: Option<String>,
    pub supervisor_id: Option<UserId>,
    pub search: Option<String>,
}

#[async_trait]
pub trait JobRepository: Send + Sync {
    /// Jobs the caller may see.
    ///
    /// Visibility (domain-rules R3) is applied **inside** the query rather than
    /// filtered afterwards, so a caller cannot receive rows it should not see
    /// even transiently.
    async fn list_visible(
        &self,
        scope: TenantScope,
        viewer: UserId,
        viewer_sees_all: bool,
        filter: &JobFilter,
    ) -> PortResult<Vec<Job>>;

    async fn find(
        &self,
        scope: TenantScope,
        viewer: UserId,
        viewer_sees_all: bool,
        id: JobId,
    ) -> PortResult<Option<Job>>;

    async fn create(&self, scope: TenantScope, input: &JobInput) -> PortResult<Job>;

    async fn update(
        &self,
        scope: TenantScope,
        id: JobId,
        input: &JobInput,
    ) -> PortResult<Option<Job>>;

    /// Archives the job. Rows are kept; it drops out of the default list.
    async fn archive(&self, scope: TenantScope, id: JobId) -> PortResult<bool>;

    /// Deletes for real, cascading to diary, media, call-forward, tasks, links
    /// and scheduler allocations. Manager-only, behind an explicit flag -- see
    /// docs/adr/0002-scope-decisions.md.
    async fn purge(&self, scope: TenantScope, id: JobId) -> PortResult<bool>;

    async fn assignments(&self, scope: TenantScope, id: JobId) -> PortResult<Vec<JobAssignment>>;

    /// Adds an assignment and re-syncs `jobs.supervisor_id` in one transaction.
    ///
    /// The legacy `setPrimary` performed three un-transactioned writes and
    /// could leave a job with two primaries or none (domain-rules R4).
    async fn add_assignment(
        &self,
        scope: TenantScope,
        id: JobId,
        user: UserId,
        primary: bool,
    ) -> PortResult<Vec<JobAssignment>>;

    async fn remove_assignment(
        &self,
        scope: TenantScope,
        id: JobId,
        user: UserId,
    ) -> PortResult<Vec<JobAssignment>>;

    async fn set_primary_assignment(
        &self,
        scope: TenantScope,
        id: JobId,
        user: UserId,
    ) -> PortResult<Vec<JobAssignment>>;

    /// Job ids a supervisor may see: assignments UNION primary-supervisor jobs.
    async fn visible_job_ids(&self, scope: TenantScope, viewer: UserId) -> PortResult<Vec<JobId>>;
}

/// A row of `job_tasks`.
#[derive(Debug, Clone)]
pub struct JobTask {
    pub id: i32,
    pub job_id: JobId,
    pub title: String,
    pub status: String,
    pub notes: Option<String>,
    pub sort_order: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct JobTaskInput {
    pub title: String,
    pub status: Option<String>,
    pub notes: Option<String>,
    pub sort_order: Option<i32>,
}

#[async_trait]
pub trait JobTaskRepository: Send + Sync {
    async fn list(&self, scope: TenantScope, job: JobId) -> PortResult<Vec<JobTask>>;
    async fn create(
        &self,
        scope: TenantScope,
        job: JobId,
        input: &JobTaskInput,
    ) -> PortResult<JobTask>;
    async fn update(
        &self,
        scope: TenantScope,
        id: i32,
        input: &JobTaskInput,
    ) -> PortResult<Option<JobTask>>;
    async fn update_notes(
        &self,
        scope: TenantScope,
        id: i32,
        notes: Option<&str>,
    ) -> PortResult<Option<JobTask>>;
    async fn delete(&self, scope: TenantScope, id: i32) -> PortResult<bool>;
}

// ── site diary ──────────────────────────────────────────────────────────────

use pmk_domain::diary::{
    ActionStatus, DiaryEntry, DiaryEntryInput, DiaryNote, DiaryNoteComment, DiaryNoteInput,
    WeatherStamp,
};
use pmk_domain::ids::{DiaryEntryId, DiaryNoteId};

#[derive(Debug, Clone, Default)]
pub struct DiaryFilter {
    pub job_id: Option<JobId>,
    pub from: Option<chrono::NaiveDate>,
    pub to: Option<chrono::NaiveDate>,
    pub action_status: Option<String>,
}

#[async_trait]
pub trait DiaryRepository: Send + Sync {
    /// Entries on jobs the caller may see. `visible_jobs` is `None` for roles
    /// with company-wide access, otherwise the resolved id set (R3).
    async fn list(
        &self,
        scope: TenantScope,
        visible_jobs: Option<&[JobId]>,
        filter: &DiaryFilter,
    ) -> PortResult<Vec<DiaryEntry>>;

    async fn find(
        &self,
        scope: TenantScope,
        visible_jobs: Option<&[JobId]>,
        id: DiaryEntryId,
    ) -> PortResult<Option<DiaryEntry>>;

    /// Creates an entry. The weather stamp is frozen at creation and never
    /// refreshed (domain-rules R8).
    async fn create(
        &self,
        scope: TenantScope,
        author: UserId,
        input: &DiaryEntryInput,
        weather: &WeatherStamp,
        today: chrono::NaiveDate,
    ) -> PortResult<DiaryEntry>;

    async fn update(
        &self,
        scope: TenantScope,
        id: DiaryEntryId,
        input: &DiaryEntryInput,
    ) -> PortResult<Option<DiaryEntry>>;

    async fn delete(&self, scope: TenantScope, id: DiaryEntryId) -> PortResult<bool>;

    async fn set_action_status(
        &self,
        scope: TenantScope,
        id: DiaryEntryId,
        status: Option<ActionStatus>,
        raised_by: Option<UserId>,
    ) -> PortResult<Option<DiaryEntry>>;

    // -- notes ---------------------------------------------------------------

    async fn notes(
        &self,
        scope: TenantScope,
        entry: DiaryEntryId,
        include_archived: bool,
    ) -> PortResult<Vec<DiaryNote>>;

    async fn add_note(
        &self,
        scope: TenantScope,
        entry: DiaryEntryId,
        input: &DiaryNoteInput,
        raised_by: Option<UserId>,
    ) -> PortResult<DiaryNote>;

    async fn update_note(
        &self,
        scope: TenantScope,
        note: DiaryNoteId,
        input: &DiaryNoteInput,
    ) -> PortResult<Option<DiaryNote>>;

    async fn set_note_action_status(
        &self,
        scope: TenantScope,
        note: DiaryNoteId,
        status: Option<ActionStatus>,
        raised_by: Option<UserId>,
    ) -> PortResult<Option<DiaryNote>>;

    async fn set_note_archived(
        &self,
        scope: TenantScope,
        note: DiaryNoteId,
        archived: bool,
    ) -> PortResult<Option<DiaryNote>>;

    async fn delete_note(&self, scope: TenantScope, note: DiaryNoteId) -> PortResult<bool>;

    // -- comment threads -----------------------------------------------------

    async fn comments(
        &self,
        scope: TenantScope,
        note: DiaryNoteId,
    ) -> PortResult<Vec<DiaryNoteComment>>;

    async fn add_comment(
        &self,
        scope: TenantScope,
        note: DiaryNoteId,
        author: UserId,
        content: &str,
    ) -> PortResult<DiaryNoteComment>;
}

/// Current conditions for the weather stamp.
///
/// A failure here must never block entry creation (domain-rules R8), so the
/// service treats any error as "no stamp" rather than propagating it.
#[async_trait]
pub trait WeatherProvider: Send + Sync {
    async fn current(&self, lat: f64, lon: f64) -> PortResult<WeatherStamp>;
}

// ── call forward ────────────────────────────────────────────────────────────

use pmk_domain::call_forward::{CallForwardInput, CallForwardItem, ItemType};
use pmk_domain::ids::CallForwardItemId;

#[derive(Debug, Clone, Default)]
pub struct CallForwardFilter {
    pub job_id: Option<JobId>,
    pub status: Option<String>,
    pub parent_id: Option<CallForwardItemId>,
}

/// One entry of a bulk reorder.
///
/// `parent_id` is a *double* option on purpose: `None` means the client did not
/// mention parentage and it must be left alone, while `Some(None)` means an
/// explicit detach to the root. Collapsing the two silently reparents every
/// item in a plain sort-order reorder.
#[derive(Debug, Clone, Copy)]
pub struct ReorderEntry {
    pub id: CallForwardItemId,
    pub sort_order: i32,
    pub parent_id: Option<Option<CallForwardItemId>>,
}

#[async_trait]
pub trait CallForwardRepository: Send + Sync {
    async fn list(
        &self,
        scope: TenantScope,
        visible_jobs: Option<&[JobId]>,
        filter: &CallForwardFilter,
    ) -> PortResult<Vec<CallForwardItem>>;

    async fn find(
        &self,
        scope: TenantScope,
        visible_jobs: Option<&[JobId]>,
        id: CallForwardItemId,
    ) -> PortResult<Option<CallForwardItem>>;

    /// Type and job of an item, for parent validation without loading it all.
    async fn type_and_job(
        &self,
        scope: TenantScope,
        id: CallForwardItemId,
    ) -> PortResult<Option<(ItemType, JobId)>>;

    async fn create(
        &self,
        scope: TenantScope,
        job: JobId,
        input: &CallForwardInput,
    ) -> PortResult<CallForwardItem>;

    /// Creates a tree in one transaction. `local_parent` indexes into the same
    /// slice, so a client can describe a hierarchy before any ids exist.
    async fn create_bulk(
        &self,
        scope: TenantScope,
        job: JobId,
        items: &[(CallForwardInput, Option<usize>)],
    ) -> PortResult<Vec<CallForwardItem>>;

    async fn update(
        &self,
        scope: TenantScope,
        id: CallForwardItemId,
        input: &CallForwardInput,
    ) -> PortResult<Option<CallForwardItem>>;

    async fn delete(&self, scope: TenantScope, id: CallForwardItemId) -> PortResult<bool>;

    /// Rewrites sort order and parentage for many items atomically.
    async fn reorder(&self, scope: TenantScope, entries: &[ReorderEntry]) -> PortResult<u64>;

    /// Items due to finish within `days`, for the dashboard.
    async fn upcoming(
        &self,
        scope: TenantScope,
        visible_jobs: Option<&[JobId]>,
        from: chrono::NaiveDate,
        days: i64,
    ) -> PortResult<Vec<CallForwardItem>>;
}

// ── job links (formerly "dropbox folders") ──────────────────────────────────

use pmk_domain::job::{JobLink, JobLinkInput};

/// Cloud-storage links on a job.
///
/// Backed by `job_dropbox_folders`, whose name is historical: the rows are
/// plain shared URLs, never API paths. See docs/audit/integrations-reality.md.
#[async_trait]
pub trait JobLinkRepository: Send + Sync {
    async fn list(&self, scope: TenantScope, job: JobId) -> PortResult<Vec<JobLink>>;
    async fn create(
        &self,
        scope: TenantScope,
        job: JobId,
        input: &JobLinkInput,
    ) -> PortResult<JobLink>;
    async fn update(
        &self,
        scope: TenantScope,
        id: i32,
        input: &JobLinkInput,
    ) -> PortResult<Option<JobLink>>;
    async fn delete(&self, scope: TenantScope, id: i32) -> PortResult<bool>;
}

// ── media ───────────────────────────────────────────────────────────────────

use pmk_domain::ids::MediaId;
use pmk_domain::media::FileType;

/// A stored file, from either `diary_media` or `job_media`.
///
/// The two tables are near-identical but differ in what they hang off: diary
/// media belongs to an entry (and optionally a note), job media belongs
/// directly to a job. The legacy code kept them separate so importing a photo
/// to a job did not need a synthetic diary entry; that split is preserved.
#[derive(Debug, Clone)]
pub struct Media {
    pub id: MediaId,
    pub owner: MediaOwner,
    pub file_type: FileType,
    pub mime_type: String,
    pub original_name: String,
    pub stored_name: String,
    pub file_size: i64,
    pub url: String,
    pub uploaded_by: Option<UserId>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaOwner {
    Diary {
        entry: DiaryEntryId,
        note: Option<DiaryNoteId>,
    },
    Job {
        job: JobId,
    },
}

/// What the API records once an upload is confirmed present in storage.
#[derive(Debug, Clone)]
pub struct MediaRecord {
    pub owner: MediaOwner,
    pub file_type: FileType,
    pub mime_type: String,
    pub original_name: String,
    pub stored_name: String,
    pub file_size: i64,
    pub url: String,
    pub uploaded_by: UserId,
}

#[async_trait]
pub trait MediaRepository: Send + Sync {
    async fn list_for_diary(
        &self,
        scope: TenantScope,
        entry: DiaryEntryId,
    ) -> PortResult<Vec<Media>>;

    async fn list_for_job(&self, scope: TenantScope, job: JobId) -> PortResult<Vec<Media>>;

    async fn find(
        &self,
        scope: TenantScope,
        id: MediaId,
        job_media: bool,
    ) -> PortResult<Option<Media>>;

    async fn record(&self, scope: TenantScope, rec: &MediaRecord) -> PortResult<Media>;

    /// Deletes the row and enqueues the object for removal.
    ///
    /// The two cannot be done atomically -- storage is not in the transaction --
    /// so the row goes first and the object is swept by the worker. An orphaned
    /// object costs storage; an orphaned row shows the user a broken image.
    async fn delete(&self, scope: TenantScope, id: MediaId, job_media: bool) -> PortResult<bool>;

    /// Objects awaiting deletion, oldest first.
    async fn pending_deletions(&self, limit: i64) -> PortResult<Vec<String>>;

    async fn mark_deleted(&self, stored_name: &str) -> PortResult<()>;

    async fn mark_deletion_failed(&self, stored_name: &str, error: &str) -> PortResult<()>;

    /// The job's Files tab: uploaded documents plus inspection drafts.
    async fn job_files(&self, scope: TenantScope, job: JobId) -> PortResult<Vec<JobFile>>;

    /// Deletes several media rows on one job, all or nothing.
    ///
    /// `uploaded_by` restricts the caller to their own uploads; `None` means
    /// no restriction. Nothing is deleted if any selection is missing or
    /// forbidden -- a partial delete would leave the user unable to tell what
    /// went and what did not.
    async fn delete_many(
        &self,
        scope: TenantScope,
        job: JobId,
        selections: &[MediaSelection],
        uploaded_by: Option<UserId>,
    ) -> PortResult<BulkDeletePlan>;
}

// ── trade scheduler ─────────────────────────────────────────────────────────

use pmk_domain::ids::{SchedulerAbsenceId, SchedulerAllocationId, SchedulerWorkerId};
use pmk_domain::scheduler::{
    Absence, AbsenceInput, Allocation, AllocationInput, Worker, WorkerInput,
};

/// A maintenance job: scheduler-only work that is not a real construction job.
#[derive(Debug, Clone)]
pub struct MaintenanceJob {
    pub id: i32,
    pub name: String,
    pub reference: Option<String>,
    pub address: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct MaintenanceJobInput {
    pub name: String,
    pub reference: Option<String>,
    pub address: Option<String>,
    pub status: Option<String>,
}

/// A free-text note pinned to one job on one day.
#[derive(Debug, Clone)]
pub struct DayNote {
    pub id: i32,
    pub job_id: JobId,
    pub note_date: chrono::NaiveDate,
    pub note: String,
}

/// The window the board renders.
#[derive(Debug, Clone, Copy)]
pub struct DateRange {
    pub from: chrono::NaiveDate,
    pub to: chrono::NaiveDate,
}

/// Everything one board render needs, fetched together.
///
/// The legacy implementation issued a query per worker per day. This returns
/// the whole window at once so a 20-worker month is a handful of round-trips
/// rather than hundreds.
#[derive(Debug, Clone, Default)]
pub struct BoardData {
    pub workers: Vec<Worker>,
    pub absences: Vec<Absence>,
    pub allocations: Vec<Allocation>,
    pub day_notes: Vec<DayNote>,
}

#[async_trait]
pub trait SchedulerRepository: Send + Sync {
    // -- workers -------------------------------------------------------------
    async fn workers(&self, scope: TenantScope, include_inactive: bool) -> PortResult<Vec<Worker>>;
    async fn create_worker(&self, scope: TenantScope, input: &WorkerInput) -> PortResult<Worker>;
    async fn update_worker(
        &self,
        scope: TenantScope,
        id: SchedulerWorkerId,
        input: &WorkerInput,
    ) -> PortResult<Option<Worker>>;
    /// Soft delete: `active = false`. Allocation history is preserved.
    async fn deactivate_worker(
        &self,
        scope: TenantScope,
        id: SchedulerWorkerId,
    ) -> PortResult<bool>;
    /// Hard delete, cascading to allocations and absences.
    async fn purge_worker(&self, scope: TenantScope, id: SchedulerWorkerId) -> PortResult<bool>;

    // -- absences ------------------------------------------------------------
    async fn absences(&self, scope: TenantScope, range: DateRange) -> PortResult<Vec<Absence>>;
    async fn create_absence(&self, scope: TenantScope, input: &AbsenceInput)
        -> PortResult<Absence>;
    async fn update_absence(
        &self,
        scope: TenantScope,
        id: SchedulerAbsenceId,
        input: &AbsenceInput,
    ) -> PortResult<Option<Absence>>;
    async fn delete_absence(&self, scope: TenantScope, id: SchedulerAbsenceId) -> PortResult<bool>;

    // -- allocations ---------------------------------------------------------
    async fn allocations(
        &self,
        scope: TenantScope,
        range: DateRange,
    ) -> PortResult<Vec<Allocation>>;
    async fn create_allocation(
        &self,
        scope: TenantScope,
        input: &AllocationInput,
    ) -> PortResult<Allocation>;
    async fn update_allocation(
        &self,
        scope: TenantScope,
        id: SchedulerAllocationId,
        input: &AllocationInput,
    ) -> PortResult<Option<Allocation>>;
    async fn delete_allocation(
        &self,
        scope: TenantScope,
        id: SchedulerAllocationId,
    ) -> PortResult<bool>;

    // -- maintenance jobs ----------------------------------------------------
    async fn maintenance_jobs(&self, scope: TenantScope) -> PortResult<Vec<MaintenanceJob>>;
    async fn create_maintenance_job(
        &self,
        scope: TenantScope,
        input: &MaintenanceJobInput,
    ) -> PortResult<MaintenanceJob>;
    async fn update_maintenance_job(
        &self,
        scope: TenantScope,
        id: i32,
        input: &MaintenanceJobInput,
    ) -> PortResult<Option<MaintenanceJob>>;
    async fn delete_maintenance_job(&self, scope: TenantScope, id: i32) -> PortResult<bool>;

    // -- day notes -----------------------------------------------------------
    async fn day_notes(&self, scope: TenantScope, range: DateRange) -> PortResult<Vec<DayNote>>;
    /// Upserts: one note per job per day (`uq_scheduler_job_day_note`).
    async fn set_day_note(
        &self,
        scope: TenantScope,
        job: JobId,
        date: chrono::NaiveDate,
        note: &str,
    ) -> PortResult<DayNote>;
    async fn delete_day_note(&self, scope: TenantScope, id: i32) -> PortResult<bool>;

    /// One round-trip per collection, for a whole board window.
    async fn board(&self, scope: TenantScope, range: DateRange) -> PortResult<BoardData>;
}

// ── forms ───────────────────────────────────────────────────────────────────

use pmk_domain::forms::{EtoInput, InspectionDraftInput};
use pmk_domain::ids::{InspectionFormId, InspectionFormItemId};

/// What raising an ETO produced.
#[derive(Debug, Clone)]
pub struct EtoRaised {
    pub entry_id: DiaryEntryId,
    pub note_id: DiaryNoteId,
    pub eto_number: i32,
    pub po_number: String,
    pub raised_by: String,
}

/// One approved ETO, as the job's ETO list shows it.
#[derive(Debug, Clone)]
pub struct EtoListItem {
    pub note_id: DiaryNoteId,
    pub entry_id: DiaryEntryId,
    /// Parsed back out of the note text -- it was never stored as a column.
    pub eto_number: String,
    pub content: String,
    pub raised_by: String,
    pub diary_date: chrono::NaiveDate,
    pub approved_at: chrono::DateTime<chrono::Utc>,
    pub job_number: Option<String>,
    pub job_name: Option<String>,
    pub job_address: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InspectionPhoto {
    pub id: i32,
    pub item_id: InspectionFormItemId,
    pub diary_media_id: MediaId,
    pub sort_order: i32,
    pub original_name: String,
    pub mime_type: String,
    pub file_size: i64,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct InspectionItem {
    pub id: InspectionFormItemId,
    pub client_key: String,
    pub room: String,
    pub description: String,
    pub actioned: bool,
    pub sort_order: i32,
    pub photos: Vec<InspectionPhoto>,
}

#[derive(Debug, Clone)]
pub struct InspectionForm {
    pub id: InspectionFormId,
    pub company_id: i32,
    pub job_id: JobId,
    pub created_by: UserId,
    pub diary_entry_id: DiaryEntryId,
    pub diary_note_id: DiaryNoteId,
    pub inspection_date: chrono::NaiveDate,
    pub inspector: String,
    pub inspection_type: String,
    pub stage: String,
    pub observations: String,
    pub weather_data: Option<serde_json::Value>,
    /// Bumped by every write. The client echoes it back, and a mismatch is a
    /// 409 rather than a silent overwrite of someone else's edit.
    pub revision: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub items: Vec<InspectionItem>,
}

/// A photo about to be attached to an inspection item.
#[derive(Debug, Clone)]
pub struct InspectionPhotoRecord {
    pub media: MediaRecord,
    pub client_key: String,
}

#[async_trait]
pub trait FormsRepository: Send + Sync {
    /// Raises an ETO: allocates the next per-job number, writes the diary
    /// entry and its `eto` note, all in one transaction.
    ///
    /// The whole thing is one call because the number allocation (R7) and the
    /// note that carries it must commit together -- a diary entry naming a
    /// number the sequence never issued, or a burnt number with no note, are
    /// both worse than failing.
    ///
    /// `stamp` is the diary entry's date and `HH:MM` time, both in the
    /// company's timezone. It comes from the caller's `Clock` rather than
    /// `now()` here so one zone decides every date boundary.
    async fn raise_eto(
        &self,
        scope: TenantScope,
        author: UserId,
        author_name: &str,
        input: &EtoInput,
        stamp: (chrono::NaiveDate, &str),
    ) -> PortResult<EtoRaised>;

    /// Approved, unarchived ETOs on a job, newest first.
    async fn etos_for_job(&self, scope: TenantScope, job: JobId) -> PortResult<Vec<EtoListItem>>;

    /// The caller's draft for a job, creating one if they have none.
    ///
    /// One draft per (company, job, user) by
    /// `inspection_forms_user_job_unique`: two supervisors inspect the same
    /// job independently, but one person opening the form twice must land on
    /// the same draft rather than forking it.
    ///
    /// `stamp` carries the new entry's date and `HH:MM` time; `empty_note` is
    /// the rendered note body for a draft with nothing in it yet.
    async fn get_or_create_draft(
        &self,
        scope: TenantScope,
        job: JobId,
        user: UserId,
        stamp: (chrono::NaiveDate, &str),
        empty_note: &str,
    ) -> PortResult<InspectionForm>;

    /// Reads a draft the caller owns.
    async fn find_draft(
        &self,
        scope: TenantScope,
        id: InspectionFormId,
        user: UserId,
    ) -> PortResult<Option<InspectionForm>>;

    /// Saves a draft wholesale, reconciling its items.
    ///
    /// `Ok(None)` means the revision did not match: someone else saved first.
    /// The rendered note text is passed in rather than built here so the
    /// renderer stays in the domain.
    async fn save_draft(
        &self,
        scope: TenantScope,
        id: InspectionFormId,
        user: UserId,
        input: &InspectionDraftInput,
        rendered: &RenderedInspection,
    ) -> PortResult<Option<InspectionForm>>;

    /// Attaches uploaded photos to one item, bumping the revision.
    ///
    /// `Ok(None)` on a revision mismatch.
    async fn attach_photos(
        &self,
        scope: TenantScope,
        id: InspectionFormId,
        user: UserId,
        expected_revision: i32,
        photos: &[InspectionPhotoRecord],
    ) -> PortResult<Option<InspectionForm>>;

    /// Detaches a photo and queues its object for deletion.
    ///
    /// `Ok(None)` on a revision mismatch; `NotFound` if the photo is not on
    /// this form.
    async fn delete_photo(
        &self,
        scope: TenantScope,
        id: InspectionFormId,
        user: UserId,
        expected_revision: i32,
        photo_id: i32,
    ) -> PortResult<Option<InspectionForm>>;

    /// Photo counts per `client_key`, for rendering the note text.
    async fn photo_counts(
        &self,
        scope: TenantScope,
        id: InspectionFormId,
    ) -> PortResult<std::collections::HashMap<String, usize>>;
}

/// The strings a draft save writes into the diary, rendered by the domain.
#[derive(Debug, Clone)]
pub struct RenderedInspection {
    pub note_content: String,
    pub entry_summary: String,
    /// `None` clears the action flag -- an untouched draft is not an action.
    pub action_status: Option<&'static str>,
}

// ── dashboard ───────────────────────────────────────────────────────────────

use pmk_domain::dashboard::{ActionItem, DashboardStats, JobScope};

/// A job as the dashboard drill-down lists it.
#[derive(Debug, Clone)]
pub struct DashboardJob {
    pub id: JobId,
    pub job_number: Option<String>,
    pub address: Option<String>,
    pub name: Option<String>,
    pub status: String,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
}

/// A call-forward item as the dashboard drill-down lists it.
#[derive(Debug, Clone)]
pub struct DashboardCallForward {
    pub id: i32,
    pub job_id: JobId,
    pub job_number: Option<String>,
    pub job_address: Option<String>,
    pub title: String,
    pub item_type: String,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub status: String,
}

/// A diary entry as the dashboard drill-down lists it.
#[derive(Debug, Clone)]
pub struct DashboardDiaryEntry {
    pub id: DiaryEntryId,
    pub job_id: JobId,
    pub job_number: Option<String>,
    pub job_address: Option<String>,
    pub date: chrono::NaiveDate,
    pub work_completed: Option<String>,
    pub author_name: Option<String>,
}

/// A stage claim falling due.
#[derive(Debug, Clone)]
pub struct UpcomingClaim {
    pub id: i32,
    pub job_id: JobId,
    pub job_name: Option<String>,
    pub job_address: Option<String>,
    pub job_number: Option<String>,
    pub title: String,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_start: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub status: String,
    pub notes: Option<String>,
}

/// Which jobs a drill-down list covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobListFilter {
    /// Every job in scope, whatever its status.
    All,
    Active,
    Completed,
}

#[async_trait]
pub trait DashboardRepository: Send + Sync {
    async fn stats(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        diary_since: chrono::NaiveDate,
        include_user_count: bool,
    ) -> PortResult<DashboardStats>;

    async fn jobs_list(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: JobListFilter,
    ) -> PortResult<Vec<DashboardJob>>;

    /// Open items: `not_started` or `in_progress`, headers excluded.
    async fn open_call_forwards(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
    ) -> PortResult<Vec<DashboardCallForward>>;

    /// Past their estimated finish, not finished, not on hold.
    async fn overdue_call_forwards(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<DashboardCallForward>>;

    /// How many days late each overdue item is, for bucketing.
    async fn overdue_days(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<i64>>;

    async fn recent_diary(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        since: chrono::NaiveDate,
    ) -> PortResult<Vec<DashboardDiaryEntry>>;

    /// Stage claims due on or before `cutoff`, still open.
    ///
    /// Deliberately unbounded below: an overdue claim stays visible until it
    /// is marked complete, rather than dropping off the dashboard.
    async fn upcoming_claims(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        cutoff: chrono::NaiveDate,
    ) -> PortResult<Vec<UpcomingClaim>>;

    /// Unarchived notes carrying an action status, newest entry first.
    async fn action_items(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
    ) -> PortResult<Vec<ActionItem>>;
}

// ── calendar ────────────────────────────────────────────────────────────────

use pmk_domain::dashboard::calendar::{
    CalendarFilterJob, CalendarFilterSupervisor, EventKind, MonthWindow,
};

/// A job that overlaps the calendar window.
#[derive(Debug, Clone)]
pub struct CalendarJobRow {
    pub id: JobId,
    pub name: Option<String>,
    pub job_number: Option<String>,
    pub address: Option<String>,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub status: String,
    pub supervisor_id: Option<UserId>,
    pub supervisor_name: Option<String>,
}

/// A call-forward item that overlaps the calendar window.
#[derive(Debug, Clone)]
pub struct CalendarItemRow {
    pub id: i32,
    pub title: String,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_start: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub status: String,
    pub supplier_trade: Option<String>,
    pub job_id: JobId,
    pub job_name: Option<String>,
    pub job_number: Option<String>,
    pub job_address: Option<String>,
    pub supervisor_id: Option<UserId>,
    pub supervisor_name: Option<String>,
}

/// Narrows a calendar query beyond the caller's own visibility.
#[derive(Debug, Clone, Copy, Default)]
pub struct CalendarFilter {
    /// One job only. Rejected with 403 if it is outside the caller's scope.
    pub job_id: Option<JobId>,
    /// Jobs whose primary supervisor is this user.
    pub supervisor_id: Option<UserId>,
}

#[async_trait]
pub trait CalendarRepository: Send + Sync {
    async fn jobs_in_window(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        window: MonthWindow,
        filter: CalendarFilter,
    ) -> PortResult<Vec<CalendarJobRow>>;

    /// Call-forward items of one kind overlapping the window.
    ///
    /// Overlap is measured on the *effective* dates -- coalesced across all
    /// four columns -- so an item carrying only an estimated start is still
    /// found.
    async fn items_in_window(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        window: MonthWindow,
        kind: EventKind,
        filter: CalendarFilter,
    ) -> PortResult<Vec<CalendarItemRow>>;

    async fn filter_jobs(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
    ) -> PortResult<Vec<CalendarFilterJob>>;

    /// Supervisors reachable from the caller's jobs: primary supervisors
    /// union assignees, deduplicated, ordered by name.
    async fn filter_supervisors(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
    ) -> PortResult<Vec<CalendarFilterSupervisor>>;
}

// ── progress ────────────────────────────────────────────────────────────────

use pmk_domain::ids::ProgressId;
use pmk_domain::progress::{Progress, ProgressInput};

#[async_trait]
pub trait ProgressRepository: Send + Sync {
    /// Records for the jobs in scope, oldest date first.
    ///
    /// `job` narrows to one job, which the caller has already checked they
    /// can see.
    async fn list(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        job: Option<JobId>,
    ) -> PortResult<Vec<Progress>>;

    async fn find(&self, scope: TenantScope, id: ProgressId) -> PortResult<Option<Progress>>;

    async fn create(&self, scope: TenantScope, input: &ProgressInput) -> PortResult<Progress>;

    /// Updates everything but `job_id`.
    ///
    /// A record belongs to the job it was raised against; moving it to another
    /// job would rewrite that job's history, so the field is not updatable.
    async fn update(
        &self,
        scope: TenantScope,
        id: ProgressId,
        input: &ProgressInput,
    ) -> PortResult<Option<Progress>>;

    async fn delete(&self, scope: TenantScope, id: ProgressId) -> PortResult<bool>;
}

// ── job files ───────────────────────────────────────────────────────────────

/// A row of the job's Files tab.
///
/// Two very different things share this shape: uploaded documents, and
/// inspection drafts, which are not files at all but are listed beside them so
/// the tab shows everything attached to the job. The synthetic rows carry a
/// `stored_name` of `inspection-form:{id}` and a zero size, which is how the
/// legacy UI tells them apart.
#[derive(Debug, Clone)]
pub struct JobFile {
    pub id: i32,
    pub file_type: String,
    pub mime_type: String,
    pub original_name: String,
    pub stored_name: String,
    pub file_size: i64,
    pub url: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub uploader_name: Option<String>,
}

/// One item in a bulk media delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaSelection {
    /// `true` for `job_media`, `false` for `diary_media`.
    pub job_media: bool,
    pub id: MediaId,
}

/// What a bulk delete found before deciding whether to proceed.
#[derive(Debug, Clone, Default)]
pub struct BulkDeletePlan {
    /// Selections with no matching row on this job.
    pub missing: Vec<MediaSelection>,
    /// Rows the caller may not remove.
    pub forbidden: Vec<MediaSelection>,
    /// Rows that were deleted, with the objects queued for sweeping.
    pub deleted: Vec<MediaSelection>,
}

// ── reports ─────────────────────────────────────────────────────────────────

use pmk_domain::ids::ReportId;
use pmk_domain::reports::ReportRange;

/// Narrows a report beyond the caller's own visibility.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReportFilter {
    pub job_id: Option<JobId>,
    /// Jobs this person supervises, whether as primary or by assignment.
    pub supervisor_id: Option<UserId>,
    /// Diary author, on the reports that are about entries rather than jobs.
    pub author_id: Option<UserId>,
    pub range: ReportRange,
}

/// A stored, generated report.
#[derive(Debug, Clone)]
pub struct StoredReport {
    pub id: ReportId,
    pub job_id: JobId,
    pub report_type: String,
    pub title: String,
    /// Free text, not JSON: the column is `text NOT NULL` and the legacy wrote
    /// whatever the generator produced into it.
    pub content: String,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct StoredReportInput {
    pub job_id: JobId,
    pub report_type: String,
    pub title: String,
    pub content: String,
}

/// The filter dropdowns every report shares.
#[derive(Debug, Clone, Default)]
pub struct ReportMeta {
    pub jobs: Vec<ReportMetaJob>,
    pub supervisors: Vec<CalendarFilterSupervisor>,
}

#[derive(Debug, Clone)]
pub struct ReportMetaJob {
    pub id: JobId,
    pub name: String,
    pub job_number: String,
    pub status: String,
    pub address: Option<String>,
}

/// One job's line on the progress report, already aggregated.
#[derive(Debug, Clone)]
pub struct JobProgressRow {
    pub job_id: JobId,
    pub job_number: String,
    pub job_name: String,
    pub client: Option<String>,
    pub address: Option<String>,
    pub status: String,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub supervisor_id: Option<UserId>,
    pub supervisor_name: Option<String>,
    pub total_tasks: i64,
    pub not_started: i64,
    pub in_progress: i64,
    pub completed: i64,
    pub on_hold: i64,
    pub delayed_count: i64,
}

/// An overdue call-forward item.
#[derive(Debug, Clone)]
pub struct DelayRow {
    pub task_id: i32,
    pub title: String,
    pub item_type: String,
    pub supplier_trade: Option<String>,
    pub job_id: JobId,
    pub job_number: String,
    pub job_name: String,
    pub job_address: Option<String>,
    pub supervisor_id: Option<UserId>,
    pub supervisor_name: Option<String>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub delay_days: i64,
    pub status: String,
}

/// A diary entry with its notes rolled up by category.
#[derive(Debug, Clone)]
pub struct DiaryReportRow {
    pub id: DiaryEntryId,
    pub date: chrono::NaiveDate,
    pub job_id: JobId,
    pub job_number: String,
    pub job_name: String,
    pub job_address: Option<String>,
    pub author_id: Option<UserId>,
    pub author_name: Option<String>,
    pub weather: Option<String>,
    pub workforce: Option<i32>,
    pub work_completed: String,
    pub materials: String,
    pub trades_on_site: String,
    pub safety_notes: String,
    pub client_instructions: String,
    pub issues: String,
    pub notes: String,
}

/// A call-forward item that has not started yet.
#[derive(Debug, Clone)]
pub struct UpcomingTaskRow {
    pub task_id: i32,
    pub title: String,
    pub item_type: String,
    pub supplier_trade: Option<String>,
    pub job_id: JobId,
    pub job_number: String,
    pub job_name: String,
    pub job_address: Option<String>,
    pub supervisor_id: Option<UserId>,
    pub supervisor_name: Option<String>,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub days_until_start: Option<i64>,
    pub days_until_finish: Option<i64>,
    pub status: String,
}

/// A stage claim with the month it is forecast to fall in.
#[derive(Debug, Clone)]
pub struct StageClaimRow {
    pub id: i32,
    pub title: String,
    pub job_id: JobId,
    pub job_number: String,
    pub job_name: String,
    pub job_address: Option<String>,
    pub supervisor_name: Option<String>,
    pub supplier_trade: Option<String>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub status: String,
}

/// A diary entry viewed as an inspection.
#[derive(Debug, Clone)]
pub struct InspectionRow {
    pub id: DiaryEntryId,
    pub date: chrono::NaiveDate,
    pub job_id: JobId,
    pub job_number: String,
    pub job_name: String,
    pub job_address: Option<String>,
    pub inspector: Option<String>,
    pub safety_notes: Option<String>,
    pub issues: Option<String>,
    pub workforce: Option<i32>,
    pub trades_on_site: Option<String>,
}

/// A diary entry's weather record.
#[derive(Debug, Clone)]
pub struct WeatherRow {
    pub id: DiaryEntryId,
    pub date: chrono::NaiveDate,
    pub job_id: JobId,
    pub job_number: String,
    pub job_name: String,
    pub author_name: Option<String>,
    pub location_name: Option<String>,
    pub weather_condition: Option<String>,
    pub temperature: Option<f64>,
    pub rainfall_mm: Option<f64>,
    pub wind_speed_kmh: Option<f64>,
    pub issues: Option<String>,
    pub workforce: Option<i32>,
}

/// The raw counts behind one supervisor's performance line.
#[derive(Debug, Clone)]
pub struct SupervisorCounts {
    pub supervisor_id: UserId,
    pub name: String,
    pub email: String,
    pub active_jobs: i64,
    pub completed_jobs: i64,
    pub total_jobs: i64,
    pub started_tasks: i64,
    pub started_on_time: i64,
    pub finished_tasks: i64,
    pub finished_on_time: i64,
    pub total_tasks: i64,
    pub delayed_tasks: i64,
    pub total_delay_days: i64,
    pub diary_entries: i64,
    /// Job windows, for counting the working days a diary was expected on.
    pub job_windows: Vec<(chrono::NaiveDate, chrono::NaiveDate)>,
}

#[async_trait]
pub trait ReportsRepository: Send + Sync {
    async fn stored(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        job: Option<JobId>,
    ) -> PortResult<Vec<StoredReport>>;

    async fn create_stored(
        &self,
        scope: TenantScope,
        input: &StoredReportInput,
    ) -> PortResult<StoredReport>;

    async fn meta(&self, scope: TenantScope, jobs: &JobScope) -> PortResult<ReportMeta>;

    async fn job_progress(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<JobProgressRow>>;

    async fn delays(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<DelayRow>>;

    async fn diary_summary(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
    ) -> PortResult<Vec<DiaryReportRow>>;

    async fn upcoming_tasks(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<UpcomingTaskRow>>;

    async fn stage_claims(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
    ) -> PortResult<Vec<StageClaimRow>>;

    async fn inspections(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
    ) -> PortResult<Vec<InspectionRow>>;

    async fn weather(
        &self,
        scope: TenantScope,
        jobs: &JobScope,
        filter: ReportFilter,
    ) -> PortResult<Vec<WeatherRow>>;

    /// Everything the daily-progress report needs for one job on one day.
    async fn daily_progress(
        &self,
        scope: TenantScope,
        job: JobId,
        date: chrono::NaiveDate,
    ) -> PortResult<Option<DailyProgressData>>;

    async fn supervisor_counts(
        &self,
        scope: TenantScope,
        filter: ReportFilter,
        today: chrono::NaiveDate,
    ) -> PortResult<Vec<SupervisorCounts>>;
}

/// The job, its programme and the day's diary, for the daily report.
#[derive(Debug, Clone)]
pub struct DailyProgressData {
    pub job_id: JobId,
    pub job_number: String,
    pub job_name: String,
    pub client: Option<String>,
    pub supervisor_id: Option<UserId>,
    pub supervisor_name: Option<String>,
    pub tasks: Vec<DailyTaskRow>,
    pub diary: Option<DiaryReportRow>,
}

/// One programme item, as the daily report sees it.
#[derive(Debug, Clone)]
pub struct DailyTaskRow {
    pub id: i32,
    pub title: String,
    pub item_type: String,
    pub supplier_trade: Option<String>,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_start: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub status: String,
    pub sort_order: i32,
}

// ── notifications ───────────────────────────────────────────────────────────

use pmk_domain::ids::NotificationId;
use pmk_domain::notifications::{Notification, NotificationPrefs, PushSubscription};

/// A notification about to be raised.
#[derive(Debug, Clone)]
pub struct NotificationInput {
    pub user_id: UserId,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    pub link: Option<String>,
}

#[async_trait]
pub trait NotificationRepository: Send + Sync {
    /// One user's notifications, newest first.
    async fn for_user(
        &self,
        scope: TenantScope,
        user: UserId,
        limit: i64,
    ) -> PortResult<Vec<Notification>>;

    async fn unread_count(&self, scope: TenantScope, user: UserId) -> PortResult<i64>;

    /// Marks one as read. `false` if it is not this user's.
    async fn mark_read(
        &self,
        scope: TenantScope,
        user: UserId,
        id: NotificationId,
    ) -> PortResult<bool>;

    async fn mark_all_read(&self, scope: TenantScope, user: UserId) -> PortResult<i64>;

    /// Raises notifications for several users at once.
    ///
    /// Batched because one diary note can notify every manager on a job, and a
    /// row per statement would make that N round-trips.
    async fn create_many(
        &self,
        scope: TenantScope,
        inputs: &[NotificationInput],
    ) -> PortResult<Vec<Notification>>;

    async fn prefs(&self, scope: TenantScope, user: UserId) -> PortResult<NotificationPrefs>;

    async fn set_prefs(
        &self,
        scope: TenantScope,
        user: UserId,
        prefs: NotificationPrefs,
    ) -> PortResult<NotificationPrefs>;

    async fn save_subscription(
        &self,
        scope: TenantScope,
        user: UserId,
        sub: &PushSubscription,
    ) -> PortResult<()>;

    async fn remove_subscription(
        &self,
        scope: TenantScope,
        user: UserId,
        endpoint: &str,
    ) -> PortResult<bool>;

    /// Subscriptions to deliver to, for the push worker.
    async fn subscriptions_for(
        &self,
        scope: TenantScope,
        users: &[UserId],
    ) -> PortResult<Vec<(UserId, PushSubscription)>>;
}
