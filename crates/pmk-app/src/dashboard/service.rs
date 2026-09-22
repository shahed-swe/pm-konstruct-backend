use std::sync::Arc;

use pmk_domain::dashboard::{delay_severity, ActionItem, DashboardStats, DelaySeverity, JobScope};
use pmk_ports::repository::{
    DashboardCallForward, DashboardDiaryEntry, DashboardJob, DashboardRepository, JobListFilter,
    JobRepository, UpcomingClaim,
};
use pmk_ports::Clock;

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

/// How far back "recent" reaches, for the diary counter and its drill-down.
const RECENT_DIARY_DAYS: i64 = 7;

/// How far ahead upcoming claims are shown: to the end of next month, plus a
/// week. Far enough to plan around, not so far that the list is noise.
const CLAIM_HORIZON_EXTRA_DAYS: i64 = 7;

pub struct DashboardService {
    dashboard: Arc<dyn DashboardRepository>,
    jobs: Arc<dyn JobRepository>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for DashboardService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashboardService").finish_non_exhaustive()
    }
}

impl DashboardService {
    #[must_use]
    pub fn new(
        dashboard: Arc<dyn DashboardRepository>,
        jobs: Arc<dyn JobRepository>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            dashboard,
            jobs,
            clock,
        }
    }

    /// Which jobs this caller's dashboard covers.
    ///
    /// Managers and office users see the whole company; a supervisor sees the
    /// jobs R3 makes visible -- their assignments union the jobs they are
    /// primary supervisor of.
    ///
    /// The legacy `getActionableNotes` filtered supervisors by assignments
    /// *alone*, so a supervisor who was primary on a job but not separately
    /// assigned to it saw the job everywhere except in their action items.
    /// That inconsistency is not reproduced: every dashboard query uses this.
    async fn scope(&self, s: &SessionUser) -> AppResult<JobScope> {
        if s.user.role.sees_all_company_jobs() {
            return Ok(JobScope::All);
        }
        let ids = self
            .jobs
            .visible_job_ids(s.principal.scope(), s.user.id)
            .await?;
        Ok(JobScope::Only(ids))
    }

    fn recent_cutoff(&self) -> chrono::NaiveDate {
        self.clock.today() - chrono::Duration::days(RECENT_DIARY_DAYS)
    }

    pub async fn stats(&self, s: &SessionUser) -> AppResult<DashboardStats> {
        let scope = self.scope(s).await?;
        // Headcount is a manager-only figure -- every other role gets null
        // rather than a number they should not have. `Role::Manager` is the
        // only role that bypasses permission checks, so this is that test.
        let include_users = s.user.role.bypasses_permission_checks();
        Ok(self
            .dashboard
            .stats(
                s.principal.scope(),
                &scope,
                self.recent_cutoff(),
                include_users,
            )
            .await?)
    }

    pub async fn action_items(&self, s: &SessionUser) -> AppResult<Vec<ActionItem>> {
        let scope = self.scope(s).await?;
        Ok(self
            .dashboard
            .action_items(s.principal.scope(), &scope)
            .await?)
    }

    pub async fn jobs_list(
        &self,
        s: &SessionUser,
        filter: JobListFilter,
    ) -> AppResult<Vec<DashboardJob>> {
        let scope = self.scope(s).await?;
        Ok(self
            .dashboard
            .jobs_list(s.principal.scope(), &scope, filter)
            .await?)
    }

    pub async fn open_call_forwards(
        &self,
        s: &SessionUser,
    ) -> AppResult<Vec<DashboardCallForward>> {
        let scope = self.scope(s).await?;
        Ok(self
            .dashboard
            .open_call_forwards(s.principal.scope(), &scope)
            .await?)
    }

    pub async fn overdue_call_forwards(
        &self,
        s: &SessionUser,
    ) -> AppResult<Vec<DashboardCallForward>> {
        let scope = self.scope(s).await?;
        Ok(self
            .dashboard
            .overdue_call_forwards(s.principal.scope(), &scope, self.clock.today())
            .await?)
    }

    pub async fn delay_severity(&self, s: &SessionUser) -> AppResult<Vec<DelaySeverity>> {
        let scope = self.scope(s).await?;
        let days = self
            .dashboard
            .overdue_days(s.principal.scope(), &scope, self.clock.today())
            .await?;
        Ok(delay_severity(&days))
    }

    pub async fn recent_diary(&self, s: &SessionUser) -> AppResult<Vec<DashboardDiaryEntry>> {
        let scope = self.scope(s).await?;
        Ok(self
            .dashboard
            .recent_diary(s.principal.scope(), &scope, self.recent_cutoff())
            .await?)
    }

    pub async fn upcoming_claims(&self, s: &SessionUser) -> AppResult<Vec<UpcomingClaim>> {
        let scope = self.scope(s).await?;
        let cutoff = claim_horizon(self.clock.today())?;
        Ok(self
            .dashboard
            .upcoming_claims(s.principal.scope(), &scope, cutoff)
            .await?)
    }
}

/// End of next month, plus a week.
///
/// Computed from the company's today rather than the server's, so a claim due
/// on the 1st does not appear or vanish depending on which side of midnight
/// UTC the request lands.
fn claim_horizon(today: chrono::NaiveDate) -> AppResult<chrono::NaiveDate> {
    use chrono::Datelike;
    // The first of the month after next, minus a day, is the last day of next
    // month -- which sidesteps having to know how long next month is.
    let (year, month) = match today.month() {
        11 => (today.year() + 1, 1),
        12 => (today.year() + 1, 2),
        m => (today.year(), m + 2),
    };
    let first_of_month_after_next = chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| AppError::Internal("could not compute the claim horizon".into()))?;
    Ok(first_of_month_after_next + chrono::Duration::days(CLAIM_HORIZON_EXTRA_DAYS - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn the_claim_horizon_reaches_a_week_past_the_end_of_next_month() {
        // From March, next month is April (30 days), so the horizon is 7 May.
        assert_eq!(claim_horizon(d(2026, 3, 15)).unwrap(), d(2026, 5, 7));
        // From January, next month is February -- 28 days in 2026.
        assert_eq!(claim_horizon(d(2026, 1, 1)).unwrap(), d(2026, 3, 7));
    }

    #[test]
    fn it_rolls_over_the_year_from_november_and_december() {
        assert_eq!(claim_horizon(d(2026, 11, 30)).unwrap(), d(2027, 1, 7));
        assert_eq!(claim_horizon(d(2026, 12, 1)).unwrap(), d(2027, 2, 7));
    }

    #[test]
    fn a_leap_february_does_not_shorten_the_horizon() {
        // 2028 is a leap year: next month from January has 29 days, and the
        // horizon still lands 7 days past its end.
        assert_eq!(claim_horizon(d(2028, 1, 10)).unwrap(), d(2028, 3, 7));
    }

    #[test]
    fn the_horizon_does_not_depend_on_the_day_of_the_month() {
        assert_eq!(
            claim_horizon(d(2026, 6, 1)).unwrap(),
            claim_horizon(d(2026, 6, 30)).unwrap()
        );
    }
}
