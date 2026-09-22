//! Company settings: branding and outbound email.

use crate::error::{DomainError, DomainResult};
use crate::net::{is_blocked_smtp_host, validate_smtp_port};

/// How a job is labelled throughout the UI.
///
/// The variants repeat the `Job` prefix because the stored values do
/// (`job_number`, `job_name`, `job_address`) and keeping them in step makes
/// the mapping below obviously correct.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobDisplayMode {
    JobNumber,
    JobName,
    JobAddress,
}

impl JobDisplayMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::JobNumber => "job_number",
            Self::JobName => "job_name",
            Self::JobAddress => "job_address",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "job_number" => Some(Self::JobNumber),
            "job_name" => Some(Self::JobName),
            "job_address" => Some(Self::JobAddress),
            _ => None,
        }
    }
}

/// Where an emailed form is sent from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailSendMode {
    /// Hands the message to the user's own mail client. The default, and the
    /// only mode that works without SMTP configured.
    Device,
    /// Sends through the company's configured SMTP relay.
    Smtp,
}

impl EmailSendMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Smtp => "smtp",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "device" => Some(Self::Device),
            "smtp" => Some(Self::Smtp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Branding {
    pub company_name: String,
    pub logo_url: Option<String>,
    pub banner_url: Option<String>,
    pub primary_color: String,
    pub sidebar_color: String,
    pub job_display_mode: JobDisplayMode,
    pub email_send_mode: EmailSendMode,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// The values a company starts with.
///
/// Matches the column defaults, so a company with no row and a company with an
/// untouched one look identical.
#[must_use]
pub fn default_branding() -> Branding {
    Branding {
        company_name: "PM Konstruct".to_string(),
        logo_url: None,
        banner_url: None,
        primary_color: "#E84E1B".to_string(),
        sidebar_color: "#0f1117".to_string(),
        job_display_mode: JobDisplayMode::JobNumber,
        email_send_mode: EmailSendMode::Device,
        updated_at: chrono::DateTime::UNIX_EPOCH,
    }
}

#[derive(Debug, Clone)]
pub struct BrandingInput {
    pub company_name: String,
    pub primary_color: String,
    pub sidebar_color: String,
    pub job_display_mode: JobDisplayMode,
    pub email_send_mode: EmailSendMode,
}

impl BrandingInput {
    pub fn validate(&self) -> DomainResult<()> {
        if self.company_name.trim().is_empty() {
            return Err(DomainError::invalid("companyName", "a name is required"));
        }
        if self.company_name.chars().count() > 120 {
            return Err(DomainError::invalid(
                "companyName",
                "must be 120 characters or fewer",
            ));
        }
        validate_hex_colour("primaryColor", &self.primary_color)?;
        validate_hex_colour("sidebarColor", &self.sidebar_color)?;
        Ok(())
    }
}

/// A `#rrggbb` colour.
///
/// Validated rather than trusted because the value is interpolated into a CSS
/// custom property: anything else would let a manager inject into the
/// stylesheet of every page their colleagues load.
pub fn validate_hex_colour(field: &'static str, value: &str) -> DomainResult<()> {
    let v = value.trim();
    let valid = v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        Err(DomainError::invalid(field, "must be a #rrggbb colour"))
    }
}

/// A company's outbound mail relay.
#[derive(Debug, Clone, Default)]
pub struct EmailSettings {
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i32>,
    pub smtp_user: Option<String>,
    pub smtp_from: Option<String>,
    pub smtp_secure: bool,
    /// Never serialised out. Present only when a new one is being saved.
    pub smtp_pass: Option<String>,
}

impl EmailSettings {
    /// Is there enough here to send with?
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.smtp_host
            .as_deref()
            .is_some_and(|h| !h.trim().is_empty())
            && self
                .smtp_from
                .as_deref()
                .is_some_and(|f| !f.trim().is_empty())
    }

    pub fn validate(&self) -> DomainResult<()> {
        if let Some(host) = self.smtp_host.as_deref() {
            let host = host.trim();
            if !host.is_empty() && is_blocked_smtp_host(host) {
                // The server will connect to whatever is stored here, so an
                // address inside its own network is refused at the point of
                // saving rather than at the point of sending.
                return Err(DomainError::invalid(
                    "smtpHost",
                    "that host is not a permitted mail relay",
                ));
            }
        }
        if let Some(port) = self.smtp_port {
            validate_smtp_port(port)?;
        }
        if let Some(from) = self.smtp_from.as_deref() {
            let from = from.trim();
            if !from.is_empty() && !crate::identity::looks_like_email(from) {
                return Err(DomainError::invalid("smtpFrom", "must be a valid address"));
            }
        }
        Ok(())
    }
}

