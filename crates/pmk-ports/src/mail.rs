//! Outbound email.

use async_trait::async_trait;

use crate::PortResult;

/// Where and how to connect to a company's relay.
#[derive(Debug, Clone)]
pub struct SmtpCredentials {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
    pub from: String,
    /// Implicit TLS on connect, as opposed to STARTTLS.
    pub secure: bool,
}

/// A message to send.
#[derive(Debug, Clone)]
pub struct Message {
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
    /// `true` when `body` is HTML.
    pub html: bool,
}

#[async_trait]
pub trait EmailSender: Send + Sync {
    /// Sends one message through the given relay.
    ///
    /// The implementation resolves the host and refuses to connect to a
    /// private address, re-checking at connect time rather than trusting the
    /// check made when the settings were saved: DNS can change in between.
    async fn send(&self, creds: &SmtpCredentials, message: &Message) -> PortResult<()>;
}
