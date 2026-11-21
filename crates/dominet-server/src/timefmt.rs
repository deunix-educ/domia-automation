// SPDX-License-Identifier: AGPL-3.0-or-later
//! Instants shown to people: always in the server's time zone (`TZ`, default `Europe/Paris`), never in UTC.
//!
//! The zone is read once from the environment, with the same rule as `Config::timezone`. The REST API keeps UTC.

use std::sync::OnceLock;

use chrono::{DateTime, SecondsFormat, Utc};
use chrono_tz::Tz;

fn zone() -> Tz {
    static ZONE: OnceLock<Tz> = OnceLock::new();
    *ZONE.get_or_init(|| {
        std::env::var("TZ")
            .ok()
            .and_then(|name| name.parse().ok())
            .unwrap_or(chrono_tz::Europe::Paris)
    })
}

/// RFC 3339 with the local offset, to the second: `2026-11-13T09:12:05+01:00`.
#[must_use]
pub fn rfc3339(at: DateTime<Utc>) -> String {
    rfc3339_in(at, zone())
}

/// Plain local text, to the second: `2026-11-13 09:12:05`.
#[must_use]
pub fn text(at: DateTime<Utc>) -> String {
    text_in(at, zone())
}

#[must_use]
pub fn rfc3339_in(at: DateTime<Utc>, tz: Tz) -> String {
    at.with_timezone(&tz)
        .to_rfc3339_opts(SecondsFormat::Secs, false)
}

#[must_use]
pub fn text_in(at: DateTime<Utc>, tz: Tz) -> String {
    at.with_timezone(&tz)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn instants_are_written_in_local_time_summer_and_winter() {
        let paris = chrono_tz::Europe::Paris;
        let winter = Utc.with_ymd_and_hms(2026, 11, 13, 8, 12, 5).unwrap();
        let summer = Utc.with_ymd_and_hms(2026, 7, 1, 22, 30, 0).unwrap();
        assert_eq!(rfc3339_in(winter, paris), "2026-11-13T09:12:05+01:00");
        assert_eq!(text_in(winter, paris), "2026-11-13 09:12:05");
        // Summer time, and the local day is already the next one.
        assert_eq!(rfc3339_in(summer, paris), "2026-07-02T00:30:00+02:00");
        assert_eq!(text_in(summer, paris), "2026-07-02 00:30:00");
        assert_eq!(text_in(winter, chrono_tz::UTC), "2026-11-13 08:12:05");
    }
}
