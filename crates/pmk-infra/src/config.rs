//! Layered configuration: defaults -> optional file -> environment.
//!
//! Validated once at startup and then immutable. The legacy server started
//! happily with missing configuration and failed later at request time; this
//! refuses to boot instead, with a message naming the missing key.
//!
//! Environment variables use a `PMK__SECTION__KEY` prefix, e.g.
//! `PMK__DATABASE__URL`, so nothing collides with unrelated variables.

use std::time::Duration;

use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    pub storage: StorageConfig,
    pub tenancy: TenancyConfig,
    pub telemetry: TelemetryConfig,
    pub notifications: NotificationsConfig,
    pub smtp: SmtpConfig,
    pub weather: WeatherConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// Exact allowed browser origins. The legacy API used `cors()` with no
    /// arguments, i.e. a wildcard origin on a credential-bearing API.
    pub cors_allowed_origins: Vec<String>,
    pub request_timeout_secs: u64,
    pub max_body_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// HS256 signing secret. Must be at least 32 bytes; startup fails otherwise.
    pub jwt_secret: String,
    /// Legacy tokens lasted 8h with no refresh and no revocation.
    pub access_token_ttl_secs: u64,
    pub refresh_token_ttl_secs: u64,
    pub cookie_domain: Option<String>,
    /// Off only for local HTTP development.
    pub cookie_secure: bool,
    pub login_rate_limit_per_minute: u32,
    /// Authorises first-run setup. Empty disables the endpoint entirely,
    /// which is what a deployment past its first run wants -- leaving it set
    /// means anyone holding it could create a company on this installation.
    pub setup_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Where the API reaches storage directly (head, range reads, delete).
    pub endpoint: String,
    /// Host baked into presigned URLs, for clients rather than the API.
    ///
    /// Defaults to `endpoint`. They differ whenever the API sits on a private
    /// network and clients do not: MinIO behind Cloudflare in production, and
    /// a container talking to `host.docker.internal` while the browser uses
    /// `localhost` in development.
    pub public_endpoint: Option<String>,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// MinIO needs path-style addressing; AWS S3 does not.
    pub force_path_style: bool,
    pub presign_ttl_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenancyConfig {
    /// Canonical timezone for every date boundary -- delay computation, diary
    /// dates, scheduler days, report ranges. Never the host's local zone.
    /// See domain-rules.md R1 and the cross-cutting timezone note.
    pub default_timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherConfig {
    /// OpenWeatherMap key. Empty means the feature is simply not offered,
    /// which is how production has always run: no key has ever been set.
    pub openweather_api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtpConfig {
    /// Lets the server connect to a mail relay on a private address.
    ///
    /// **Development only.** A company's SMTP host is chosen by whoever
    /// configures the account, and the server then connects to it; allowing
    /// private addresses turns that into server-side request forgery -- an
    /// authenticated manager could point it at a cloud metadata endpoint or an
    /// internal admin port and have the server reach it for them.
    ///
    /// It exists so the local stack can send to MailHog, which necessarily
    /// listens on a private address. It defaults to `false`, production never
    /// sets it, and startup logs a warning when it is on so a misconfigured
    /// deployment is visible rather than silent.
    pub allow_private_relays: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationsConfig {
    /// Handed to the browser so it can create a push subscription. Public by
    /// design; the private half never leaves the server.
    ///
    /// Empty means push is simply not offered, which is not an error -- the
    /// app works without it, and it has never been configured in production.
    pub vapid_public_key: String,
    /// Signs push messages. Never logged and never served.
    pub vapid_private_key: String,
    /// The `mailto:` the push services contact about delivery problems.
    pub vapid_subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    pub json_logs: bool,
    pub otlp_endpoint: Option<String>,
    pub service_name: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".into(),
                port: 8080,
                cors_allowed_origins: vec!["http://localhost:3000".into()],
                request_timeout_secs: 30,
                max_body_bytes: 2 * 1024 * 1024,
            },
            database: DatabaseConfig {
                url: String::new(),
                max_connections: 20,
                min_connections: 2,
                acquire_timeout_secs: 5,
            },
            auth: AuthConfig {
                jwt_secret: String::new(),
                access_token_ttl_secs: 15 * 60,
                refresh_token_ttl_secs: 30 * 24 * 60 * 60,
                cookie_domain: None,
                cookie_secure: true,
                login_rate_limit_per_minute: 10,
                setup_secret: String::new(),
            },
            storage: StorageConfig {
                endpoint: "http://localhost:9000".into(),
                public_endpoint: None,
                bucket: "pmk-media".into(),
                region: "us-east-1".into(),
                access_key_id: String::new(),
                secret_access_key: String::new(),
                force_path_style: true,
                presign_ttl_secs: 900,
            },
            tenancy: TenancyConfig {
                default_timezone: "Australia/Melbourne".into(),
            },
            telemetry: TelemetryConfig {
                json_logs: false,
                otlp_endpoint: None,
                service_name: "pmk-api".into(),
            },
            weather: WeatherConfig {
                openweather_api_key: String::new(),
            },
            smtp: SmtpConfig {
                allow_private_relays: false,
            },
            notifications: NotificationsConfig {
                vapid_public_key: String::new(),
                vapid_private_key: String::new(),
                vapid_subject: "mailto:admin@pmkonstruct.com.au".into(),
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    // Boxed: figment::Error is ~208 bytes and would make every Result in the
    // startup path that large.
    #[error("configuration could not be read: {0}")]
    Read(#[from] Box<figment::Error>),

    #[error("{key} is required but not set. {hint}")]
    Missing {
        key: &'static str,
        hint: &'static str,
    },

    #[error("{key} is invalid: {reason}")]
    Invalid { key: &'static str, reason: String },
}

impl Config {
    /// Loads and validates. Returns an actionable error rather than panicking.
    pub fn load() -> Result<Self, ConfigError> {
        let cfg: Self = Figment::from(Serialized::defaults(Self::default()))
            .merge(Toml::file("pmk.toml"))
            .merge(Env::prefixed("PMK__").split("__"))
            .extract()
            .map_err(Box::new)?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.database.url.trim().is_empty() {
            return Err(ConfigError::Missing {
                key: "PMK__DATABASE__URL",
                hint: "e.g. postgres://pmk:pmk@localhost:55432/pmk",
            });
        }
        if self.auth.jwt_secret.trim().is_empty() {
            return Err(ConfigError::Missing {
                key: "PMK__AUTH__JWT_SECRET",
                hint: "generate one with: openssl rand -base64 48",
            });
        }
        // 32 bytes of entropy is the floor for HS256; shorter secrets are
        // brute-forceable and the legacy dev fallback was much shorter.
        if self.auth.jwt_secret.len() < 32 {
            return Err(ConfigError::Invalid {
                key: "PMK__AUTH__JWT_SECRET",
                reason: format!(
                    "must be at least 32 characters, got {}",
                    self.auth.jwt_secret.len()
                ),
            });
        }
        if self
            .tenancy
            .default_timezone
            .parse::<chrono_tz::Tz>()
            .is_err()
        {
            return Err(ConfigError::Invalid {
                key: "PMK__TENANCY__DEFAULT_TIMEZONE",
                reason: format!(
                    "'{}' is not an IANA timezone",
                    self.tenancy.default_timezone
                ),
            });
        }
        if self.server.cors_allowed_origins.iter().any(|o| o == "*") {
            return Err(ConfigError::Invalid {
                key: "PMK__SERVER__CORS_ALLOWED_ORIGINS",
                reason: "'*' is not permitted on a credential-bearing API".into(),
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn timezone(&self) -> chrono_tz::Tz {
        // Validated in `validate`, so this cannot fail in practice; fall back
        // to UTC rather than panicking if it somehow does.
        self.tenancy
            .default_timezone
            .parse()
            .unwrap_or(chrono_tz::UTC)
    }

    #[must_use]
    pub fn access_ttl(&self) -> Duration {
        Duration::from_secs(self.auth.access_token_ttl_secs)
    }

    #[must_use]
    pub fn refresh_ttl(&self) -> Duration {
        Duration::from_secs(self.auth.refresh_token_ttl_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Config {
        let mut c = Config::default();
        c.database.url = "postgres://localhost/pmk".into();
        c.auth.jwt_secret = "x".repeat(48);
        c
    }

    #[test]
    fn a_valid_config_passes() {
        assert!(valid().validate().is_ok());
    }

    #[test]
    fn missing_database_url_is_named_in_the_error() {
        let mut c = valid();
        c.database.url = String::new();
        let e = c.validate().expect_err("should fail");
        assert!(e.to_string().contains("PMK__DATABASE__URL"), "{e}");
    }

    #[test]
    fn short_jwt_secret_is_rejected() {
        let mut c = valid();
        c.auth.jwt_secret = "tooshort".into();
        let e = c.validate().expect_err("should fail");
        assert!(e.to_string().contains("at least 32"), "{e}");
    }

    #[test]
    fn wildcard_cors_origin_is_rejected() {
        let mut c = valid();
        c.server.cors_allowed_origins = vec!["*".into()];
        assert!(c.validate().is_err(), "wildcard CORS must not be allowed");
    }

    #[test]
    fn bogus_timezone_is_rejected() {
        let mut c = valid();
        c.tenancy.default_timezone = "Mars/Olympus".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn default_timezone_is_melbourne_not_the_host_zone() {
        assert_eq!(valid().timezone(), chrono_tz::Australia::Melbourne);
    }
}
