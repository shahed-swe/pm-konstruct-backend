//! Weather shaping: the parts that would be wrong in ways an API call cannot
//! tell you about.

use crate::error::{DomainError, DomainResult};

/// A coordinate pair on Earth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinates {
    pub lat: f64,
    pub lon: f64,
}

impl Coordinates {
    pub fn parse(lat: &str, lon: &str) -> DomainResult<Self> {
        let lat = parse_coord("lat", lat)?;
        let lon = parse_coord("lon", lon)?;
        Self::new(lat, lon)
    }

    pub fn new(lat: f64, lon: f64) -> DomainResult<Self> {
        // NaN fails every comparison, so it would slip through a plain range
        // check and reach the provider as `lat=NaN`.
        if lat.is_nan() || lon.is_nan() || !(-90.0..=90.0).contains(&lat) {
            return Err(DomainError::invalid(
                "lat",
                "must be a number between -90 and 90",
            ));
        }
        if !(-180.0..=180.0).contains(&lon) {
            return Err(DomainError::invalid(
                "lon",
                "must be a number between -180 and 180",
            ));
        }
        Ok(Self { lat, lon })
    }

    /// Rounded to two decimal places, about a kilometre.
    ///
    /// This is the cache key as well as what is sent upstream: two people on
    /// the same site share a lookup, and the stored coordinate is not precise
    /// enough to place someone at a particular house.
    #[must_use]
    pub fn rounded(self) -> Self {
        Self {
            lat: round_2dp(self.lat),
            lon: round_2dp(self.lon),
        }
    }

    /// The cache key for a rounded pair.
    #[must_use]
    pub fn cache_key(self) -> String {
        let r = self.rounded();
        format!("{},{}", r.lat, r.lon)
    }
}

fn parse_coord(field: &'static str, raw: &str) -> DomainResult<f64> {
    let raw = raw.trim();
    // Decimal degrees only: the legacy regex accepted nothing else, and
    // `f64::from_str` would otherwise take "inf", "NaN" and "1e400".
    let valid = !raw.is_empty()
        && raw
            .strip_prefix('-')
            .unwrap_or(raw)
            .split('.')
            .enumerate()
            .all(|(i, part)| i < 2 && !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
    if !valid {
        return Err(DomainError::invalid(field, "must be a decimal number"));
    }
    raw.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| DomainError::invalid(field, "must be a decimal number"))
}

