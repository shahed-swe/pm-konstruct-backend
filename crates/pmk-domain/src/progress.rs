//! Progress records: a percentage against a job on a date.

use crate::error::{DomainError, DomainResult};
use crate::ids::{JobId, ProgressId};

#[derive(Debug, Clone)]
pub struct Progress {
    pub id: ProgressId,
    pub job_id: JobId,
    pub date: chrono::NaiveDate,
    pub percent_complete: f32,
    pub milestone: Option<String>,
    pub description: Option<String>,
    /// Free-form URLs. Stored as `text[]`, not as media rows: the legacy UI
    /// pastes links here rather than uploading.
    pub photos: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct ProgressInput {
    pub job_id: JobId,
    pub date: chrono::NaiveDate,
    pub percent_complete: f32,
    pub milestone: Option<String>,
    pub description: Option<String>,
    pub photos: Vec<String>,
}

/// A progress record's photo list is capped so one row cannot be turned into
/// an unbounded blob.
pub const MAX_PROGRESS_PHOTOS: usize = 50;

impl ProgressInput {
    pub fn validate(&self) -> DomainResult<()> {
        if self.job_id.get() <= 0 {
            return Err(DomainError::invalid("jobId", "a valid job is required"));
        }
        // NaN would pass `>= 0.0 && <= 100.0` as false and reach the CHECK
        // constraint as a 500, so it is rejected by name here.
        if self.percent_complete.is_nan() {
            return Err(DomainError::invalid(
                "percentComplete",
                "must be a number between 0 and 100",
            ));
        }
        if !(0.0..=100.0).contains(&self.percent_complete) {
            return Err(DomainError::invalid(
                "percentComplete",
                "must be between 0 and 100",
            ));
        }
        if self.photos.len() > MAX_PROGRESS_PHOTOS {
            return Err(DomainError::invalid(
                "photos",
                format!("cannot hold more than {MAX_PROGRESS_PHOTOS} links"),
            ));
        }
        for url in &self.photos {
            // Same rule as job links: an https URL, so a stored value cannot
            // become a javascript: payload when the UI renders it as an
            // anchor.
            if !url.trim().starts_with("https://") {
                return Err(DomainError::invalid(
                    "photos",
                    "every link must start with https://",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(percent: f32) -> ProgressInput {
        ProgressInput {
            job_id: JobId(1),
            date: chrono::NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
            percent_complete: percent,
            milestone: None,
            description: None,
            photos: Vec::new(),
        }
    }

    #[test]
    fn the_whole_percentage_range_is_accepted() {
        assert!(input(0.0).validate().is_ok());
        assert!(input(50.5).validate().is_ok());
        assert!(input(100.0).validate().is_ok());
    }

    #[test]
    fn out_of_range_percentages_are_a_400_not_a_check_violation() {
        assert!(input(-0.1).validate().is_err());
        assert!(input(100.1).validate().is_err());
    }

    #[test]
    fn nan_is_rejected_by_name() {
        // `!(0.0..=100.0).contains(&NAN)` is true, so without this it would
        // reach the CHECK constraint and surface as a 500.
        let e = input(f32::NAN).validate().unwrap_err();
        assert!(matches!(e, DomainError::Invalid { field, .. } if field == "percentComplete"));
    }

    #[test]
    fn infinity_is_out_of_range() {
        assert!(input(f32::INFINITY).validate().is_err());
        assert!(input(f32::NEG_INFINITY).validate().is_err());
    }

    #[test]
    fn photo_links_must_be_https() {
        let mut i = input(10.0);
        i.photos = vec!["http://example.com/a.jpg".into()];
        assert!(i.validate().is_err());
        i.photos = vec!["javascript:alert(1)".into()];
        assert!(i.validate().is_err());
        i.photos = vec!["https://example.com/a.jpg".into()];
        assert!(i.validate().is_ok());
    }

    #[test]
    fn the_photo_list_is_capped() {
        let mut i = input(10.0);
        i.photos = vec!["https://e.com/a.jpg".to_string(); MAX_PROGRESS_PHOTOS];
        assert!(i.validate().is_ok());
        i.photos.push("https://e.com/b.jpg".into());
        assert!(i.validate().is_err());
    }
}
