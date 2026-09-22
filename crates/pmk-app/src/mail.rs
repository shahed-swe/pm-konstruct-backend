//! Sending through a company's configured relay.
//!
//! Extracted so the several features that send -- the settings test message,
//! a diary entry, an inspection form -- all go out the same way and are
//! refused the same way when nothing is configured. Otherwise each would grow
//! its own copy of "which host, which credentials, and is it even set up".

use std::sync::Arc;

use pmk_domain::DomainError;
use pmk_ports::repository::SettingsRepository;
use pmk_ports::{EmailSender, Message, SmtpCredentials};

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

/// The submission port, which is what the column defaults to.
const DEFAULT_SMTP_PORT: u16 = 587;

pub struct Mailer {
    settings: Arc<dyn SettingsRepository>,
    sender: Arc<dyn EmailSender>,
}

impl std::fmt::Debug for Mailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mailer").finish_non_exhaustive()
    }
}

impl Mailer {
    #[must_use]
    pub fn new(settings: Arc<dyn SettingsRepository>, sender: Arc<dyn EmailSender>) -> Self {
        Self { settings, sender }
    }

    /// Sends one message through the caller's company relay.
    pub async fn send(&self, s: &SessionUser, message: &Message) -> AppResult<()> {
        let creds = self.credentials(s).await?;
        self.sender.send(&creds, message).await?;
        Ok(())
    }

    /// Assembles the relay credentials, including the stored password.
    async fn credentials(&self, s: &SessionUser) -> AppResult<SmtpCredentials> {
        let scope = s.principal.scope();
        let settings = self.settings.email_settings(scope).await?;
        if !settings.is_configured() {
            return Err(AppError::Domain(DomainError::invalid(
                "smtpHost",
                "Configure an SMTP host and sender address before sending email",
            )));
        }
        // Re-validated on the way out: stored settings may predate the address
        // checks, and the relay is about to be connected to.
        settings.validate().map_err(AppError::Domain)?;

        Ok(SmtpCredentials {
            host: settings.smtp_host.unwrap_or_default(),
            port: settings
                .smtp_port
                .and_then(|p| u16::try_from(p).ok())
                .unwrap_or(DEFAULT_SMTP_PORT),
            user: settings.smtp_user,
            password: self.settings.smtp_password(scope).await?,
            from: settings.smtp_from.unwrap_or_default(),
            secure: settings.smtp_secure,
        })
    }
}
