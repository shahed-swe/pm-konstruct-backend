use std::sync::Arc;

use pmk_domain::media::{kind_for, object_key, validate_upload, FileType, UploadRequest};
use pmk_domain::settings::{Branding, BrandingInput, EmailSettings, EmailStatus};
use pmk_domain::tenant::CompanyId;
use pmk_domain::DomainError;
use pmk_ports::repository::SettingsRepository;
use pmk_ports::{Message, ObjectStore};

use crate::identity::SessionUser;
use crate::media::upload;
use crate::{AppError, AppResult};

/// The largest logo or banner a company may upload.
///
/// Five megabytes, as the legacy multer limit was. A site photo needs room; a
/// logo does not.
const MAX_BRANDING_BYTES: u64 = 5 * 1024 * 1024;

/// A presigned slot for a logo or banner.
#[derive(Debug, Clone)]
pub struct PreparedBrandingUpload {
    pub stored_name: String,
    pub upload_url: String,
    pub expires_in_secs: u64,
}

pub struct SettingsService {
    settings: Arc<dyn SettingsRepository>,
    store: Arc<dyn ObjectStore>,
    mail: Arc<crate::mail::Mailer>,
}

impl std::fmt::Debug for SettingsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsService").finish_non_exhaustive()
    }
}

impl SettingsService {
    #[must_use]
    pub fn new(
        settings: Arc<dyn SettingsRepository>,
        store: Arc<dyn ObjectStore>,
        mail: Arc<crate::mail::Mailer>,
    ) -> Self {
        Self {
            settings,
            store,
            mail,
        }
    }

    // ── branding ────────────────────────────────────────────────────────────

    pub async fn branding(&self, s: &SessionUser) -> AppResult<Branding> {
        Ok(self.settings.branding(s.principal.scope()).await?)
    }

    /// The branding the login page shows, before anyone has signed in.
    pub async fn default_branding(&self) -> AppResult<Branding> {
        Ok(self.settings.default_branding().await?)
    }

    pub async fn set_branding(
        &self,
        s: &SessionUser,
        input: &BrandingInput,
    ) -> AppResult<Branding> {
        input.validate().map_err(AppError::Domain)?;
        Ok(self
            .settings
            .set_branding(s.principal.scope(), input)
            .await?)
    }

    /// Step one of replacing a logo or banner: a presigned PUT.
    pub async fn prepare_image(
        &self,
        s: &SessionUser,
        banner: bool,
        file: &UploadRequest,
    ) -> AppResult<PreparedBrandingUpload> {
        assert_image(&file.mime)?;
        if file.size_bytes > MAX_BRANDING_BYTES {
            return Err(AppError::Domain(DomainError::invalid(
                "sizeBytes",
                "a logo or banner must be 5 MB or smaller",
            )));
        }
        let validated = validate_upload(file).map_err(AppError::Domain)?;

        let key = self.key_for(s.principal.company_id(), banner, &validated.stored_name);
        let presigned = self
            .store
            .presign_put(&key, validated.kind.mime, validated.size_bytes)
            .await?;

        Ok(PreparedBrandingUpload {
            stored_name: validated.stored_name,
            upload_url: presigned.url,
            expires_in_secs: presigned.expires_in_secs,
        })
    }

    /// Step two: verify what landed, then point the branding at it.
    ///
    /// The object the new one replaces is removed, so a company changing its
    /// logo weekly does not accumulate every past version.
    pub async fn confirm_image(
        &self,
        s: &SessionUser,
        banner: bool,
        stored_name: &str,
        original_name: &str,
        mime_type: &str,
    ) -> AppResult<Branding> {
        assert_image(mime_type)?;
        let key = self.key_for(s.principal.company_id(), banner, stored_name);

        let verified = upload::verify(self.store.as_ref(), &key, mime_type, original_name).await?;
        if verified.kind.file_type != FileType::Photo {
            upload::discard(self.store.as_ref(), &key).await;
            return Err(AppError::Domain(DomainError::invalid(
                "mimeType",
                "Only image files are allowed (JPG, PNG, GIF, WebP)",
            )));
        }

        let previous = self
            .settings
            .set_branding_image(s.principal.scope(), banner, Some(&key))
            .await?;
        self.sweep(previous.as_deref(), &key).await;
        Ok(self.settings.branding(s.principal.scope()).await?)
    }

