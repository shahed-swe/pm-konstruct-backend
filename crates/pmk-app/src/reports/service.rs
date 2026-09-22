use std::sync::Arc;

use pmk_domain::dashboard::JobScope;
use pmk_domain::ids::{JobId, UserId};
use pmk_domain::reports::performance::{
    diary_compliance, mean_1dp, mean_days, percentage, performance_score, working_days,
};
use pmk_domain::reports::{
    completion_pct, job_health, round_1dp, was_rainy, weather_impact, JobHealth, ReportRange,
    Severity,
};
use pmk_domain::DomainError;
use pmk_ports::repository::{
    DailyProgressData, DelayRow, DiaryReportRow, InspectionRow, JobProgressRow, JobRepository,
    ReportFilter, ReportMeta, ReportsRepository, StageClaimRow, StoredReport, StoredReportInput,
    UpcomingTaskRow, WeatherRow,
};
use pmk_ports::Clock;

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

/// What a report was asked for.
#[derive(Debug, Clone, Default)]
pub struct ReportQuery {
    pub job_id: Option<i32>,
    pub supervisor_id: Option<i32>,
    pub author_id: Option<i32>,
    pub date_from: Option<chrono::NaiveDate>,
    pub date_to: Option<chrono::NaiveDate>,
    /// Inspection report only: `clear`, `issues_found` or `pending`.
    pub status: Option<String>,
}

impl ReportQuery {
    fn filter(&self) -> ReportFilter {
        ReportFilter {
            job_id: self.job_id.map(JobId),
            supervisor_id: self.supervisor_id.map(UserId),
            author_id: self.author_id.map(UserId),
            range: ReportRange {
                from: self.date_from,
                to: self.date_to,
            },
        }
    }
}

/// A job's progress line, with the classifications applied.
#[derive(Debug, Clone)]
pub struct JobProgress {
    pub row: JobProgressRow,
    pub completion_pct: i64,
    pub health: JobHealth,
}

/// A delay row with its severity.
#[derive(Debug, Clone)]
pub struct SeverityRow {
    pub row: DelayRow,
    pub severity: Severity,
}

/// What an inspection found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectionStatus {
    /// Issues were recorded.
    IssuesFound,
    /// Safety notes were recorded and no issues were.
    Clear,
    /// Neither: the entry exists but nothing was inspected.
    Pending,
}

impl InspectionStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IssuesFound => "issues_found",
            Self::Clear => "clear",
            Self::Pending => "pending",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "issues_found" => Some(Self::IssuesFound),
            "clear" => Some(Self::Clear),
            "pending" => Some(Self::Pending),
            _ => None,
        }
    }

    /// Issues outrank safety notes: an entry recording both has found
    /// something, and reporting it as clear would bury that.
    fn of(row: &InspectionRow) -> Self {
        let has = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.trim().is_empty());
        if has(&row.issues) {
            Self::IssuesFound
        } else if has(&row.safety_notes) {
            Self::Clear
        } else {
            Self::Pending
        }
    }
}

/// A diary entry graded as an inspection.
#[derive(Debug, Clone)]
pub struct Inspection {
    pub row: InspectionRow,
    pub status: InspectionStatus,
}

/// One day's weather, with whether it cost time.
#[derive(Debug, Clone)]
pub struct WeatherImpactRow {
    pub row: WeatherRow,
    pub is_impact_day: bool,
    pub impact_reason: Option<String>,
}

/// The weather report and its totals.
#[derive(Debug, Clone)]
pub struct WeatherImpact {
    pub rows: Vec<WeatherImpactRow>,
    pub total_impact_days: usize,
    pub total_rainfall_mm: f64,
    pub avg_temperature: Option<f64>,
    pub rainy_days: usize,
}

/// One job on one day.
#[derive(Debug, Clone)]
pub struct DailyProgressReport {
    pub date: chrono::NaiveDate,
    pub data: DailyProgressData,
    pub completed_tasks: i64,
    pub in_progress_tasks: i64,
    pub not_started_tasks: i64,
    pub delayed_tasks: i64,
    pub days_behind_program: i64,
    pub completion_pct: i64,
    pub health: JobHealth,
    /// Items whose planned window covers the report date.
    pub planned_today: Vec<i32>,
}

