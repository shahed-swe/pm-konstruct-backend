use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pmk_domain::diary::WeatherStamp;
use pmk_domain::ids::UserId;
use pmk_domain::weather::Coordinates;
use pmk_ports::repository::WeatherProvider;

use crate::identity::SessionUser;
use crate::{AppError, AppResult};

/// How many lookups one person may make per window.
///
/// The upstream plan is metered and billed to the company, so a page stuck in
/// a refresh loop must not be able to spend the month's quota. Ten a minute is
/// far more than a person moving around a site needs.
const RATE_LIMIT: u32 = 10;
const RATE_WINDOW: Duration = Duration::from_secs(60);

/// How many users are tracked at once.
///
/// Bounded so the map cannot grow without limit; exceeding it drops the
/// stalest entries, which at worst gives someone a fresh window early.
const RATE_MAP_MAX: usize = 10_000;

struct Window {
    count: u32,
    started: Instant,
}

pub struct WeatherService {
    /// `None` when no API key is configured. The feature is then unavailable
    /// rather than broken, which is how it has always been in production.
    provider: Option<Arc<dyn WeatherProvider>>,
    limits: Mutex<HashMap<i32, Window>>,
}

impl std::fmt::Debug for WeatherService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WeatherService")
            .field("configured", &self.provider.is_some())
            .finish_non_exhaustive()
    }
}

impl WeatherService {
    #[must_use]
    pub fn new(provider: Option<Arc<dyn WeatherProvider>>) -> Self {
        Self {
            provider,
            limits: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.provider.is_some()
    }

    pub async fn current(&self, s: &SessionUser, coords: Coordinates) -> AppResult<WeatherStamp> {
        // Counted before the configuration check, as the legacy did: a client
        // looping against an unconfigured deployment is still a client
        // looping, and the limiter is the thing that says so.
        self.check_rate(s.user.id)?;
        let Some(provider) = &self.provider else {
            return Err(AppError::FeatureUnavailable(
                "Weather service not configured".into(),
            ));
        };
        let coords = coords.rounded();
        Ok(provider.current(coords.lat, coords.lon).await?)
    }

    /// Counts this call against the caller's window.
    ///
    /// A poisoned lock lets the call through rather than failing it: the
    /// limiter is a cost control, and refusing real work because a mutex was
    /// left in a bad state would be the worse failure.
    fn check_rate(&self, user: UserId) -> AppResult<()> {
        let Ok(mut limits) = self.limits.lock() else {
            return Ok(());
        };

        if limits.len() >= RATE_MAP_MAX {
            limits.retain(|_, w| w.started.elapsed() < RATE_WINDOW);
            if limits.len() >= RATE_MAP_MAX {
                if let Some(stalest) = limits
                    .iter()
                    .min_by_key(|(_, w)| w.started)
                    .map(|(k, _)| *k)
                {
                    limits.remove(&stalest);
                }
            }
        }

        let window = limits.entry(user.get()).or_insert_with(|| Window {
            count: 0,
            started: Instant::now(),
        });
        if window.started.elapsed() >= RATE_WINDOW {
            window.count = 0;
            window.started = Instant::now();
        }
        if window.count >= RATE_LIMIT {
            return Err(AppError::TooManyRequests(
                "Too many weather requests. Please wait before trying again.".into(),
            ));
        }
        window.count += 1;
        Ok(())
    }
}
