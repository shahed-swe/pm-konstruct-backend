//! Report response shapes.
//!
//! Field names match the legacy report payloads exactly: the frontend renders
//! these tables directly and the client exports them to PDF, so a renamed key
//! is a broken column rather than a cosmetic change.

use pmk_app::reports::{
    DailyProgressReport, Inspection, JobProgress, ReportQuery, ScoredSupervisor, SeverityRow,
    WeatherImpact,
};
use pmk_ports::repository::{
    DiaryReportRow, ReportMeta, StageClaimRow, StoredReport, StoredReportInput, UpcomingTaskRow,
};
use serde::{Deserialize, Serialize};

/// The query string every report shares.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReportQueryParams {
    pub job_id: Option<i32>,
    pub supervisor_id: Option<i32>,
    pub author_id: Option<i32>,
    pub date_from: Option<chrono::NaiveDate>,
    pub date_to: Option<chrono::NaiveDate>,
    pub status: Option<String>,
}

impl From<ReportQueryParams> for ReportQuery {
    fn from(q: ReportQueryParams) -> Self {
        Self {
            job_id: q.job_id,
            supervisor_id: q.supervisor_id,
            author_id: q.author_id,
            date_from: q.date_from,
            date_to: q.date_to,
            status: q.status,
        }
    }
}

