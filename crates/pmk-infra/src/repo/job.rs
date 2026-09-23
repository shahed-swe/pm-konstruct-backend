//! `JobRepository` over Postgres.
//!
//! Every method opens a transaction and sets the tenant GUC first, so RLS is in
//! force for the whole unit of work. Visibility (domain-rules R3) is expressed
//! in SQL rather than filtered in Rust: a supervisor's query never returns rows
//! it should not see, even transiently.

use async_trait::async_trait;
use pmk_domain::access::Role;
use pmk_domain::ids::{JobId, UserId};
use pmk_domain::job::{Job, JobAssignment, JobInput, JobStatus};
use pmk_domain::tenant::{CompanyId, TenantScope};
use pmk_ports::repository::{AssignedSupervisor, JobFilter, JobPeople, JobRepository};
use pmk_ports::{PortError, PortResult};
use sqlx::{PgPool, Postgres, Transaction};

use super::user::map_sqlx;
use crate::db::tenant::set_tenant;

const JOB_COLUMNS: &str = "id, company_id, name, job_number, client, client_number, client_email, \
     address, status, start_date, end_date, manager_id, supervisor_id, dropbox_path, \
     description, contact2_name, contact2_phone, contact2_email, created_at, updated_at";

#[derive(Debug, sqlx::FromRow)]
struct JobRow {
    id: i32,
    company_id: i32,
    name: String,
    job_number: String,
    client: String,
    client_number: Option<String>,
    client_email: Option<String>,
    address: String,
    status: String,
    start_date: Option<chrono::NaiveDate>,
    end_date: Option<chrono::NaiveDate>,
    manager_id: Option<i32>,
    supervisor_id: Option<i32>,
    dropbox_path: Option<String>,
    description: Option<String>,
    contact2_name: Option<String>,
    contact2_phone: Option<String>,
    contact2_email: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl JobRow {
    fn into_domain(self) -> PortResult<Job> {
        // Constrained by `jobs_status_check`, so an unknown value means the
        // constraint was bypassed -- corrupt data, not user input.
        let status = JobStatus::parse(&self.status).ok_or_else(|| {
            PortError::Storage(format!(
                "job {} has unrecognised status '{}'",
                self.id, self.status
            ))
        })?;
        Ok(Job {
            id: JobId(self.id),
            company_id: CompanyId(self.company_id),
            name: self.name,
            job_number: self.job_number,
            client: self.client,
            client_number: self.client_number,
            client_email: self.client_email,
            address: self.address,
            status,
            start_date: self.start_date,
            end_date: self.end_date,
            manager_id: self.manager_id.map(UserId),
            supervisor_id: self.supervisor_id.map(UserId),
            dropbox_path: self.dropbox_path,
            description: self.description,
            contact2_name: self.contact2_name,
            contact2_phone: self.contact2_phone,
            contact2_email: self.contact2_email,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct AssignmentRow {
    user_id: i32,
    name: Option<String>,
    role: Option<String>,
    is_primary: bool,
    assigned_at: chrono::DateTime<chrono::Utc>,
}

impl AssignmentRow {
    fn into_domain(self) -> JobAssignment {
        JobAssignment {
            user_id: UserId(self.user_id),
            name: self.name.unwrap_or_default(),
            // The join is LEFT because users.id is ON DELETE CASCADE but a race
            // could still produce a missing row; default rather than fail.
            role: self
                .role
                .as_deref()
                .and_then(Role::parse)
                .unwrap_or(Role::Office),
            is_primary: self.is_primary,
            assigned_at: self.assigned_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PgJobRepository {
    pool: PgPool,
}

impl PgJobRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin(&self, scope: TenantScope) -> PortResult<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        Ok(tx)
    }

    /// Reloads assignments and rewrites `jobs.supervisor_id` to match.
    ///
    /// Domain-rules R4: the primary assignment wins, otherwise the earliest.
    /// Runs inside the caller's transaction so the assignment change and the
    /// denormalised column can never disagree.
    async fn sync_primary_supervisor(
        tx: &mut Transaction<'_, Postgres>,
        job: JobId,
    ) -> PortResult<Vec<JobAssignment>> {
        let rows: Vec<AssignmentRow> = sqlx::query_as(
            "SELECT a.user_id, u.name, u.role, a.is_primary, a.assigned_at \
             FROM job_assignments a LEFT JOIN users u ON u.id = a.user_id \
             WHERE a.job_id = $1 ORDER BY a.assigned_at ASC",
        )
        .bind(job.get())
        .fetch_all(&mut **tx)
        .await
        .map_err(map_sqlx)?;

        let assignments: Vec<JobAssignment> =
            rows.into_iter().map(AssignmentRow::into_domain).collect();

        let pairs: Vec<(UserId, bool)> = assignments
            .iter()
            .map(|a| (a.user_id, a.is_primary))
            .collect();
        let primary = pmk_domain::job::primary_supervisor(&pairs);

        sqlx::query("UPDATE jobs SET supervisor_id = $1, updated_at = NOW() WHERE id = $2")
            .bind(primary.map(UserId::get))
            .bind(job.get())
            .execute(&mut **tx)
            .await
            .map_err(map_sqlx)?;

        Ok(assignments)
    }

    /// Confirms the job is inside the tenant *and* visible to this caller.
    ///
    /// RLS already prevents cross-tenant reads; this adds the per-supervisor
    /// rule on top. Returns `NotFound` rather than a forbidden error so job
    /// existence does not leak.
    async fn assert_visible(
        tx: &mut Transaction<'_, Postgres>,
        viewer: UserId,
        sees_all: bool,
        job: JobId,
    ) -> PortResult<()> {
        let visible: Option<(i32,)> = if sees_all {
            sqlx::query_as("SELECT id FROM jobs WHERE id = $1")
                .bind(job.get())
                .fetch_optional(&mut **tx)
                .await
        } else {
            sqlx::query_as(
                "SELECT j.id FROM jobs j \
                 WHERE j.id = $1 AND (j.supervisor_id = $2 \
                   OR EXISTS (SELECT 1 FROM job_assignments a \
                              WHERE a.job_id = j.id AND a.user_id = $2))",
            )
            .bind(job.get())
            .bind(viewer.get())
            .fetch_optional(&mut **tx)
            .await
        }
        .map_err(map_sqlx)?;

        visible.map(|_| ()).ok_or(PortError::NotFound)
    }
}

#[async_trait]
impl JobRepository for PgJobRepository {
    async fn list_visible(
        &self,
        scope: TenantScope,
        viewer: UserId,
        viewer_sees_all: bool,
        filter: &JobFilter,
    ) -> PortResult<Vec<Job>> {
        let mut tx = self.begin(scope).await?;

        // Visibility is part of the WHERE clause, not a post-filter.
        let visibility = if viewer_sees_all {
            "TRUE"
        } else {
            "(j.supervisor_id = $1 OR EXISTS (SELECT 1 FROM job_assignments a \
               WHERE a.job_id = j.id AND a.user_id = $1))"
        };

        let sql = format!(
            "SELECT {JOB_COLUMNS} FROM jobs j \
             WHERE {visibility} \
               AND ($2::text IS NULL OR j.status = $2) \
               -- Archived jobs are hidden unless asked for by name.
               AND ($2::text IS NOT NULL OR j.status <> 'archived') \
               AND ($3::int IS NULL OR j.supervisor_id = $3) \
               AND ($4::text IS NULL OR j.name ILIKE '%' || $4 || '%' \
                    OR j.job_number ILIKE '%' || $4 || '%' \
                    OR j.client ILIKE '%' || $4 || '%') \
             ORDER BY j.created_at DESC"
        );

        let rows: Vec<JobRow> = sqlx::query_as(&sql)
            .bind(viewer.get())
            .bind(filter.status.as_deref())
            .bind(filter.supervisor_id.map(UserId::get))
            .bind(filter.search.as_deref().filter(|s| !s.trim().is_empty()))
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;
        rows.into_iter().map(JobRow::into_domain).collect()
    }

    async fn find(
        &self,
        scope: TenantScope,
        viewer: UserId,
        viewer_sees_all: bool,
        id: JobId,
    ) -> PortResult<Option<Job>> {
        let mut tx = self.begin(scope).await?;
        match Self::assert_visible(&mut tx, viewer, viewer_sees_all, id).await {
            Ok(()) => {}
            Err(PortError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        }
        let sql = format!("SELECT {JOB_COLUMNS} FROM jobs j WHERE id = $1");
        let row: Option<JobRow> = sqlx::query_as(&sql)
            .bind(id.get())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        row.map(JobRow::into_domain).transpose()
    }

    async fn create(&self, scope: TenantScope, input: &JobInput) -> PortResult<Job> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "INSERT INTO jobs (company_id, name, job_number, client, client_number, \
                client_email, address, status, start_date, end_date, manager_id, \
                supervisor_id, dropbox_path, description, contact2_name, contact2_phone, \
                contact2_email) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,COALESCE($8,'active'),$9,$10,$11,$12,$13,$14,$15,$16,$17) \
             RETURNING {JOB_COLUMNS}"
        );
        let row: JobRow = sqlx::query_as(&sql)
            .bind(scope.company_id().get())
            .bind(&input.name)
            .bind(&input.job_number)
            .bind(&input.client)
            .bind(input.client_number.as_deref())
            .bind(input.client_email.as_deref())
            .bind(&input.address)
            .bind(input.status.map(|s| s.as_str()))
            .bind(input.start_date)
            .bind(input.end_date)
            .bind(input.manager_id.map(UserId::get))
            .bind(input.supervisor_id.map(UserId::get))
            .bind(input.dropbox_path.as_deref())
            .bind(input.description.as_deref())
            .bind(input.contact2_name.as_deref())
            .bind(input.contact2_phone.as_deref())
            .bind(input.contact2_email.as_deref())
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        row.into_domain()
    }

    async fn update(
        &self,
        scope: TenantScope,
        id: JobId,
        input: &JobInput,
    ) -> PortResult<Option<Job>> {
        let mut tx = self.begin(scope).await?;
        let sql = format!(
            "UPDATE jobs SET name=$2, job_number=$3, client=$4, client_number=$5, \
                client_email=$6, address=$7, status=COALESCE($8, status), start_date=$9, \
                end_date=$10, manager_id=$11, supervisor_id=$12, dropbox_path=$13, \
                description=$14, contact2_name=$15, contact2_phone=$16, contact2_email=$17, \
                updated_at=NOW() \
             WHERE id=$1 RETURNING {JOB_COLUMNS}"
        );
        let row: Option<JobRow> = sqlx::query_as(&sql)
            .bind(id.get())
            .bind(&input.name)
            .bind(&input.job_number)
            .bind(&input.client)
            .bind(input.client_number.as_deref())
            .bind(input.client_email.as_deref())
            .bind(&input.address)
            .bind(input.status.map(|s| s.as_str()))
            .bind(input.start_date)
            .bind(input.end_date)
            .bind(input.manager_id.map(UserId::get))
            .bind(input.supervisor_id.map(UserId::get))
            .bind(input.dropbox_path.as_deref())
            .bind(input.description.as_deref())
            .bind(input.contact2_name.as_deref())
            .bind(input.contact2_phone.as_deref())
            .bind(input.contact2_email.as_deref())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        row.map(JobRow::into_domain).transpose()
    }

    async fn archive(&self, scope: TenantScope, id: JobId) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        // The default for DELETE /jobs/{id}. Nothing is lost; the job simply
        // stops appearing in the list.
        let r = sqlx::query(
            "UPDATE jobs SET status = 'archived', updated_at = NOW() \
             WHERE id = $1 AND status <> 'archived'",
        )
        .bind(id.get())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }

    async fn purge(&self, scope: TenantScope, id: JobId) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        // Before the delete, not after: the cascade takes the media rows with
        // it, and their `stored_name`s are the only record of what is in
        // storage. Skipping this leaves every photo on the job orphaned in the
        // bucket -- nothing points at them, so nothing ever deletes them.
        let queued =
            super::media_queue::enqueue_job_media(&mut tx, scope.company_id().get(), id.get())
                .await?;
        // ON DELETE CASCADE reaches diary entries, notes, comments, media,
        // call-forward items, tasks, links and scheduler allocations. One job
        // can anchor years of site diary, which is why this is not the default.
        let r = sqlx::query("DELETE FROM jobs WHERE id = $1")
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        if r.rows_affected() > 0 && queued > 0 {
            tracing::info!(
                job = id.get(),
                objects = queued,
                "purge queued media for deletion"
            );
        }
        Ok(r.rows_affected() > 0)
    }

    async fn people(
        &self,
        scope: TenantScope,
        ids: &[JobId],
    ) -> PortResult<std::collections::HashMap<i32, JobPeople>> {
        use std::collections::HashMap;

        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let raw: Vec<i32> = ids.iter().map(|i| i.get()).collect();
        let mut tx = self.begin(scope).await?;

        // The manager's name comes from the job row's own join; the
        // supervisors from the assignment table, ordered the way the legacy
        // ordered them, because the first one is shown as the primary when
        // none is flagged.
        let managers: Vec<(i32, Option<String>)> = sqlx::query_as(
            "SELECT j.id, m.name FROM jobs j \
             LEFT JOIN users m ON m.id = j.manager_id \
             WHERE j.id = ANY($1)",
        )
        .bind(&raw)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        let supervisors: Vec<(i32, i32, Option<String>, bool)> = sqlx::query_as(
            "SELECT a.job_id, a.user_id, u.name, a.is_primary \
             FROM job_assignments a LEFT JOIN users u ON u.id = a.user_id \
             WHERE a.job_id = ANY($1) ORDER BY a.assigned_at ASC",
        )
        .bind(&raw)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)?;

        let mut out: HashMap<i32, JobPeople> = HashMap::with_capacity(managers.len());
        for (job_id, manager_name) in managers {
            out.entry(job_id).or_default().manager_name = manager_name;
        }
        for (job_id, user_id, name, is_primary) in supervisors {
            out.entry(job_id)
                .or_default()
                .supervisors
                .push(AssignedSupervisor {
                    user_id: UserId::new(user_id),
                    // A deleted user leaves the join null. The legacy sent an
                    // empty string here rather than dropping the row, so the
                    // count of supervisors on a job stayed right.
                    name: name.unwrap_or_default(),
                    is_primary,
                });
        }
        Ok(out)
    }

    async fn assignments(&self, scope: TenantScope, id: JobId) -> PortResult<Vec<JobAssignment>> {
        let mut tx = self.begin(scope).await?;
        let rows: Vec<AssignmentRow> = sqlx::query_as(
            "SELECT a.user_id, u.name, u.role, a.is_primary, a.assigned_at \
             FROM job_assignments a LEFT JOIN users u ON u.id = a.user_id \
             WHERE a.job_id = $1 ORDER BY a.assigned_at ASC",
        )
        .bind(id.get())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(AssignmentRow::into_domain).collect())
    }

    async fn add_assignment(
        &self,
        scope: TenantScope,
        id: JobId,
        user: UserId,
        primary: bool,
    ) -> PortResult<Vec<JobAssignment>> {
        let mut tx = self.begin(scope).await?;

        // The first assignment on a job becomes primary automatically, matching
        // the legacy `isPrimary || existing.length === 0`.
        let existing: (i64,) =
            sqlx::query_as("SELECT count(*) FROM job_assignments WHERE job_id = $1")
                .bind(id.get())
                .fetch_one(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        let make_primary = primary || existing.0 == 0;

        // uq_job_assignments_single_primary is a partial unique index, so an
        // existing primary must be cleared in the same transaction.
        if make_primary {
            sqlx::query("UPDATE job_assignments SET is_primary = FALSE WHERE job_id = $1")
                .bind(id.get())
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        }

        sqlx::query("INSERT INTO job_assignments (job_id, user_id, is_primary) VALUES ($1,$2,$3)")
            .bind(id.get())
            .bind(user.get())
            .bind(make_primary)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        let out = Self::sync_primary_supervisor(&mut tx, id).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(out)
    }

    async fn remove_assignment(
        &self,
        scope: TenantScope,
        id: JobId,
        user: UserId,
    ) -> PortResult<Vec<JobAssignment>> {
        let mut tx = self.begin(scope).await?;
        sqlx::query("DELETE FROM job_assignments WHERE job_id = $1 AND user_id = $2")
            .bind(id.get())
            .bind(user.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        // Removing the primary promotes the earliest remaining assignment.
        let out = Self::sync_primary_supervisor(&mut tx, id).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(out)
    }

    async fn set_primary_assignment(
        &self,
        scope: TenantScope,
        id: JobId,
        user: UserId,
    ) -> PortResult<Vec<JobAssignment>> {
        let mut tx = self.begin(scope).await?;

        // Lock the job row first so two concurrent calls serialise. Without
        // this the clear-then-set pair can interleave -- the legacy defect in
        // domain-rules R4.
        sqlx::query("SELECT id FROM jobs WHERE id = $1 FOR UPDATE")
            .bind(id.get())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        sqlx::query("UPDATE job_assignments SET is_primary = FALSE WHERE job_id = $1")
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        let updated = sqlx::query(
            "UPDATE job_assignments SET is_primary = TRUE WHERE job_id = $1 AND user_id = $2",
        )
        .bind(id.get())
        .bind(user.get())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        if updated.rows_affected() == 0 {
            return Err(PortError::NotFound);
        }

        let out = Self::sync_primary_supervisor(&mut tx, id).await?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(out)
    }

    async fn visible_job_ids(&self, scope: TenantScope, viewer: UserId) -> PortResult<Vec<JobId>> {
        let mut tx = self.begin(scope).await?;
        let rows: Vec<(i32,)> = sqlx::query_as(
            "SELECT j.id FROM jobs j \
             WHERE j.supervisor_id = $1 \
                OR EXISTS (SELECT 1 FROM job_assignments a \
                           WHERE a.job_id = j.id AND a.user_id = $1)",
        )
        .bind(viewer.get())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows.into_iter().map(|(id,)| JobId(id)).collect())
    }
}
