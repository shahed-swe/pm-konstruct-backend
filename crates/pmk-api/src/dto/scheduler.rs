//! Trade scheduler DTOs.

use pmk_domain::ids::{JobId, SchedulerAbsenceId, SchedulerAllocationId, SchedulerWorkerId};
use pmk_domain::scheduler::{
    Absence, AbsenceInput, AbsenceType, Allocation, AllocationInput, AllocationTarget, Worker,
    WorkerInput,
};
use pmk_domain::DomainError;
use pmk_ports::repository::{BoardData, DateRange, DayNote, MaintenanceJob, MaintenanceJobInput};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerDto {
    pub id: i32,
    pub company_id: i32,
    pub name: String,
    pub trade: Option<String>,
    pub color: String,
    pub active: bool,
    pub on_leave: bool,
    pub leave_from: Option<chrono::NaiveDate>,
    pub leave_to: Option<chrono::NaiveDate>,
}

impl From<Worker> for WorkerDto {
    fn from(w: Worker) -> Self {
        Self {
            id: w.id.get(),
            company_id: w.company_id.get(),
            name: w.name,
            trade: w.trade,
            color: w.color,
            active: w.active,
            on_leave: w.on_leave,
            leave_from: w.leave_from,
            leave_to: w.leave_to,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerRequest {
    pub name: String,
    pub trade: Option<String>,
    pub color: Option<String>,
    pub active: Option<bool>,
}

impl From<WorkerRequest> for WorkerInput {
    fn from(r: WorkerRequest) -> Self {
        Self {
            name: r.name,
            trade: r.trade,
            color: r.color,
            active: r.active,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbsenceDto {
    pub id: i32,
    pub worker_id: i32,
    pub absence_type: String,
    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate,
}

impl From<Absence> for AbsenceDto {
    fn from(a: Absence) -> Self {
        Self {
            id: a.id.get(),
            worker_id: a.worker_id.get(),
            absence_type: a.absence_type.as_str().to_string(),
            start_date: a.start_date,
            end_date: a.end_date,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbsenceRequest {
    pub worker_id: i32,
    /// Accepts `type` too, which is what the legacy client sent.
    #[serde(alias = "type")]
    pub absence_type: String,
    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate,
}

impl AbsenceRequest {
    pub fn into_input(self) -> Result<AbsenceInput, DomainError> {
        let absence_type = AbsenceType::parse(&self.absence_type).ok_or_else(|| {
            DomainError::invalid(
                "absenceType",
                format!("must be one of {}", AbsenceType::ALL),
            )
        })?;
        Ok(AbsenceInput {
            worker_id: self.worker_id,
            absence_type,
            start_date: self.start_date,
            end_date: self.end_date,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AllocationDto {
    pub id: i32,
    pub worker_id: i32,
    pub job_id: Option<i32>,
    pub maintenance_job_id: Option<i32>,
    pub assigned_date: chrono::NaiveDate,
    pub note: Option<String>,
}

impl From<Allocation> for AllocationDto {
    fn from(a: Allocation) -> Self {
        let (job, maintenance) = match a.target {
            AllocationTarget::Job(j) => (Some(j.get()), None),
            AllocationTarget::MaintenanceJob(m) => (None, Some(m)),
        };
        Self {
            id: a.id.get(),
            worker_id: a.worker_id.get(),
            job_id: job,
            maintenance_job_id: maintenance,
            assigned_date: a.assigned_date,
            note: a.note,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllocationRequest {
    pub worker_id: i32,
    pub job_id: Option<i32>,
    pub maintenance_job_id: Option<i32>,
    pub assigned_date: chrono::NaiveDate,
    pub note: Option<String>,
}

impl From<AllocationRequest> for AllocationInput {
    fn from(r: AllocationRequest) -> Self {
        Self {
            worker_id: r.worker_id,
            job_id: r.job_id,
            maintenance_job_id: r.maintenance_job_id,
            assigned_date: r.assigned_date,
            note: r.note,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceJobDto {
    pub id: i32,
    pub name: String,
    pub reference: Option<String>,
    pub address: Option<String>,
    pub status: String,
}

impl From<MaintenanceJob> for MaintenanceJobDto {
    fn from(m: MaintenanceJob) -> Self {
        Self {
            id: m.id,
            name: m.name,
            reference: m.reference,
            address: m.address,
            status: m.status,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceJobRequest {
    pub name: String,
    pub reference: Option<String>,
    pub address: Option<String>,
    pub status: Option<String>,
}

impl From<MaintenanceJobRequest> for MaintenanceJobInput {
    fn from(r: MaintenanceJobRequest) -> Self {
        Self {
            name: r.name,
            reference: r.reference,
            address: r.address,
            status: r.status,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayNoteDto {
    pub id: i32,
    pub job_id: i32,
    pub note_date: chrono::NaiveDate,
    pub note: String,
}

impl From<DayNote> for DayNoteDto {
    fn from(n: DayNote) -> Self {
        Self {
            id: n.id,
            job_id: n.job_id.get(),
            note_date: n.note_date,
            note: n.note,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayNoteRequest {
    pub job_id: i32,
    pub note_date: chrono::NaiveDate,
    pub note: String,
}

/// The whole board window in one response.
///
/// The legacy UI assembled this from several calls per worker per day; one
/// consistent snapshot is both faster and free of the tearing that produced.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardDto {
    pub from: chrono::NaiveDate,
    pub to: chrono::NaiveDate,
    pub workers: Vec<WorkerDto>,
    pub absences: Vec<AbsenceDto>,
    pub allocations: Vec<AllocationDto>,
    pub day_notes: Vec<DayNoteDto>,
}

impl BoardDto {
    #[must_use]
    pub fn new(range: DateRange, data: BoardData) -> Self {
        Self {
            from: range.from,
            to: range.to,
            workers: data.workers.into_iter().map(Into::into).collect(),
            absences: data.absences.into_iter().map(Into::into).collect(),
            allocations: data.allocations.into_iter().map(Into::into).collect(),
            day_notes: data.day_notes.into_iter().map(Into::into).collect(),
        }
    }
}

/// A date window. Both ends are required: an unbounded scheduler query would
/// scan every allocation the company has ever made.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeQuery {
    pub from: chrono::NaiveDate,
    pub to: chrono::NaiveDate,
}

impl From<RangeQuery> for DateRange {
    fn from(q: RangeQuery) -> Self {
        Self {
            from: q.from,
            to: q.to,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkerListQuery {
    #[serde(default)]
    pub include_inactive: bool,
}

#[must_use]
pub fn worker_id(v: i32) -> SchedulerWorkerId {
    SchedulerWorkerId(v)
}
#[must_use]
pub fn absence_id(v: i32) -> SchedulerAbsenceId {
    SchedulerAbsenceId(v)
}
#[must_use]
pub fn allocation_id(v: i32) -> SchedulerAllocationId {
    SchedulerAllocationId(v)
}
#[must_use]
pub fn job_id(v: i32) -> JobId {
    JobId(v)
}
