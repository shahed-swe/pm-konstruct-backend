//! Structured logging and tracing.
//!
//! JSON in production so logs are queryable in Loki; pretty in development.

use crate::config::TelemetryConfig;

/// Initialises the global subscriber. Safe to call once; later calls are
/// ignored rather than panicking.
pub fn init(cfg: &TelemetryConfig) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,pmk=debug,sqlx=warn"));

    let registry = tracing_subscriber::registry().with(filter);

    if cfg.json_logs {
        let _ = registry
            .with(tracing_subscriber::fmt::layer().json().with_target(true))
            .try_init();
    } else {
        let _ = registry
            .with(tracing_subscriber::fmt::layer().with_target(true))
            .try_init();
    }

    if let Some(endpoint) = &cfg.otlp_endpoint {
        tracing::info!(otlp = %endpoint, "OTLP endpoint configured (exporter wired in Phase 13)");
    }
}
