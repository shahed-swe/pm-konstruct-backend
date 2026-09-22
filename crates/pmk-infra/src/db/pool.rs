//! Connection pool.
//!
//! The application connects as `pmk_app`, which is subject to row-level
//! security (migration 0004). It deliberately does **not** have BYPASSRLS: the
//! worker and CLI use `pmk_admin`/`pmk_migrator` for the genuinely cross-tenant
//! work (billing webhook delivery, media GC).

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::time::Duration;

pub type PgPool = sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout: Duration,
    pub idle_timeout: Duration,
    /// Fails startup rather than serving requests against an unreachable
    /// database — the legacy server started and 503'd instead.
    pub test_before_acquire: bool,
}

impl PoolConfig {
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: 20,
            min_connections: 2,
            acquire_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(600),
            test_before_acquire: true,
        }
    }
}

pub async fn connect(cfg: &PoolConfig) -> Result<PgPool, sqlx::Error> {
    let opts: PgConnectOptions = cfg.url.parse()?;
    PgPoolOptions::new()
        .max_connections(cfg.max_connections)
        .min_connections(cfg.min_connections)
        .acquire_timeout(cfg.acquire_timeout)
        .idle_timeout(cfg.idle_timeout)
        .test_before_acquire(cfg.test_before_acquire)
        .connect_with(opts)
        .await
}
