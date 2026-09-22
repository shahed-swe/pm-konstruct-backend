//! Time as a dependency.
//!
//! Three domain rules depend on "today" (delay computation R1, ETO diary dates
//! R7, scheduler day boundaries R12), and the legacy code read the system clock
//! in server-local time while parsing dates as UTC. That is only self-
//! consistent on a UTC host.
//!
//! Every date boundary in the new system goes through a `Clock` carrying an
//! explicit timezone, so behaviour does not change when the server moves.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;

pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now_utc(&self) -> DateTime<Utc>;

    /// The timezone all date boundaries are computed in. Configured per
    /// company; defaults to Australia/Melbourne.
    fn timezone(&self) -> Tz;

    /// "Today" in [`Clock::timezone`]. Never the host's local date.
    fn today(&self) -> NaiveDate {
        self.now_utc().with_timezone(&self.timezone()).date_naive()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    tz: Tz,
}

impl SystemClock {
    #[must_use]
    pub const fn new(tz: Tz) -> Self {
        Self { tz }
    }
}

impl Clock for SystemClock {
    fn now_utc(&self) -> DateTime<Utc> {
        Utc::now()
    }
    fn timezone(&self) -> Tz {
        self.tz
    }
}

/// Frozen clock for tests, so date-dependent rules are deterministic.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    at: DateTime<Utc>,
    tz: Tz,
}

impl FixedClock {
    #[must_use]
    pub const fn new(at: DateTime<Utc>, tz: Tz) -> Self {
        Self { at, tz }
    }
}

impl Clock for FixedClock {
    fn now_utc(&self) -> DateTime<Utc> {
        self.at
    }
    fn timezone(&self) -> Tz {
        self.tz
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn today_uses_the_configured_timezone_not_the_host() {
        // 2026-09-21T22:00Z is already the 22nd in Melbourne (+10).
        let at = Utc.with_ymd_and_hms(2026, 9, 21, 22, 0, 0).unwrap();
        let melbourne = FixedClock::new(at, chrono_tz::Australia::Melbourne);
        let utc = FixedClock::new(at, chrono_tz::UTC);
        assert_eq!(
            melbourne.today(),
            NaiveDate::from_ymd_opt(2026, 9, 22).unwrap()
        );
        assert_eq!(utc.today(), NaiveDate::from_ymd_opt(2026, 9, 21).unwrap());
    }
}