/// The daily-progress report names its job and date explicitly.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyProgressQuery {
    pub job_id: i32,
    pub date: chrono::NaiveDate,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredReportDto {
    pub id: i32,
    pub job_id: i32,
    #[serde(rename = "type")]
    pub report_type: String,
    pub title: String,
    pub content: String,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<StoredReport> for StoredReportDto {
    fn from(r: StoredReport) -> Self {
        Self {
            id: r.id.get(),
            job_id: r.job_id.get(),
            report_type: r.report_type,
            title: r.title,
            content: r.content,
            generated_at: r.generated_at,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateReportRequest {
    pub job_id: i32,
    #[serde(rename = "type")]
    pub report_type: String,
    pub title: String,
    #[serde(default)]
    pub content: String,
}

impl From<CreateReportRequest> for StoredReportInput {
    fn from(r: CreateReportRequest) -> Self {
        Self {
            job_id: pmk_domain::ids::JobId(r.job_id),
            report_type: r.report_type,
            title: r.title,
            content: r.content,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportMetaJobDto {
    pub id: i32,
    pub name: String,
    pub job_number: String,
    pub status: String,
    pub address: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportMetaDto {
    pub jobs: Vec<ReportMetaJobDto>,
    pub supervisors: Vec<crate::dto::CalendarFilterSupervisorDto>,
}

impl From<ReportMeta> for ReportMetaDto {
    fn from(m: ReportMeta) -> Self {
        Self {
            jobs: m
                .jobs
                .into_iter()
                .map(|j| ReportMetaJobDto {
                    id: j.id.get(),
                    name: j.name,
                    job_number: j.job_number,
                    status: j.status,
                    address: j.address,
                })
                .collect(),
            supervisors: m
                .supervisors
                .into_iter()
                .map(|s| crate::dto::CalendarFilterSupervisorDto {
                    id: s.id.get(),
                    name: s.name,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgressDto {
    pub job_id: i32,
    pub job_number: String,
    pub job_name: String,
    pub client: Option<String>,
    pub address: Option<String>,
    pub status: String,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub supervisor_id: Option<i32>,
    pub supervisor_name: Option<String>,
    pub total_tasks: i64,
    pub not_started: i64,
    pub in_progress: i64,
    pub completed: i64,
    pub on_hold: i64,
    pub delayed_count: i64,
    pub completion_pct: i64,
    pub health: &'static str,
}

impl From<JobProgress> for JobProgressDto {
    fn from(p: JobProgress) -> Self {
        let r = p.row;
        Self {
            job_id: r.job_id.get(),
            job_number: r.job_number,
            job_name: r.job_name,
            client: r.client,
            address: r.address,
            status: r.status,
            start_date: r.start_date,
            end_date: r.end_date,
            supervisor_id: r.supervisor_id.map(|u| u.get()),
            supervisor_name: r.supervisor_name,
            total_tasks: r.total_tasks,
            not_started: r.not_started,
            in_progress: r.in_progress,
            completed: r.completed,
            on_hold: r.on_hold,
            delayed_count: r.delayed_count,
            completion_pct: p.completion_pct,
            health: p.health.as_str(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelayDto {
    pub task_id: i32,
    pub title: String,
    pub item_type: String,
    pub supplier_trade: Option<String>,
    pub job_id: i32,
    pub job_number: String,
    pub job_name: String,
    pub job_address: Option<String>,
    pub supervisor_id: Option<i32>,
    pub supervisor_name: Option<String>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub delay_days: i64,
    pub status: String,
    pub severity: &'static str,
}

impl From<SeverityRow> for DelayDto {
    fn from(d: SeverityRow) -> Self {
        let r = d.row;
        Self {
            task_id: r.task_id,
            title: r.title,
            item_type: r.item_type,
            supplier_trade: r.supplier_trade,
            job_id: r.job_id.get(),
            job_number: r.job_number,
            job_name: r.job_name,
            job_address: r.job_address,
            supervisor_id: r.supervisor_id.map(|u| u.get()),
            supervisor_name: r.supervisor_name,
            est_finish: r.est_finish,
            actual_finish: r.actual_finish,
            delay_days: r.delay_days,
            status: r.status,
            severity: d.severity.as_str(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryReportDto {
    pub id: i32,
    pub date: chrono::NaiveDate,
    pub job_id: i32,
    pub job_number: String,
    pub job_name: String,
    pub job_address: Option<String>,
    pub author_id: Option<i32>,
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

impl From<DiaryReportRow> for DiaryReportDto {
    fn from(r: DiaryReportRow) -> Self {
        Self {
            id: r.id.get(),
            date: r.date,
            job_id: r.job_id.get(),
            job_number: r.job_number,
            job_name: r.job_name,
            job_address: r.job_address,
            author_id: r.author_id.map(|u| u.get()),
            author_name: r.author_name,
            weather: r.weather,
            workforce: r.workforce,
            work_completed: r.work_completed,
            materials: r.materials,
            trades_on_site: r.trades_on_site,
            safety_notes: r.safety_notes,
            client_instructions: r.client_instructions,
            issues: r.issues,
            notes: r.notes,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingTaskDto {
    pub task_id: i32,
    pub title: String,
    pub item_type: String,
    pub supplier_trade: Option<String>,
    pub job_id: i32,
    pub job_number: String,
    pub job_name: String,
    pub job_address: Option<String>,
    pub supervisor_id: Option<i32>,
    pub supervisor_name: Option<String>,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub days_until_start: Option<i64>,
    pub days_until_finish: Option<i64>,
    pub status: String,
}

impl From<UpcomingTaskRow> for UpcomingTaskDto {
    fn from(r: UpcomingTaskRow) -> Self {
        Self {
            task_id: r.task_id,
            title: r.title,
            item_type: r.item_type,
            supplier_trade: r.supplier_trade,
            job_id: r.job_id.get(),
            job_number: r.job_number,
            job_name: r.job_name,
            job_address: r.job_address,
            supervisor_id: r.supervisor_id.map(|u| u.get()),
            supervisor_name: r.supervisor_name,
            est_start: r.est_start,
            est_finish: r.est_finish,
            days_until_start: r.days_until_start,
            days_until_finish: r.days_until_finish,
            status: r.status,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageClaimDto {
    pub id: i32,
    pub title: String,
    pub job_id: i32,
    pub job_number: String,
    pub job_name: String,
    pub job_address: Option<String>,
    pub supervisor_name: Option<String>,
    pub supplier_trade: Option<String>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub status: String,
    /// `YYYY-MM` of the estimated finish, which is how the forecast groups.
    pub forecast_month: Option<String>,
}

impl From<StageClaimRow> for StageClaimDto {
    fn from(r: StageClaimRow) -> Self {
        Self {
            forecast_month: r.est_finish.map(|d| d.format("%Y-%m").to_string()),
            id: r.id,
            title: r.title,
            job_id: r.job_id.get(),
            job_number: r.job_number,
            job_name: r.job_name,
            job_address: r.job_address,
            supervisor_name: r.supervisor_name,
            supplier_trade: r.supplier_trade,
            est_finish: r.est_finish,
            actual_finish: r.actual_finish,
            status: r.status,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionDto {
    pub id: i32,
    pub date: chrono::NaiveDate,
    pub job_id: i32,
    pub job_number: String,
    pub job_name: String,
    pub job_address: Option<String>,
    pub inspector: Option<String>,
    pub safety_notes: Option<String>,
    pub issues: Option<String>,
    pub status: &'static str,
    pub workforce: Option<i32>,
    pub trades_on_site: Option<String>,
}

impl From<Inspection> for InspectionDto {
    fn from(i: Inspection) -> Self {
        let r = i.row;
        Self {
            id: r.id.get(),
            date: r.date,
            job_id: r.job_id.get(),
            job_number: r.job_number,
            job_name: r.job_name,
            job_address: r.job_address,
            inspector: r.inspector,
            safety_notes: r.safety_notes,
            issues: r.issues,
            status: i.status.as_str(),
            workforce: r.workforce,
            trades_on_site: r.trades_on_site,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherRowDto {
    pub id: i32,
    pub date: chrono::NaiveDate,
    pub job_id: i32,
    pub job_number: String,
    pub job_name: String,
    pub author_name: Option<String>,
    pub location_name: Option<String>,
    pub weather_condition: Option<String>,
    pub temperature: Option<f64>,
    pub rainfall_mm: Option<f64>,
    pub wind_speed_kmh: Option<f64>,
    pub is_weather_impact_day: bool,
    pub impact_reason: Option<String>,
    pub issues: Option<String>,
    pub workforce: Option<i32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherImpactDto {
    pub rows: Vec<WeatherRowDto>,
    pub total_impact_days: usize,
    pub total_rainfall_mm: f64,
    pub avg_temperature: Option<f64>,
    pub rainy_days: usize,
}

impl From<WeatherImpact> for WeatherImpactDto {
    fn from(w: WeatherImpact) -> Self {
        Self {
            total_impact_days: w.total_impact_days,
            total_rainfall_mm: w.total_rainfall_mm,
            avg_temperature: w.avg_temperature,
            rainy_days: w.rainy_days,
            rows: w
                .rows
                .into_iter()
                .map(|x| {
                    let r = x.row;
                    WeatherRowDto {
                        id: r.id.get(),
                        date: r.date,
                        job_id: r.job_id.get(),
                        job_number: r.job_number,
                        job_name: r.job_name,
                        author_name: r.author_name,
                        location_name: r.location_name,
                        weather_condition: r.weather_condition,
                        temperature: r.temperature,
                        rainfall_mm: r.rainfall_mm,
                        wind_speed_kmh: r.wind_speed_kmh,
                        is_weather_impact_day: x.is_impact_day,
                        impact_reason: x.impact_reason,
                        issues: r.issues,
                        workforce: r.workforce,
                    }
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyProgressDto {
    pub date: chrono::NaiveDate,
    pub job_id: i32,
    pub job_number: String,
    pub job_name: String,
    pub client: Option<String>,
    pub supervisor_id: Option<i32>,
    pub supervisor_name: Option<String>,
    pub total_tasks: usize,
    pub completed_tasks: i64,
    pub in_progress_tasks: i64,
    pub not_started_tasks: i64,
    pub delayed_tasks: i64,
    pub days_behind_program: i64,
    pub completion_pct: i64,
    pub health: &'static str,
    /// Ids of the programme items whose window covers the report date.
    pub planned_today: Vec<i32>,
    pub diary: Option<DiaryReportDto>,
}

impl From<DailyProgressReport> for DailyProgressDto {
    fn from(d: DailyProgressReport) -> Self {
        let data = d.data;
        Self {
            date: d.date,
            job_id: data.job_id.get(),
            job_number: data.job_number,
            job_name: data.job_name,
            client: data.client,
            supervisor_id: data.supervisor_id.map(|u| u.get()),
            supervisor_name: data.supervisor_name,
            total_tasks: data
                .tasks
                .iter()
                .filter(|t| t.item_type != "HEADER")
                .count(),
            completed_tasks: d.completed_tasks,
            in_progress_tasks: d.in_progress_tasks,
            not_started_tasks: d.not_started_tasks,
            delayed_tasks: d.delayed_tasks,
            days_behind_program: d.days_behind_program,
            completion_pct: d.completion_pct,
            health: d.health.as_str(),
            planned_today: d.planned_today,
            diary: data.diary.map(Into::into),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorPerformanceDto {
    pub supervisor_id: i32,
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

impl From<ScoredSupervisor> for SupervisorPerformanceDto {
    fn from(s: ScoredSupervisor) -> Self {
        Self {
            supervisor_id: s.supervisor_id.get(),
            name: s.name,
            email: s.email,
            active_jobs: s.active_jobs,
            completed_jobs: s.completed_jobs,
            total_jobs: s.total_jobs,
            on_time_start_rate: s.on_time_start_rate,
            on_time_completion_rate: s.on_time_completion_rate,
            delayed_task_count: s.delayed_task_count,
            avg_delay_days: s.avg_delay_days,
            diary_compliance_rate: s.diary_compliance_rate,
            performance_score: s.performance_score,
        }
    }
}
