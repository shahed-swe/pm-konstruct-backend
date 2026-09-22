//! The `Job` aggregate.

use crate::ids::{JobId, UserId};
use crate::job::JobStatus;
use crate::tenant::CompanyId;
use crate::{DomainError, DomainResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: JobId,
    pub company_id: CompanyId,
    pub name: String,
    pub job_number: String,
    pub client: String,
    pub client_number: Option<String>,
    pub client_email: Option<String>,
    pub address: String,
    pub status: JobStatus,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub manager_id: Option<UserId>,
    pub supervisor_id: Option<UserId>,
    pub dropbox_path: Option<String>,
    pub description: Option<String>,
    pub contact2_name: Option<String>,
    pub contact2_phone: Option<String>,
    pub contact2_email: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Fields accepted when creating or updating a job.
#[derive(Debug, Clone, Default)]
pub struct JobInput {
    pub name: String,
    pub job_number: String,
    pub client: String,
    pub client_number: Option<String>,
    pub client_email: Option<String>,
    pub address: String,
    pub status: Option<JobStatus>,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub manager_id: Option<UserId>,
    pub supervisor_id: Option<UserId>,
    pub dropbox_path: Option<String>,
    pub description: Option<String>,
    pub contact2_name: Option<String>,
    pub contact2_phone: Option<String>,
    pub contact2_email: Option<String>,
}

impl JobInput {
    /// Validates before the database sees it, so the caller gets a field-level
    /// 400 rather than a constraint violation surfacing as a 409 or 500.
    ///
    /// `jobs_date_order_check` and `jobs_company_job_number_unique` still back
    /// this up at the database level -- validation here is for the message, not
    /// for correctness.
    pub fn validate(&self) -> DomainResult<()> {
        for (field, value) in [
            ("name", &self.name),
            ("jobNumber", &self.job_number),
            ("client", &self.client),
            ("address", &self.address),
        ] {
            if value.trim().is_empty() {
                return Err(DomainError::invalid(field, "is required"));
            }
        }
        if self.name.len() > 200 {
            return Err(DomainError::invalid(
                "name",
                "must be 200 characters or fewer",
            ));
        }
        if self.job_number.len() > 60 {
            return Err(DomainError::invalid(
                "jobNumber",
                "must be 60 characters or fewer",
            ));
        }
        for (field, value) in [
            ("clientEmail", &self.client_email),
            ("contact2Email", &self.contact2_email),
        ] {
            if let Some(e) = value {
                if !e.trim().is_empty() && !looks_like_email(e) {
                    return Err(DomainError::invalid(field, "must be an email address"));
                }
            }
        }
        // Mirrors jobs_date_order_check.
        if let (Some(s), Some(e)) = (self.start_date, self.end_date) {
            if s > e {
                return Err(DomainError::invalid(
                    "endDate",
                    "must be on or after the start date",
                ));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn normalised(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.job_number = self.job_number.trim().to_string();
        self.client = self.client.trim().to_string();
        self.address = self.address.trim().to_string();
        self
    }
}

/// Deliberately minimal: a full RFC 5322 parser is not the point, and the
/// legacy system accepted anything with an @ in it.
fn looks_like_email(s: &str) -> bool {
    let s = s.trim();
    // Whitespace anywhere is invalid, including in the local part -- an earlier
    // version only checked the domain and accepted "a b@c.com".
    if s.chars().any(char::is_whitespace) {
        return false;
    }
    match s.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !domain.contains('@')
        }
        None => false,
    }
}

/// One row of `job_assignments`, joined to the user for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobAssignment {
    /// The assigned **user**. The legacy DTO called this `id` while holding a
    /// user id, which is how `syncPrimarySupervisor` came to look correct while
    /// reading a confusing field (domain-rules R4).
    pub user_id: UserId,
    pub name: String,
    pub role: crate::access::Role,
    pub is_primary: bool,
    pub assigned_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> JobInput {
        JobInput {
            name: "Riverside".into(),
            job_number: "BSC-1".into(),
            client: "Client".into(),
            address: "1 St".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_complete_input_validates() {
        assert!(base().validate().is_ok());
    }

    #[test]
    fn required_fields_are_named_individually() {
        for (field, mutate) in [
            (
                "name",
                (|i: &mut JobInput| i.name = "  ".into()) as fn(&mut JobInput),
            ),
            ("jobNumber", |i: &mut JobInput| i.job_number = "".into()),
            ("client", |i: &mut JobInput| i.client = "".into()),
            ("address", |i: &mut JobInput| i.address = "".into()),
        ] {
            let mut input = base();
            mutate(&mut input);
            match input.validate() {
                Err(DomainError::Invalid { field: f, .. }) => assert_eq!(f, field),
                other => panic!("expected {field} to be rejected, got {other:?}"),
            }
        }
    }

    #[test]
    fn end_before_start_is_rejected_with_the_end_date_named() {
        let mut i = base();
        i.start_date = Some("2026-05-10".parse().unwrap());
        i.end_date = Some("2026-05-01".parse().unwrap());
        match i.validate() {
            Err(DomainError::Invalid { field, .. }) => assert_eq!(field, "endDate"),
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn equal_start_and_end_dates_are_allowed() {
        let mut i = base();
        let d = "2026-05-10".parse().unwrap();
        i.start_date = Some(d);
        i.end_date = Some(d);
        assert!(i.validate().is_ok(), "a one-day job is valid");
    }

    #[test]
    fn malformed_emails_are_rejected_but_empty_ones_are_not() {
        let mut i = base();
        for bad in ["nope", "a@b", "@b.com", "a@.com", "a b@c.com"] {
            i.client_email = Some(bad.into());
            assert!(i.validate().is_err(), "{bad} should be rejected");
        }
        i.client_email = Some("  ".into());
        assert!(i.validate().is_ok(), "blank is treated as absent");
        i.client_email = Some("a@b.com".into());
        assert!(i.validate().is_ok());
    }

    #[test]
    fn normalisation_trims_but_does_not_alter_case() {
        let mut i = base();
        i.name = "  Riverside  ".into();
        i.job_number = " BSC-1 ".into();
        let n = i.normalised();
        assert_eq!(n.name, "Riverside");
        assert_eq!(n.job_number, "BSC-1");
    }
}
