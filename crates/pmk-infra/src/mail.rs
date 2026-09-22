//! SMTP sending, with the host pinned to a verified public address.
//!
//! The relay is chosen by whoever configures the company, so the address is
//! checked twice: once when the settings are saved, and again here at connect
//! time. Between those two moments DNS can change, and a name that resolved to
//! a public address when it was saved can resolve to `127.0.0.1` when it is
//! used. That is DNS rebinding, and re-resolving inside the SMTP client would
//! reopen the window this closes.
//!
//! The resolved address is handed to the transport directly, so the client
//! performs no lookup of its own. TLS still validates against the *name*, not
//! the address, so pinning does not weaken certificate checking.

use async_trait::async_trait;
use lettre::message::{header::ContentType, Mailbox};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message as LettreMessage, Tokio1Executor};
use pmk_domain::net::{is_blocked_smtp_host, is_private_ip};
use pmk_ports::{EmailSender, Message, PortError, PortResult, SmtpCredentials};
use std::net::IpAddr;

#[derive(Debug, Clone, Default)]
pub struct LettreEmailSender {
    /// Development only; see `SmtpConfig::allow_private_relays`.
    allow_private: bool,
}

impl LettreEmailSender {
    /// A sender that refuses every private address. What production uses.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            allow_private: false,
        }
    }

    /// A sender that will connect to a private address.
    ///
    /// For the local stack only, so the send path can be exercised against
    /// MailHog. The caller is expected to have logged a warning.
    #[must_use]
    pub const fn allowing_private_relays(allow: bool) -> Self {
        Self {
            allow_private: allow,
        }
    }
}

fn refused(detail: impl Into<String>) -> PortError {
    PortError::Unavailable {
        service: "smtp",
        detail: detail.into(),
    }
}

/// Resolves a host to public addresses, refusing anything private.
///
/// Fails closed at every step: a blocked name, no records, or *any* record
/// pointing somewhere private stops the send. One private answer among several
/// public ones is enough, because the client would be free to use it.
async fn resolve_public(host: &str, allow_private: bool) -> PortResult<IpAddr> {
    let host = host.trim();
    if allow_private {
        // Development only. Still resolves, so the rest of the path is the
        // same code production runs.
        return resolve_any(host).await;
    }
    if is_blocked_smtp_host(host) {
        return Err(refused("that host is not a permitted mail relay"));
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(ip) {
            return Err(refused("the relay resolves to a private address"));
        }
        return Ok(ip);
    }

    // Port 25 is arbitrary: `to_socket_addrs` needs one and only the address
    // is used. The real port comes from the settings.
    let target = format!("{host}:25");
    let addrs = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        target.to_socket_addrs().map(|i| i.collect::<Vec<_>>())
    })
    .await
    .map_err(|e| refused(e.to_string()))?
    .map_err(|e| refused(format!("could not resolve the relay: {e}")))?;

    if addrs.is_empty() {
        return Err(refused("the relay has no DNS records"));
    }
    if let Some(bad) = addrs.iter().find(|a| is_private_ip(a.ip())) {
        // Reported without the address: the caller configured a name, and
        // echoing what it resolved to would turn this into a scanner.
        tracing::warn!(host, resolved = %bad.ip(), "refused an SMTP relay on a private address");
        return Err(refused("the relay resolves to a private address"));
    }

    addrs
        .first()
        .map(|a| a.ip())
        .ok_or_else(|| refused("the relay has no DNS records"))
}

/// Resolves without the private-address check. Reached only when
/// `allow_private_relays` is on.
async fn resolve_any(host: &str) -> PortResult<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip);
    }
    let target = format!("{host}:25");
    let addrs = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        target.to_socket_addrs().map(|i| i.collect::<Vec<_>>())
    })
    .await
    .map_err(|e| refused(e.to_string()))?
    .map_err(|e| refused(format!("could not resolve the relay: {e}")))?;

    addrs
        .first()
        .map(|a| a.ip())
        .ok_or_else(|| refused("the relay has no DNS records"))
}

#[async_trait]
impl EmailSender for LettreEmailSender {
    async fn send(&self, creds: &SmtpCredentials, message: &Message) -> PortResult<()> {
        let ip = resolve_public(&creds.host, self.allow_private).await?;

        let from: Mailbox = creds
            .from
            .parse()
            .map_err(|e| PortError::Storage(format!("invalid from address: {e}")))?;

        let mut builder = LettreMessage::builder()
            .from(from)
            .subject(message.subject.clone());
        for to in &message.to {
            let mailbox: Mailbox = to
                .parse()
                .map_err(|e| PortError::Storage(format!("invalid recipient {to}: {e}")))?;
            builder = builder.to(mailbox);
        }

        let email = builder
            .header(if message.html {
                ContentType::TEXT_HTML
            } else {
                ContentType::TEXT_PLAIN
            })
            .body(message.body.clone())
            .map_err(|e| PortError::Storage(format!("could not build the message: {e}")))?;

        // Built against the pinned address, so no second lookup happens. TLS
        // is still negotiated for the configured *name*.
        let mut transport = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(ip.to_string())
            .port(creds.port);

        if creds.secure {
            let tls = TlsParameters::new(creds.host.clone())
                .map_err(|e| refused(format!("could not set up TLS: {e}")))?;
            transport = transport.tls(Tls::Wrapper(tls));
        } else {
            // Opportunistic STARTTLS: upgrade when the relay offers it, and
            // still deliver when it does not. MailHog, which the dev stack
            // runs, offers no TLS at all.
            let tls = TlsParameters::new(creds.host.clone())
                .map_err(|e| refused(format!("could not set up TLS: {e}")))?;
            transport = transport.tls(Tls::Opportunistic(tls));
        }

        if let (Some(user), Some(pass)) = (creds.user.as_ref(), creds.password.as_ref()) {
            if !user.is_empty() {
                transport = transport.credentials(Credentials::new(user.clone(), pass.clone()));
            }
        }

        transport
            .build()
            .send(email)
            .await
            .map_err(|e| refused(e.to_string()))?;
        Ok(())
    }
}
