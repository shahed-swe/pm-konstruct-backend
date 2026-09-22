//! Reporting use cases.

mod service;
pub use service::{
    DailyProgressReport, Inspection, InspectionStatus, JobProgress, ReportQuery, ReportsService,
    ScoredSupervisor, SeverityRow, WeatherImpact, WeatherImpactRow,
};
