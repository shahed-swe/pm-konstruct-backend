//! `pmk-api` entry point.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::ServiceExt;
use pmk_api::{build_router, AppState};
use pmk_app::identity::{token::TokenCodec, AuthService};
use pmk_infra::config::Config;
use pmk_infra::db::{connect, PoolConfig};
use pmk_infra::repo::{
    PgBillingRepository, PgCalendarRepository, PgCallForwardRepository, PgDashboardRepository,
    PgDiaryRepository, PgFormsRepository, PgJobLinkRepository, PgJobRepository,
    PgJobTaskRepository, PgMediaRepository, PgNotificationRepository, PgProgressRepository,
    PgRefreshTokenRepository, PgReportsRepository, PgSchedulerRepository, PgUserRepository,
};
use pmk_infra::telemetry;
use pmk_ports::SystemClock;
use tower::Layer;
use tower_http::normalize_path::NormalizePathLayer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Config is validated before anything else, so a missing secret fails
    // startup with an actionable message rather than at request time.
    let config = Config::load()?;
    telemetry::init(&config.telemetry);

    tracing::info!(
        port = config.server.port,
        timezone = %config.tenancy.default_timezone,
        "starting pmk-api"
    );

    let mut pool_cfg = PoolConfig::new(config.database.url.clone());
    pool_cfg.max_connections = config.database.max_connections;
    pool_cfg.min_connections = config.database.min_connections;
    pool_cfg.acquire_timeout = std::time::Duration::from_secs(config.database.acquire_timeout_secs);
    let pool = connect(&pool_cfg).await?;

    let users = Arc::new(PgUserRepository::new(pool.clone()));
    let refresh = Arc::new(PgRefreshTokenRepository::new(pool.clone()));
    let billing = Arc::new(PgBillingRepository::new(pool.clone()));
    let codec = TokenCodec::new(&config.auth.jwt_secret, config.access_ttl())?;
    let auth = Arc::new(AuthService::new(
        users,
        refresh,
        billing,
        codec,
        config.refresh_ttl(),
    )?);

    let job_repo = Arc::new(PgJobRepository::new(pool.clone()));
    let job_repo2 = job_repo.clone();
    let job_repo3 = job_repo.clone();
    let job_repo4 = job_repo.clone();
    let job_repo5 = job_repo.clone();
    let job_repo6 = job_repo.clone();
    let job_repo7 = job_repo.clone();
    let job_repo8 = job_repo.clone();
    let jobs = Arc::new(pmk_app::jobs::JobService::new(
        job_repo.clone(),
        Arc::new(PgJobTaskRepository::new(pool.clone())),
        Arc::new(PgJobLinkRepository::new(pool.clone())),
    ));
    let clock: Arc<dyn pmk_ports::Clock> = Arc::new(SystemClock::new(config.timezone()));

    // Starts LISTENing before any service that publishes to it is built, so
    // an event raised by the first request has somewhere to go.
    let events: Arc<dyn pmk_ports::EventBus> =
        Arc::new(pmk_infra::events::PgEventBus::start(pool.clone()).await?);
    let diary = Arc::new(pmk_app::diary::DiaryService::new(
        Arc::new(PgDiaryRepository::new(pool.clone())),
        job_repo,
        // Wired in Phase 14; until then entries save with no weather stamp,
        // which is the same graceful degradation R8 requires on an outage.
        None,
        clock.clone(),
        Some(events.clone()),
    ));

    let call_forward = Arc::new(pmk_app::call_forward::CallForwardService::new(
        Arc::new(PgCallForwardRepository::new(pool.clone())),
        job_repo2,
        clock.clone(),
    ));

    let diary_repo = Arc::new(PgDiaryRepository::new(pool.clone()));
    let store: Arc<dyn pmk_ports::ObjectStore> =
        Arc::new(pmk_infra::storage::S3ObjectStore::new(&config.storage));
    let store2 = store.clone();
    let media = Arc::new(pmk_app::media::MediaService::new(
        Arc::new(PgMediaRepository::new(pool.clone())),
        store,
        job_repo3,
        diary_repo,
    ));

    let scheduler = Arc::new(pmk_app::scheduler::SchedulerService::new(
        Arc::new(PgSchedulerRepository::new(pool.clone())),
        job_repo4,
    ));

    let forms = Arc::new(pmk_app::forms::FormsService::new(
        Arc::new(PgFormsRepository::new(pool.clone())),
        job_repo5,
        store2,
        clock.clone(),
    ));

    let dashboard = Arc::new(pmk_app::dashboard::DashboardService::new(
        Arc::new(PgDashboardRepository::new(pool.clone())),
        Arc::new(PgCalendarRepository::new(pool.clone())),
        job_repo6,
        clock.clone(),
    ));

    let progress = Arc::new(pmk_app::progress::ProgressService::new(
        Arc::new(PgProgressRepository::new(pool.clone())),
        job_repo7,
    ));

    let reports = Arc::new(pmk_app::reports::ReportsService::new(
        Arc::new(PgReportsRepository::new(pool.clone())),
        job_repo8,
        clock.clone(),
    ));

    let notifications = Arc::new(pmk_app::notifications::NotificationService::new(
        Arc::new(PgNotificationRepository::new(pool.clone())),
        config.notifications.vapid_public_key.clone(),
    ));

    let users = Arc::new(pmk_app::users::UsersService::new(
        Arc::new(PgUserRepository::new(pool.clone())),
        Arc::new(PgBillingRepository::new(pool.clone())),
        clock.clone(),
    ));

    let state = AppState {
        clock,
        config: Arc::new(config.clone()),
        auth,
        jobs,
        diary,
        call_forward,
        media,
        scheduler,
        forms,
        dashboard,
        progress,
        reports,
        notifications,
        users,
        events,
        pool: pool.clone(),
        ready: Arc::new(AtomicBool::new(false)),
    };

    // Readiness reflects the schema actually being present. Migrations are run
    // out of process by `pmk-cli migrate`, not on boot: the legacy server ran
    // ~1,100 lines of DDL at startup with no lock, so two instances booting
    // together raced.
    match sqlx::query("SELECT 1 FROM users LIMIT 1")
        .execute(&pool)
        .await
    {
        Ok(_) => {
            state.mark_ready();
            tracing::info!("schema present; serving traffic");
        }
        Err(e) => tracing::warn!(
            error = %e,
            "schema not ready -- run `pmk-cli migrate`. /readyz will report 503."
        ),
    }

    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "listening");

    // Express treated `/api/jobs` and `/api/jobs/` as the same route; axum
    // does not, so every collection endpoint would 404 on a trailing slash.
    // Trimming it here rather than inside `build_router` keeps that function
    // returning a plain `Router`, which the tests construct directly.
    let app = NormalizePathLayer::trim_trailing_slash().layer(build_router(state));

    axum::serve(
        listener,
        ServiceExt::<axum::extract::Request>::into_make_service(app),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

/// Drains in-flight requests on SIGTERM so a rolling deploy does not drop them.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("received SIGINT, shutting down"),
        () = term => tracing::info!("received SIGTERM, shutting down"),
    }
}
