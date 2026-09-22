//! Weather shapes.

use pmk_domain::diary::WeatherStamp;
use serde::Serialize;

/// The stamp, with the decimals rendered as numbers rather than strings.
///
/// The legacy returned JSON numbers here and the UI does arithmetic on them,
/// so serialising `Decimal` in its string form would break the display.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherDto {
    pub location_name: Option<String>,
    pub location_lat: Option<f64>,
    pub location_lng: Option<f64>,
    pub temperature: Option<f64>,
    pub weather_condition: Option<String>,
    pub weather_icon: Option<String>,
    pub wind_speed_kmh: Option<f64>,
    pub rainfall_mm: Option<f64>,
    pub sunrise_time: Option<String>,
    pub sunset_time: Option<String>,
}

fn f64_of(d: Option<rust_decimal::Decimal>) -> Option<f64> {
    use rust_decimal::prelude::ToPrimitive;
    d.and_then(|v| v.to_f64())
}

impl From<WeatherStamp> for WeatherDto {
    fn from(w: WeatherStamp) -> Self {
        Self {
            location_name: w.location_name,
            location_lat: f64_of(w.location_lat),
            location_lng: f64_of(w.location_lng),
            temperature: f64_of(w.temperature),
            weather_condition: w.weather_condition,
            weather_icon: w.weather_icon,
            wind_speed_kmh: f64_of(w.wind_speed_kmh),
            rainfall_mm: f64_of(w.rainfall_mm),
            sunrise_time: w.sunrise_time,
            sunset_time: w.sunset_time,
        }
    }
}

/// The structured reading stored against a diary entry.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherSnapshotDto {
    pub diary_entry_id: i32,
    pub temperature_c: Option<f64>,
    pub conditions: Option<String>,
    pub rain: Option<String>,
    pub wind_description: Option<String>,
    pub wind_speed_kmh: Option<f64>,
    pub humidity_pct: Option<i32>,
    pub source: String,
    pub snapshot_at: chrono::DateTime<chrono::Utc>,
}

impl From<pmk_ports::repository::WeatherSnapshot> for WeatherSnapshotDto {
    fn from(w: pmk_ports::repository::WeatherSnapshot) -> Self {
        Self {
            diary_entry_id: w.diary_entry_id.get(),
            temperature_c: f64_of(w.temperature_c),
            conditions: w.conditions,
            rain: w.rain,
            wind_description: w.wind_description,
            wind_speed_kmh: f64_of(w.wind_speed_kmh),
            humidity_pct: w.humidity_pct,
            source: w.source,
            snapshot_at: w.snapshot_at,
        }
    }
}