/// What the status endpoint reports, without disclosing the credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmailStatus {
    pub configured: bool,
    pub has_password: bool,
    pub secure: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branding() -> BrandingInput {
        BrandingInput {
            company_name: "BuildSmart".into(),
            primary_color: "#E84E1B".into(),
            sidebar_color: "#0f1117".into(),
            job_display_mode: JobDisplayMode::JobNumber,
            email_send_mode: EmailSendMode::Device,
        }
    }

    #[test]
    fn a_valid_branding_payload_is_accepted() {
        assert!(branding().validate().is_ok());
    }

    #[test]
    fn colours_must_be_six_digit_hex() {
        for bad in ["red", "#fff", "#12345g", "#1234567", "", "E84E1B", "#E84E1"] {
            let b = BrandingInput {
                primary_color: bad.into(),
                ..branding()
            };
            assert!(b.validate().is_err(), "{bad:?} should be rejected");
        }
        // Upper and lower case both fine.
        for good in ["#ffffff", "#000000", "#AbCdEf"] {
            let b = BrandingInput {
                primary_color: good.into(),
                ..branding()
            };
            assert!(b.validate().is_ok(), "{good:?} should be accepted");
        }
    }

    #[test]
    fn a_colour_cannot_smuggle_css_into_the_page() {
        // The value ends up in a CSS custom property on every page.
        for attack in [
            "#fff; } body { display:none } .x {",
            "red;background:url(http://evil/)",
            "#000000\"",
        ] {
            let b = BrandingInput {
                sidebar_color: attack.into(),
                ..branding()
            };
            assert!(b.validate().is_err(), "{attack:?} should be rejected");
        }
    }

    #[test]
    fn a_blank_company_name_is_rejected() {
        let b = BrandingInput {
            company_name: "  ".into(),
            ..branding()
        };
        assert!(b.validate().is_err());
    }

    #[test]
    fn the_display_modes_round_trip() {
        // The three the settings route accepts (routes/settings.ts:66).
        for m in [
            JobDisplayMode::JobNumber,
            JobDisplayMode::JobName,
            JobDisplayMode::JobAddress,
        ] {
            assert_eq!(JobDisplayMode::parse(m.as_str()), Some(m));
        }
        assert_eq!(JobDisplayMode::parse("client"), None);
        // Not the bare forms: the stored values are prefixed.
        assert_eq!(JobDisplayMode::parse("name"), None);
        assert_eq!(JobDisplayMode::parse("address"), None);
    }

    #[test]
    fn the_send_modes_round_trip() {
        for m in [EmailSendMode::Device, EmailSendMode::Smtp] {
            assert_eq!(EmailSendMode::parse(m.as_str()), Some(m));
        }
        assert_eq!(EmailSendMode::parse("server"), None);
    }

    #[test]
    fn the_defaults_match_the_column_defaults() {
        let d = default_branding();
        assert_eq!(d.primary_color, "#E84E1B");
        assert_eq!(d.sidebar_color, "#0f1117");
        assert_eq!(d.job_display_mode, JobDisplayMode::JobNumber);
        assert_eq!(d.email_send_mode, EmailSendMode::Device);
        assert!(d.logo_url.is_none() && d.banner_url.is_none());
    }

    #[test]
    fn an_smtp_host_inside_the_servers_own_network_is_refused_on_save() {
        for bad in ["localhost", "127.0.0.1", "169.254.169.254", "mail.internal"] {
            let e = EmailSettings {
                smtp_host: Some(bad.into()),
                ..EmailSettings::default()
            };
            assert!(e.validate().is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn a_real_relay_is_accepted() {
        let e = EmailSettings {
            smtp_host: Some("smtp.gmail.com".into()),
            smtp_port: Some(587),
            smtp_from: Some("site@buildsmart.com.au".into()),
            ..EmailSettings::default()
        };
        assert!(e.validate().is_ok());
    }

    #[test]
    fn clearing_the_host_is_allowed() {
        // Emptying the field is how a company turns server sending off.
        let e = EmailSettings {
            smtp_host: Some(String::new()),
            ..EmailSettings::default()
        };
        assert!(e.validate().is_ok());
        assert!(!e.is_configured());
    }

    #[test]
    fn an_out_of_range_port_is_refused() {
        let e = EmailSettings {
            smtp_host: Some("smtp.gmail.com".into()),
            smtp_port: Some(0),
            ..EmailSettings::default()
        };
        assert!(e.validate().is_err());
    }

    #[test]
    fn a_malformed_from_address_is_refused() {
        let e = EmailSettings {
            smtp_host: Some("smtp.gmail.com".into()),
            smtp_from: Some("not-an-address".into()),
            ..EmailSettings::default()
        };
        assert!(e.validate().is_err());
    }

    #[test]
    fn configuration_needs_both_a_host_and_a_from_address() {
        let host_only = EmailSettings {
            smtp_host: Some("smtp.gmail.com".into()),
            ..EmailSettings::default()
        };
        assert!(!host_only.is_configured());

        let both = EmailSettings {
            smtp_host: Some("smtp.gmail.com".into()),
            smtp_from: Some("a@b.com".into()),
            ..EmailSettings::default()
        };
        assert!(both.is_configured());
    }
}
