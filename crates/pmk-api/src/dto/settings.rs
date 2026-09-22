//! Company-settings shapes.
//!
//! No response here ever carries the SMTP password. The read path does not
//! select the column at all, so that is structural rather than a habit.

use pmk_app::settings::PreparedBrandingUpload;
use pmk_domain::settings::{
    Branding, BrandingInput, EmailSendMode, EmailSettings, EmailStatus, JobDisplayMode,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrandingDto {
    pub company_name: String,
    /// The API path that serves the image, not the object key: the bucket is
    /// closed and the key is an internal detail.
    pub logo_url: Option<String>,
    pub banner_url: Option<String>,
    pub primary_color: String,
    pub sidebar_color: String,
    pub job_display_mode: &'static str,
    pub email_send_mode: &'static str,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Branding> for BrandingDto {
    fn from(b: Branding) -> Self {
        Self {
            company_name: b.company_name,
            logo_url: b
                .logo_url
                .map(|_| "/api/settings/branding/logo".to_string()),
            banner_url: b
                .banner_url
                .map(|_| "/api/settings/branding/banner".to_string()),
            primary_color: b.primary_color,
            sidebar_color: b.sidebar_color,
            job_display_mode: b.job_display_mode.as_str(),
            email_send_mode: b.email_send_mode.as_str(),
            updated_at: b.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrandingRequest {
    pub company_name: String,
    pub primary_color: String,
    pub sidebar_color: String,
    #[serde(default)]
    pub job_display_mode: Option<String>,
    #[serde(default)]
    pub email_send_mode: Option<String>,
}

impl BrandingRequest {
    /// Parses the two closed sets, which are stored as free text.
    pub fn into_input(self) -> Result<BrandingInput, crate::error::ApiError> {
        let bad = |field: &'static str, allowed: &str| {
            crate::error::ApiError::new(
                axum::http::StatusCode::BAD_REQUEST,
                format!("must be one of {allowed}"),
            )
            .with_field(field)
        };
        let job_display_mode = match self.job_display_mode.as_deref() {
            None => JobDisplayMode::JobNumber,
            Some(v) => JobDisplayMode::parse(v)
                .ok_or_else(|| bad("jobDisplayMode", "job_number, job_name, job_address"))?,
        };
        let email_send_mode = match self.email_send_mode.as_deref() {
            None => EmailSendMode::Device,
            Some(v) => {
                EmailSendMode::parse(v).ok_or_else(|| bad("emailSendMode", "device, smtp"))?
            }
        };
        Ok(BrandingInput {
            company_name: self.company_name,
            primary_color: self.primary_color,
            sidebar_color: self.sidebar_color,
            job_display_mode,
            email_send_mode,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedBrandingUploadDto {
    pub stored_name: String,
    pub upload_url: String,
    pub expires_in_secs: u64,
}

impl From<PreparedBrandingUpload> for PreparedBrandingUploadDto {
    fn from(p: PreparedBrandingUpload) -> Self {
        Self {
            stored_name: p.stored_name,
            upload_url: p.upload_url,
            expires_in_secs: p.expires_in_secs,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareBrandingRequest {
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmBrandingRequest {
    pub stored_name: String,
    pub original_name: String,
    pub mime_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSettingsDto {
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i32>,
    pub smtp_user: Option<String>,
    pub smtp_from: Option<String>,
    pub smtp_secure: bool,
}

impl From<EmailSettings> for EmailSettingsDto {
    fn from(e: EmailSettings) -> Self {
        Self {
            smtp_host: e.smtp_host,
            smtp_port: e.smtp_port,
            smtp_user: e.smtp_user,
            smtp_from: e.smtp_from,
            smtp_secure: e.smtp_secure,
        }
    }
}

/// An absent password means "keep the stored one", which is how the settings
/// form can be re-saved without the password being present in the page.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSettingsRequest {
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i32>,
    pub smtp_user: Option<String>,
    pub smtp_pass: Option<String>,
    pub smtp_from: Option<String>,
    #[serde(default)]
    pub smtp_secure: bool,
}

impl From<EmailSettingsRequest> for EmailSettings {
    fn from(r: EmailSettingsRequest) -> Self {
        Self {
            smtp_host: r.smtp_host,
            smtp_port: r.smtp_port,
            smtp_user: r.smtp_user,
            smtp_from: r.smtp_from,
            smtp_secure: r.smtp_secure,
            // An empty string is the form saying "unchanged", not "blank it".
            smtp_pass: r.smtp_pass.filter(|p| !p.is_empty()),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailStatusDto {
    pub configured: bool,
    pub has_password: bool,
    pub secure: bool,
}

impl From<EmailStatus> for EmailStatusDto {
    fn from(s: EmailStatus) -> Self {
        Self {
            configured: s.configured,
            has_password: s.has_password,
            secure: s.secure,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestEmailRequest {
    pub to: String,
}
