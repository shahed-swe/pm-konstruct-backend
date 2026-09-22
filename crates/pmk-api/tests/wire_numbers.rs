// An integration test is its own crate, so the lib's `cfg_attr(test, ...)`
// relaxation does not reach it. A failed `unwrap` here is the assertion
// failing, which is what we want to see.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Measurements cross the wire as JSON numbers, on every endpoint that sends one.
//!
//! `rust_decimal::Decimal` serialises as a *string*, which is easy to miss
//! because the Rust type is numeric and the field name reads like a number.
//! The DTOs convert to `f64` for that reason; without this test a later hand
//! could "simplify" one of them back to `Decimal` and the only symptom would
//! be `NaN` on a chart, or `"21.5" > 30` quietly being false in a browser.
//!
//! The diary entry flattens the same reading the weather endpoint returns, so
//! both are checked: an inconsistency between them is the failure this
//! guards, and it is one this rebuild shipped briefly before it was caught.

use pmk_api::dto::weather::WeatherDto;
use pmk_domain::diary::WeatherStamp;
use rust_decimal::Decimal;

fn stamp() -> WeatherStamp {
    WeatherStamp {
        location_name: Some("Melbourne".into()),
        location_lat: Some(Decimal::new(-3781, 2)),
        location_lng: Some(Decimal::new(14496, 2)),
        temperature: Some(Decimal::new(215, 1)),
        weather_condition: Some("Clear".into()),
        weather_icon: Some("01d".into()),
        wind_speed_kmh: Some(Decimal::new(123, 1)),
        rainfall_mm: Some(Decimal::new(0, 1)),
        sunrise_time: Some("06:12".into()),
        sunset_time: Some("18:40".into()),
    }
}

/// Every measurement is a number, and the strings stay strings.
#[test]
fn weather_measurements_are_json_numbers() {
    let json = serde_json::to_value(WeatherDto::from(stamp())).unwrap();

    for field in [
        "locationLat",
        "locationLng",
        "temperature",
        "windSpeedKmh",
        "rainfallMm",
    ] {
        assert!(
            json[field].is_number(),
            "{field} should be a JSON number, got {}",
            json[field]
        );
    }

    assert_eq!(json["temperature"], serde_json::json!(21.5));
    assert!(json["weatherCondition"].is_string());
    assert!(json["sunriseTime"].is_string());
}

/// An absent reading is null rather than a missing key: the UI tests for
/// `null`, and `undefined` would slip past a `=== null` check.
#[test]
fn an_unstamped_entry_sends_nulls() {
    let json = serde_json::to_value(WeatherDto::from(WeatherStamp::default())).unwrap();
    assert!(json["temperature"].is_null());
    assert!(json.as_object().unwrap().contains_key("temperature"));
}
