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

/// A cloud-storage link attached to a job.
///
/// Despite the `job_dropbox_folders` table name, this was never a Dropbox
/// integration: all 44 production rows are plain shared URLs, and the client
/// confirmed "it's just a link to drop box... any cloud based server. Google,
/// Dropbox etc." See docs/audit/integrations-reality.md.
///
/// The column names are kept so the data migration needs no transform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLink {
    pub id: i32,
    pub job_id: JobId,
    pub label: String,
    pub url: String,
    pub sort_order: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct JobLinkInput {
    pub label: String,
    pub url: String,
    pub sort_order: Option<i32>,
}

impl JobLinkInput {
    /// Accepts any `https://` URL, not just Dropbox.
    ///
    /// `http://` is rejected: these are shared document links that users click
    /// from a browser, and every provider serves them over TLS.
    pub fn validate(&self) -> DomainResult<()> {
        if self.label.trim().is_empty() {
            return Err(DomainError::invalid("label", "is required"));
        }
        if self.label.len() > 200 {
            return Err(DomainError::invalid(
                "label",
                "must be 200 characters or fewer",
            ));
        }
        let url = self.url.trim();
        if url.is_empty() {
            return Err(DomainError::invalid("url", "is required"));
        }
        if url.len() > 2000 {
            return Err(DomainError::invalid(
                "url",
                "must be 2000 characters or fewer",
            ));
        }
        if !url.starts_with("https://") {
            return Err(DomainError::invalid("url", "must start with https://"));
        }
        // Reject anything with whitespace or control characters: a link with a
        // newline in it can break an email header when the job is shared.
        if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(DomainError::invalid(
                "url",
                "must not contain spaces or line breaks",
            ));
        }
        // "https://" alone, or with no host.
        let host = &url["https://".len()..];
        if host.is_empty() || host.starts_with('/') {
            return Err(DomainError::invalid("url", "is not a valid address"));
        }
        Ok(())
    }

    #[must_use]
    pub fn normalised(mut self) -> Self {
        self.label = self.label.trim().to_string();
        self.url = self.url.trim().to_string();
        self
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;

    fn link(url: &str) -> JobLinkInput {
        JobLinkInput {
            label: "Site File".into(),
            url: url.into(),
            sort_order: None,
        }
    }

    #[test]
    fn accepts_a_real_production_dropbox_link() {
        let u = "https://www.dropbox.com/scl/fo/qgmvkiuajyihvhwa2now5/AA4_YblhPlKJqhn9v3ef6JM?r";
        assert!(link(u).validate().is_ok());
    }

    #[test]
    fn accepts_other_providers() {
        // The client asked for "any cloud based server. Google, Dropbox etc."
        for u in [
            "https://drive.google.com/drive/folders/1a2b3c",
            "https://onedrive.live.com/?id=root",
            "https://example.sharepoint.com/sites/site/Docs",
        ] {
            assert!(link(u).validate().is_ok(), "{u} should be accepted");
        }
    }

    #[test]
    fn rejects_non_https() {
        for u in [
            "http://www.dropbox.com/x",
            "ftp://host/x",
            "javascript:alert(1)",
            "/local/path",
        ] {
            assert!(link(u).validate().is_err(), "{u} should be rejected");
        }
    }

    #[test]
    fn rejects_urls_with_whitespace_or_newlines() {
        assert!(link("https://a.com/ b").validate().is_err());
        assert!(link("https://a.com\nX-Injected: 1").validate().is_err());
    }

    #[test]
    fn rejects_an_empty_host() {
        assert!(link("https://").validate().is_err());
        assert!(link("https:///path").validate().is_err());
    }

    #[test]
    fn label_is_required() {
        let mut l = link("https://a.com/x");
        l.label = "  ".into();
        assert!(l.validate().is_err());
    }

    #[test]
    fn normalisation_trims_both_fields() {
        let l = JobLinkInput {
            label: "  Variations  ".into(),
            url: "  https://a.com/x  ".into(),
            sort_order: None,
        }
        .normalised();
        assert_eq!(l.label, "Variations");
        assert_eq!(l.url, "https://a.com/x");
        assert!(l.validate().is_ok());
    }
}
