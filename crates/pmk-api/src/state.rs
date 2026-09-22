//! Shared application state.

use std::sync::Arc;

use pmk_app::call_forward::CallForwardService;
use pmk_app::diary::DiaryService;
use pmk_app::identity::AuthService;
use pmk_app::jobs::JobService;
use pmk_app::media::MediaService;
use pmk_infra::Config;
use pmk_ports::Clock;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub auth: Arc<AuthService>,
    pub jobs: Arc<JobService>,
    pub diary: Arc<DiaryService>,
    pub call_forward: Arc<CallForwardService>,
    pub media: Arc<MediaService>,
    pub clock: Arc<dyn Clock>,
    pub pool: sqlx::PgPool,
    /// Flipped once migrations are confirmed applied. `/readyz` reports it, and
    /// data-bearing routes are gated on it -- the legacy server used the same
    /// idea via `isSchemaReady()`.
    pub ready: Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish_non_exhaustive()
    }
}

impl AppState {
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn mark_ready(&self) {
        self.ready.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