/// A supervisor's line on the performance report.
#[derive(Debug, Clone)]
pub struct ScoredSupervisor {
    pub supervisor_id: UserId,
    pub name: String,
    pub email: String,
    pub active_jobs: i64,
    pub completed_jobs: i64,
    pub total_jobs: i64,
    pub on_time_start_rate: Option<i64>,
    pub on_time_completion_rate: Option<i64>,
    pub delayed_task_count: i64,
    pub avg_delay_days: Option<i64>,
    pub diary_compliance_rate: Option<i64>,
    pub performance_score: i64,
}

pub struct ReportsService {
    reports: Arc<dyn ReportsRepository>,
    jobs: Arc<dyn JobRepository>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for ReportsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReportsService").finish_non_exhaustive()
    }
}

impl ReportsService {
    #[must_use]
    pub fn new(
        reports: Arc<dyn ReportsRepository>,
        jobs: Arc<dyn JobRepository>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            reports,
            jobs,
            clock,
        }
    }

    async fn scope(&self, s: &SessionUser) -> AppResult<JobScope> {
        if s.user.role.sees_all_company_jobs() {
            return Ok(JobScope::All);
        }
        Ok(JobScope::Only(
            self.jobs
                .visible_job_ids(s.principal.scope(), s.user.id)
                .await?,
        ))
    }

    /// Validates the query and resolves the caller's job scope.
    ///
    /// Naming a job outside that scope is a 403, matching the legacy: the
    /// caller asked about a specific job, so an empty report would be
    /// misleading rather than merely unhelpful.
    async fn prepare(
        &self,
        s: &SessionUser,
        q: &ReportQuery,
    ) -> AppResult<(JobScope, ReportFilter)> {
        let filter = q.filter();
        filter.range.validate().map_err(AppError::Domain)?;
        let scope = self.scope(s).await?;
        if let (Some(job), JobScope::Only(visible)) = (filter.job_id, &scope) {
            if !visible.contains(&job) {
                return Err(AppError::Domain(DomainError::Forbidden(
                    "that job is not yours to report on",
                )));
            }
        }
        Ok((scope, filter))
    }

    // ── stored reports ──────────────────────────────────────────────────────

    pub async fn stored(&self, s: &SessionUser, q: &ReportQuery) -> AppResult<Vec<StoredReport>> {
        let (scope, filter) = self.prepare(s, q).await?;
        Ok(self
            .reports
            .stored(s.principal.scope(), &scope, filter.job_id)
            .await?)
    }

    pub async fn create_stored(
        &self,
        s: &SessionUser,
        input: &StoredReportInput,
    ) -> AppResult<StoredReport> {
        // Checked against the job itself, not just the caller's scope:
        // `JobScope::All` means "every job in *my* company", so it says
        // nothing about a job id belonging to another tenant. Without this the
        // insert reaches RLS and fails as a 500 instead of a 404.
        self.jobs
            .find(
                s.principal.scope(),
                s.user.id,
                s.user.role.sees_all_company_jobs(),
                input.job_id,
            )
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Job")))?;

        if input.title.trim().is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "title",
                "a report needs a title",
            )));
        }
        if input.report_type.trim().is_empty() {
            return Err(AppError::Domain(DomainError::invalid(
                "type",
                "a report needs a type",
            )));
        }
        Ok(self
            .reports
            .create_stored(s.principal.scope(), input)
            .await?)
    }

    pub async fn meta(&self, s: &SessionUser) -> AppResult<ReportMeta> {
        let scope = self.scope(s).await?;
        Ok(self.reports.meta(s.principal.scope(), &scope).await?)
    }

    // ── generated reports ───────────────────────────────────────────────────

    pub async fn job_progress(
        &self,
        s: &SessionUser,
        q: &ReportQuery,
    ) -> AppResult<Vec<JobProgress>> {
        let (scope, filter) = self.prepare(s, q).await?;
        let rows = self
            .reports
            .job_progress(s.principal.scope(), &scope, filter, self.clock.today())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| JobProgress {
                completion_pct: completion_pct(row.completed, row.total_tasks),
                health: job_health(row.delayed_count),
                row,
            })
            .collect())
    }

    pub async fn delays(&self, s: &SessionUser, q: &ReportQuery) -> AppResult<Vec<SeverityRow>> {
        let (scope, filter) = self.prepare(s, q).await?;
        let rows = self
            .reports
            .delays(s.principal.scope(), &scope, filter, self.clock.today())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| SeverityRow {
                severity: Severity::of(row.delay_days),
                row,
            })
            .collect())
    }

    pub async fn diary_summary(
        &self,
        s: &SessionUser,
        q: &ReportQuery,
    ) -> AppResult<Vec<DiaryReportRow>> {
        let (scope, filter) = self.prepare(s, q).await?;
        Ok(self
            .reports
            .diary_summary(s.principal.scope(), &scope, filter)
            .await?)
    }

    pub async fn upcoming_tasks(
        &self,
        s: &SessionUser,
        q: &ReportQuery,
    ) -> AppResult<Vec<UpcomingTaskRow>> {
        let (scope, filter) = self.prepare(s, q).await?;
        Ok(self
            .reports
            .upcoming_tasks(s.principal.scope(), &scope, filter, self.clock.today())
            .await?)
    }

    pub async fn stage_claims(
        &self,
        s: &SessionUser,
        q: &ReportQuery,
    ) -> AppResult<Vec<StageClaimRow>> {
        let (scope, filter) = self.prepare(s, q).await?;
        Ok(self
            .reports
            .stage_claims(s.principal.scope(), &scope, filter)
            .await?)
    }

    pub async fn inspections(
        &self,
        s: &SessionUser,
        q: &ReportQuery,
    ) -> AppResult<Vec<Inspection>> {
        let wanted = match q.status.as_deref() {
            None | Some("") => None,
            Some(raw) => Some(InspectionStatus::parse(raw).ok_or_else(|| {
                AppError::Domain(DomainError::invalid(
                    "status",
                    "must be one of clear, issues_found, pending",
                ))
            })?),
        };
        let (scope, filter) = self.prepare(s, q).await?;
        let rows = self
            .reports
            .inspections(s.principal.scope(), &scope, filter)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| Inspection {
                status: InspectionStatus::of(&row),
                row,
            })
            .filter(|i| wanted.is_none_or(|w| w == i.status))
            .collect())
    }

    pub async fn weather(&self, s: &SessionUser, q: &ReportQuery) -> AppResult<WeatherImpact> {
        let (scope, filter) = self.prepare(s, q).await?;
        let rows = self
            .reports
            .weather(s.principal.scope(), &scope, filter)
            .await?;

        let mut total_rainfall = 0.0;
        let mut temps: Vec<f64> = Vec::new();
        let mut rainy_days = 0usize;
        let mut out = Vec::with_capacity(rows.len());

        for row in rows {
            total_rainfall += row.rainfall_mm.unwrap_or(0.0);
            if let Some(t) = row.temperature {
                temps.push(t);
            }
            if was_rainy(row.weather_condition.as_deref(), row.rainfall_mm) {
                rainy_days += 1;
            }
            let reason = weather_impact(
                row.weather_condition.as_deref(),
                row.rainfall_mm,
                row.issues.as_deref(),
            );
            out.push(WeatherImpactRow {
                is_impact_day: reason.is_some(),
                impact_reason: reason,
                row,
            });
        }

        Ok(WeatherImpact {
            total_impact_days: out.iter().filter(|r| r.is_impact_day).count(),
            total_rainfall_mm: round_1dp(total_rainfall),
            avg_temperature: mean_1dp(&temps),
            rainy_days,
            rows: out,
        })
    }

    pub async fn daily_progress(
        &self,
        s: &SessionUser,
        job: JobId,
        date: chrono::NaiveDate,
    ) -> AppResult<DailyProgressReport> {
        let scope = self.scope(s).await?;
        if let JobScope::Only(visible) = &scope {
            if !visible.contains(&job) {
                return Err(AppError::Domain(DomainError::not_found("Job")));
            }
        }
        let data = self
            .reports
            .daily_progress(s.principal.scope(), job, date)
            .await?
            .ok_or_else(|| AppError::Domain(DomainError::not_found("Job")))?;

        let today = self.clock.today();
        let items: Vec<_> = data
            .tasks
            .iter()
            .filter(|t| t.item_type != "HEADER")
            .collect();

        let count = |status: &str| items.iter().filter(|t| t.status == status).count() as i64;
        let open = |t: &&&pmk_ports::repository::DailyTaskRow| {
            t.status != "completed" && t.status != "on_hold"
        };

        let delayed: Vec<_> = items
            .iter()
            .filter(|t| open(t) && t.est_finish.is_some_and(|f| f < today))
            .collect();

        // How far behind the worst item is, not the sum: the programme is as
        // late as its latest piece, and adding delays would double-count work
        // that slipped together.
        let days_behind = delayed
            .iter()
            .filter_map(|t| t.est_finish.map(|f| (today - f).num_days()))
            .max()
            .unwrap_or(0);

        // An item is planned for the day if its window covers it. A missing
        // end is open-ended; a missing start means it was always due.
        let planned_today = data
            .tasks
            .iter()
            .filter(|t| t.item_type != "HEADER")
            .filter(|t| {
                (t.est_start.is_some() || t.est_finish.is_some())
                    && t.est_start.is_none_or(|s| s <= date)
                    && t.est_finish.is_none_or(|f| f >= date)
            })
            .map(|t| t.id)
            .collect();

        let completed = count("completed");
        let total = items.len() as i64;
        let delayed_count = delayed.len() as i64;

        Ok(DailyProgressReport {
            date,
            completed_tasks: completed,
            in_progress_tasks: count("in_progress"),
            not_started_tasks: count("not_started"),
            delayed_tasks: delayed_count,
            days_behind_program: days_behind,
            completion_pct: completion_pct(completed, total),
            health: job_health(delayed_count),
            planned_today,
            data,
        })
    }

    pub async fn supervisor_performance(
        &self,
        s: &SessionUser,
        q: &ReportQuery,
    ) -> AppResult<Vec<ScoredSupervisor>> {
        let filter = q.filter();
        filter.range.validate().map_err(AppError::Domain)?;
        let counts = self
            .reports
            .supervisor_counts(s.principal.scope(), filter, self.clock.today())
            .await?;

        Ok(counts
            .into_iter()
            .map(|c| {
                // `None` where there is no data, so the score falls back to
                // average rather than treating "nothing finished yet" as
                // "nothing finished on time".
                let on_time_start =
                    (c.started_tasks > 0).then(|| percentage(c.started_on_time, c.started_tasks));
                let on_time_completion = (c.finished_tasks > 0)
                    .then(|| percentage(c.finished_on_time, c.finished_tasks));
                // A mean in days, not a proportion, so `percentage` is the
                // wrong tool here.
                let avg_delay = mean_days(c.total_delay_days, c.delayed_tasks);

                let expected: i64 = c
                    .job_windows
                    .iter()
                    .map(|(from, to)| working_days(*from, *to))
                    .sum();
                let compliance = diary_compliance(c.diary_entries, expected);

                ScoredSupervisor {
                    performance_score: performance_score(
                        on_time_completion,
                        on_time_start,
                        compliance,
                        c.total_tasks,
                        c.delayed_tasks,
                    ),
                    supervisor_id: c.supervisor_id,
                    name: c.name,
                    email: c.email,
                    active_jobs: c.active_jobs,
                    completed_jobs: c.completed_jobs,
                    total_jobs: c.total_jobs,
                    on_time_start_rate: on_time_start,
                    on_time_completion_rate: on_time_completion,
                    delayed_task_count: c.delayed_tasks,
                    avg_delay_days: avg_delay,
                    diary_compliance_rate: compliance,
                }
            })
            .collect())
    }
}
