//! `WeatherProvider` over OpenWeatherMap.
//!
//! Two things wrap the upstream call: a short cache keyed on the rounded
//! coordinates, and the knowledge that a failure here must never matter. A
//! diary entry saves with or without a weather stamp (domain-rules R8), and
//! the weather endpoint is a convenience.

use async_trait::async_trait;
use pmk_domain::diary::WeatherStamp;
use pmk_domain::weather::{
    capitalise, condition_icon, local_hhmm, ms_to_kmh, round_1dp, Coordinates,
};
use pmk_ports::repository::WeatherProvider;
use pmk_ports::{PortError, PortResult};
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a reading stays good for.
///
/// Ten minutes, as the legacy had. Conditions do not change faster than a site
/// can act on them, and the upstream plan is metered.
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

/// How many locations are remembered at once.
const CACHE_MAX: usize = 500;

struct CacheEntry {
    stamp: WeatherStamp,
    stored_at: Instant,
}

pub struct OpenWeatherProvider {
    api_key: String,
    client: reqwest::Client,
    // A plain mutex, not an async one: the critical section is a map lookup
    // and never awaits, so an async lock would cost more than it saved.
    cache: Mutex<HashMap<String, CacheEntry>>,
}

impl std::fmt::Debug for OpenWeatherProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenWeatherProvider")
            .finish_non_exhaustive()
    }
}

impl OpenWeatherProvider {
    /// `None` when no API key is configured, which is not an error: the
    /// feature is simply unavailable and every caller degrades.
    #[must_use]
    pub fn new(api_key: &str) -> Option<Self> {
        let key = api_key.trim();
        if key.is_empty() {
            return None;
        }
        Some(Self {
            api_key: key.to_string(),
            client: reqwest::Client::builder()
                // A slow upstream must not hold a request open: the caller
                // would rather have no stamp than a hung page.
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap_or_default(),
            cache: Mutex::new(HashMap::new()),
        })
    }

    fn cached(&self, key: &str) -> Option<WeatherStamp> {
        let mut cache = self.cache.lock().ok()?;
        match cache.get(key) {
            Some(entry) if entry.stored_at.elapsed() < CACHE_TTL => Some(entry.stamp.clone()),
            Some(_) => {
                cache.remove(key);
                None
            }
            None => None,
        }
    }

    fn store(&self, key: String, stamp: &WeatherStamp) {
        let Ok(mut cache) = self.cache.lock() else {
            return;
        };
        if cache.len() >= CACHE_MAX {
            // Drop what has expired first; only if that frees nothing does
            // anything still-valid go.
            cache.retain(|_, e| e.stored_at.elapsed() < CACHE_TTL);
            if cache.len() >= CACHE_MAX {
                if let Some(oldest) = cache
                    .iter()
                    .min_by_key(|(_, e)| e.stored_at)
                    .map(|(k, _)| k.clone())
                {
                    cache.remove(&oldest);
                }
            }
        }
        cache.insert(
            key,
            CacheEntry {
                stamp: stamp.clone(),
                stored_at: Instant::now(),
            },
        );
    }
}

/// The subset of OpenWeatherMap's response this uses.
#[derive(serde::Deserialize, Default)]
struct Response {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    main: Main,
    #[serde(default)]
    wind: Wind,
    #[serde(default)]
    rain: Rain,
    #[serde(default)]
    sys: Sys,
    #[serde(default)]
    timezone: i64,
    #[serde(default)]
    weather: Vec<Condition>,
}

#[derive(serde::Deserialize, Default)]
struct Main {
    temp: Option<f64>,
}

#[derive(serde::Deserialize, Default)]
struct Wind {
    speed: Option<f64>,
}

#[derive(serde::Deserialize, Default)]
struct Rain {
    #[serde(rename = "1h")]
    last_hour: Option<f64>,
}

#[derive(serde::Deserialize, Default)]
struct Sys {
    sunrise: Option<i64>,
    sunset: Option<i64>,
}

#[derive(serde::Deserialize, Default)]
struct Condition {
    #[serde(default)]
    main: String,
}

fn dec(v: f64) -> Option<Decimal> {
    Decimal::from_f64(v)
}

#[async_trait]
impl WeatherProvider for OpenWeatherProvider {
    async fn current(&self, lat: f64, lon: f64) -> PortResult<WeatherStamp> {
        let coords = Coordinates::new(lat, lon)
            .map_err(|e| PortError::Storage(e.to_string()))?
            .rounded();
        let key = coords.cache_key();

        if let Some(hit) = self.cached(&key) {
            return Ok(hit);
        }

        let url = format!(
            "https://api.openweathermap.org/data/2.5/weather\
             ?lat={}&lon={}&units=metric&appid={}",
            coords.lat, coords.lon, self.api_key
        );
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| PortError::Unavailable {
                service: "weather",
                detail: e.to_string(),
            })?;

        if !response.status().is_success() {
            // The status is reported; the body is not. It can echo the API
            // key back in an error message.
            return Err(PortError::Unavailable {
                service: "weather",
                detail: format!("upstream returned {}", response.status()),
            });
        }

        let body: Response = response.json().await.map_err(|e| PortError::Unavailable {
            service: "weather",
            detail: e.to_string(),
        })?;

        let condition = body
            .weather
            .first()
            .map(|c| c.main.clone())
            .unwrap_or_else(|| "Clear".to_string());

        let stamp = WeatherStamp {
            location_name: body.name.filter(|n| !n.is_empty()),
            location_lat: dec(coords.lat),
            location_lng: dec(coords.lon),
            temperature: body.main.temp.map(round_1dp).and_then(dec),
            weather_icon: Some(condition_icon(&condition).to_string()),
            weather_condition: Some(capitalise(&condition)),
            wind_speed_kmh: body.wind.speed.map(ms_to_kmh).and_then(dec),
            // Absent rain means none fell, not that it is unknown.
            rainfall_mm: dec(body.rain.last_hour.map(round_1dp).unwrap_or(0.0)),
            sunrise_time: body.sys.sunrise.and_then(|t| local_hhmm(t, body.timezone)),
            sunset_time: body.sys.sunset.and_then(|t| local_hhmm(t, body.timezone)),
        };

        self.store(key, &stamp);
        Ok(stamp)
    }
}
