// SPDX-License-Identifier: AGPL-3.0-or-later
//! Prochain passage d'un horaire (R4 lot 4, ADR 0033), fuseau et changements d'heure compris.
//!
//! Un cron s'ecrit en heure murale du fuseau du travail. **Comme un cron classique**, une heure qui
//! n'existe pas (le saut de printemps, 02:30) est lue avec le decalage d'avant le saut (donc 03:30 d'ete), et une heure qui
//! existe deux fois (le retour d'automne) est prise a sa premiere occurrence ; la seconde occurrence de l'heure repetee ne
//! declenche rien. Verifie sur 119 cas contre reference.

use chrono::{DateTime, Duration, NaiveDateTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use dominet_domain::cron::CronSpec;
use dominet_domain::schedule::JobSchedule;
use dominet_domain::solar::next_event;

/// Heure murale -> instant UTC, avec la regle de la reference (`fold = 0`).
pub(crate) fn to_utc(tz: Tz, local: NaiveDateTime) -> Option<DateTime<Utc>> {
    match tz.from_local_datetime(&local) {
        chrono::LocalResult::Single(t) => Some(t.with_timezone(&Utc)),
        chrono::LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
        chrono::LocalResult::None => {
            // Saut de printemps : decalage en vigueur juste avant le saut.
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

/// Premier passage d'une expression cron **strictement apres** `after`.
#[must_use]
pub fn cron_next(spec: &CronSpec, tz: Tz, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let mut local = after.with_timezone(&tz).naive_local();
    // Quelques tours suffisent : seuls les cas de l'heure repetee (retour d'automne) en demandent plus d'un.
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

/// Prochain passage d'un horaire **strictement apres** `after` ; `None` si l'horaire n'a pas de passage proche (cron
/// invalide ou impossible, soleil qui ne se leve ni ne se couche dans les jours qui viennent).
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
