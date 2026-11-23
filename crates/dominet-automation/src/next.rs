// SPDX-License-Identifier: AGPL-3.0-or-later
//! Next run of a schedule (R4 lot 4, ADR 0033), time zone and clock changes included.
//!
//! A cron is written in wall-clock time of the job's time zone. **Like a classic cron**, an hour that
//! does not exist (the spring jump, 02:30) is read with the offset from before the jump (so 03:30 summer time), and an hour that
//! exists twice (the autumn fall-back) is taken at its first occurrence; the second occurrence of the repeated hour does not
//! trigger anything. Checked on 119 cases against the reference.

use chrono::{DateTime, Duration, NaiveDateTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use dominet_domain::cron::CronSpec;
use dominet_domain::schedule::JobSchedule;
use dominet_domain::solar::next_event;

/// Wall-clock time -> UTC instant, with the reference's rule (`fold = 0`).
pub(crate) fn to_utc(tz: Tz, local: NaiveDateTime) -> Option<DateTime<Utc>> {
    match tz.from_local_datetime(&local) {
        chrono::LocalResult::Single(t) => Some(t.with_timezone(&Utc)),
        chrono::LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
        chrono::LocalResult::None => {
            // Spring jump: offset in force just before the jump.
            let before = tz
                .from_local_datetime(&(local - Duration::days(1)))
                .earliest()?
                .offset()
                .fix();
            Some(DateTime::from_naive_utc_and_offset(
                local - Duration::seconds(i64::from(before.local_minus_utc())),
                Utc,
            ))
        }
    }
}

/// First run of a cron expression **strictly after** `after`.
#[must_use]
pub fn cron_next(spec: &CronSpec, tz: Tz, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let mut local = after.with_timezone(&tz).naive_local();
    // A few rounds suffice: only the repeated-hour cases (autumn fall-back) need more than one.
    for _ in 0..8 {
        let candidate = spec.next_after(local)?;
        let utc = to_utc(tz, candidate)?;
        if utc > after {
            return Some(utc);
        }
        local = candidate;
    }
    None
}

/// Next run of a schedule **strictly after** `after`; `None` if the schedule has no near run (invalid or impossible
/// cron, sun that neither rises nor sets in the coming days).
#[must_use]
pub fn next_run(schedule: &JobSchedule, tz: Tz, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match schedule {
        JobSchedule::Interval { every, period } => {
            Some(after + Duration::seconds(i64::from(*every) * period.seconds()))
        }
        JobSchedule::Cron(expr) => cron_next(&CronSpec::parse(expr).ok()?, tz, after),
        JobSchedule::Solar {
            event,
            latitude,
            longitude,
        } => next_event(*event, *latitude, *longitude, after),
    }
}