    /// Clears the logo or banner and removes the object behind it.
    pub async fn clear_image(&self, s: &SessionUser, banner: bool) -> AppResult<Branding> {
        let previous = self
            .settings
            .set_branding_image(s.principal.scope(), banner, None)
            .await?;
        self.sweep(previous.as_deref(), "").await;
        Ok(self.settings.branding(s.principal.scope()).await?)
    }

    /// A short-lived URL for the default company's branding image.
    ///
    /// Used by the login page, which has no session to say whose branding to
    /// show.
    pub async fn default_image_url(&self, banner: bool) -> AppResult<String> {
        let branding = self.settings.default_branding().await?;
        let key = if banner {
            branding.banner_url
        } else {
            branding.logo_url
        }
        .ok_or_else(|| AppError::Domain(DomainError::not_found("Image")))?;
        Ok(self.store.presign_get(&key).await?.url)
    }

    /// A short-lived URL for a stored branding image.
    ///
    /// The object is not public: the bucket stays closed and the API hands out
    /// a signed URL, so a logo cannot be enumerated across tenants by guessing
    /// filenames.
    pub async fn image_url(&self, s: &SessionUser, banner: bool) -> AppResult<String> {
        let branding = self.settings.branding(s.principal.scope()).await?;
        let key = if banner {
            branding.banner_url
        } else {
            branding.logo_url
        }
        .ok_or_else(|| AppError::Domain(DomainError::not_found("Image")))?;

        Ok(self.store.presign_get(&key).await?.url)
    }

    /// Removes a displaced object, unless it is the one just stored.
    async fn sweep(&self, previous: Option<&str>, keep: &str) {
        if let Some(old) = previous {
            if !old.is_empty() && old != keep {
                upload::discard(self.store.as_ref(), old).await;
            }
        }
    }

    fn key_for(&self, company: CompanyId, banner: bool, stored_name: &str) -> String {
        // Tenant-prefixed like every other object, so a bucket policy can
        // enforce isolation as a third layer.
        object_key(
            company.get(),
            if banner {
                "branding/banner"
            } else {
                "branding/logo"
            },
            0,
            stored_name,
        )
    }

    // ── email ───────────────────────────────────────────────────────────────

    pub async fn email_settings(&self, s: &SessionUser) -> AppResult<EmailSettings> {
        Ok(self.settings.email_settings(s.principal.scope()).await?)
    }

    /// What is configured, without disclosing any of it.
    pub async fn email_status(&self, s: &SessionUser) -> AppResult<EmailStatus> {
        let scope = s.principal.scope();
        let settings = self.settings.email_settings(scope).await?;
        Ok(EmailStatus {
            configured: settings.is_configured(),
            has_password: self.settings.has_smtp_password(scope).await?,
            secure: settings.smtp_secure,
        })
    }

    pub async fn set_email_settings(
        &self,
        s: &SessionUser,
        input: &EmailSettings,
    ) -> AppResult<EmailSettings> {
        input.validate().map_err(AppError::Domain)?;
        Ok(self
            .settings
            .set_email_settings(s.principal.scope(), input)
            .await?)
    }

    /// Sends a test message to the address given, through the company's relay.
    pub async fn send_test_email(&self, s: &SessionUser, to: &str) -> AppResult<()> {
        if !pmk_domain::identity::looks_like_email(to) {
            return Err(AppError::Domain(DomainError::invalid(
                "to",
                "must be a valid address",
            )));
        }
        self.mail
            .send(
                s,
                &Message {
                    to: vec![to.to_string()],
                    subject: "PM Konstruct test message".to_string(),
                    body: "Your SMTP settings are working. This message was sent by PM Konstruct."
                        .to_string(),
                    html: false,
                },
            )
            .await?;
        Ok(())
    }

    /// Sends a message through the company's configured relay.
    ///
    /// Shared with the diary and form paths via the mailer, so all three are
    /// refused the same way when nothing is configured.
    pub async fn send(&self, s: &SessionUser, message: &Message) -> AppResult<()> {
        self.mail.send(s, message).await
    }
}

/// Branding images are images. The legacy message is kept, because the UI
/// shows it verbatim.
fn assert_image(mime: &str) -> AppResult<()> {
    let ok = kind_for(mime).is_some_and(|k| k.file_type == FileType::Photo);
    if ok {
        Ok(())
    } else {
        Err(AppError::Domain(DomainError::invalid(
            "mimeType",
            "Only image files are allowed (JPG, PNG, GIF, WebP)",
        )))
    }
}