/// Rounds half away from zero, to two places.
#[must_use]
pub fn round_2dp(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Rounds half away from zero, to one place, as the readings are printed.
#[must_use]
pub fn round_1dp(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// The icon shown beside a condition.
///
/// The keys are OpenWeatherMap's `weather[0].main` values. Anything
/// unrecognised gets the generic sun-behind-cloud rather than nothing, so a
/// new condition name does not leave a blank in the UI.
#[must_use]
pub fn condition_icon(condition: &str) -> &'static str {
    match condition {
        "Clear" => "☀️",
        "Clouds" => "☁️",
        "Rain" => "🌧",
        "Drizzle" => "🌦",
        "Thunderstorm" => "⛈",
        "Snow" => "❄️",
        "Mist" | "Fog" | "Haze" | "Smoke" | "Dust" | "Sand" | "Ash" => "🌫",
        "Squall" => "🌬",
        "Tornado" => "🌪",
        _ => "🌤",
    }
}

/// Metres per second to kilometres per hour.
#[must_use]
pub fn ms_to_kmh(ms: f64) -> f64 {
    round_1dp(ms * 3.6)
}

/// A unix timestamp as `HH:MM` at the observation's own offset.
///
/// The offset is the *location's*, not the server's or the viewer's: a sunrise
/// time is only meaningful where the sun rose.
#[must_use]
pub fn local_hhmm(unix_seconds: i64, offset_seconds: i64) -> Option<String> {
    let local = chrono::DateTime::from_timestamp(unix_seconds + offset_seconds, 0)?;
    Some(local.format("%H:%M").to_string())
}

/// Capitalises the first character, leaving the rest alone.
///
/// OpenWeatherMap returns descriptions in lower case; the UI shows them as
/// sentences.
#[must_use]
pub fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

#[cfg(test)]
// Exact comparison is the point in this module: rounding and unit conversion
// have to land on precisely the value that gets displayed and cached, and an
// epsilon would hide a rule that had drifted.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn valid_coordinates_are_accepted() {
        assert!(Coordinates::parse("-37.81", "144.96").is_ok());
        assert!(Coordinates::parse("0", "0").is_ok());
        assert!(Coordinates::parse("90", "180").is_ok());
        assert!(Coordinates::parse("-90", "-180").is_ok());
    }

    #[test]
    fn out_of_range_coordinates_are_rejected() {
        assert!(Coordinates::parse("90.01", "0").is_err());
        assert!(Coordinates::parse("-90.01", "0").is_err());
        assert!(Coordinates::parse("0", "180.01").is_err());
        assert!(Coordinates::parse("0", "-180.01").is_err());
    }

    #[test]
    fn non_numeric_coordinates_are_rejected() {
        for bad in ["", "  ", "abc", "1.2.3", "1,2", "--1", "1.", ".5", "+1"] {
            assert!(
                Coordinates::parse(bad, "0").is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn the_special_float_spellings_are_rejected() {
        // `f64::from_str` accepts all of these; the API upstream would not.
        for bad in ["NaN", "inf", "-inf", "Infinity", "1e400", "1e2"] {
            assert!(
                Coordinates::parse(bad, "0").is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn nan_cannot_reach_the_provider() {
        // NaN fails every comparison, so a plain range check would pass it.
        assert!(Coordinates::new(f64::NAN, 0.0).is_err());
        assert!(Coordinates::new(0.0, f64::NAN).is_err());
    }

    #[test]
    fn coordinates_round_to_about_a_kilometre() {
        let c = Coordinates::new(-37.814218, 144.963161).unwrap().rounded();
        assert_eq!(c.lat, -37.81);
        assert_eq!(c.lon, 144.96);
    }

    #[test]
    fn nearby_requests_share_a_cache_key() {
        // Two people on the same site should cost one lookup.
        let a = Coordinates::new(-37.8142, 144.9631).unwrap();
        let b = Coordinates::new(-37.8149, 144.9634).unwrap();
        assert_eq!(a.cache_key(), b.cache_key());
    }

    #[test]
    fn distant_requests_do_not() {
        let melbourne = Coordinates::new(-37.81, 144.96).unwrap();
        let sydney = Coordinates::new(-33.87, 151.21).unwrap();
        assert_ne!(melbourne.cache_key(), sydney.cache_key());
    }

    #[test]
    fn the_known_conditions_have_their_own_icon() {
        assert_eq!(condition_icon("Clear"), "☀️");
        assert_eq!(condition_icon("Rain"), "🌧");
        assert_eq!(condition_icon("Thunderstorm"), "⛈");
        assert_eq!(condition_icon("Tornado"), "🌪");
        // The obscuring conditions share one.
        for c in ["Mist", "Fog", "Haze", "Smoke", "Dust", "Sand", "Ash"] {
            assert_eq!(condition_icon(c), "🌫", "{c}");
        }
    }

    #[test]
    fn an_unknown_condition_still_gets_an_icon() {
        // A new condition name must not leave a blank in the UI.
        assert_eq!(condition_icon("Meteors"), "🌤");
        assert_eq!(condition_icon(""), "🌤");
    }

    #[test]
    fn wind_converts_from_metres_per_second() {
        assert_eq!(ms_to_kmh(0.0), 0.0);
        assert_eq!(ms_to_kmh(10.0), 36.0);
        // 5.5 m/s = 19.8 km/h, to one place.
        assert_eq!(ms_to_kmh(5.5), 19.8);
    }

    #[test]
    fn times_are_rendered_at_the_locations_own_offset() {
        // 2026-03-04 00:00 UTC, with Melbourne's +11 in daylight saving.
        let midnight_utc = 1_772_582_400;
        assert_eq!(local_hhmm(midnight_utc, 0).as_deref(), Some("00:00"));
        assert_eq!(
            local_hhmm(midnight_utc, 11 * 3600).as_deref(),
            Some("11:00")
        );
        // And a negative offset wraps back through the previous day.
        assert_eq!(
            local_hhmm(midnight_utc, -5 * 3600).as_deref(),
            Some("19:00")
        );
    }

    #[test]
    fn an_impossible_timestamp_yields_no_time_rather_than_a_wrong_one() {
        assert_eq!(local_hhmm(i64::MAX, 0), None);
    }

    #[test]
    fn descriptions_are_capitalised_for_display() {
        assert_eq!(capitalise("light rain"), "Light rain");
        assert_eq!(capitalise("Clear"), "Clear");
        assert_eq!(capitalise(""), "");
        // Multi-byte first character.
        assert_eq!(capitalise("ésumé"), "Ésumé");
    }
}
