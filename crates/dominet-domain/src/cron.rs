// SPDX-License-Identifier: AGPL-3.0-or-later
//! Expressions cron a cinq champs (minute heure jour-du-mois mois jour-de-semaine), avec la syntaxe de les evenements solaires de reference
//! qu'utilisait l'implementation de reference : `*`, `n`, `a-b`, `a-b/s`, `*/s`, listes separees par des virgules. Le jour de semaine va de
//! **0 (dimanche) a 6 (samedi)** ; les noms `sun` a `sat` sont admis. Une plage a l'envers (`22-3`) fait le tour du champ.
//!
//! Ce module analyse et valide ; le calcul du prochain passage (fuseau, changement d'heure) est celui du planificateur.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use crate::error::Problem;

/// Valeurs admises d'un champ, triees et sans doublon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSpec {
    pub minutes: Vec<u8>,
    pub hours: Vec<u8>,
    pub days_of_month: Vec<u8>,
    pub months: Vec<u8>,
    /// 0 = dimanche.
    pub days_of_week: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CronError {
    #[error("il faut exactement cinq champs")]
    FieldCount,
    #[error("champ {field} invalide")]
    Field { field: &'static str },
}

impl CronError {
    /// Motif de formulaire correspondant.
    #[must_use]
    pub fn problem(self) -> Problem {
        match self {
            Self::FieldCount | Self::Field { .. } => Problem::Invalid,
        }
    }
}

const NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

fn number(token: &str, min: u8, max: u8, weekday: bool) -> Option<u8> {
    let value = if let Ok(n) = token.parse::<u8>() {
        n
    } else if weekday {
        let lower = token.to_ascii_lowercase();
        u8::try_from(NAMES.iter().position(|n| *n == lower)?).ok()?
    } else {
        return None;
    };
    (min..=max).contains(&value).then_some(value)
}

fn field(
    text: &str,
    name: &'static str,
    min: u8,
    max: u8,
    weekday: bool,
) -> Result<Vec<u8>, CronError> {
    let bad = CronError::Field { field: name };
    let mut out: Vec<u8> = vec![];
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u8>().ok().filter(|s| *s > 0).ok_or(bad)?),
            None => (part, 1),
        };
        let values: Vec<u8> = if range == "*" {
            (min..=max).collect()
        } else if let Some((a, b)) = range.split_once('-') {
            let (from, to) = (
                number(a, min, max, weekday).ok_or(bad)?,
                number(b, min, max, weekday).ok_or(bad)?,
            );
            if to < from {
                // Plage a l'envers : elle fait le tour du champ (comme un cron classique).
                (from..=max).chain(min..=to).collect()
            } else {
                (from..=to).collect()
            }
        } else if step == 1 {
            vec![number(range, min, max, weekday).ok_or(bad)?]
        } else {
            // `n/s` : de n jusqu'a la fin du champ, de s en s.
            (number(range, min, max, weekday).ok_or(bad)?..=max).collect()
        };
        out.extend(values.into_iter().step_by(usize::from(step)));
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() { Err(bad) } else { Ok(out) }
}

impl CronSpec {
    /// # Errors
    /// [`CronError`] : nombre de champs, ou champ hors plage / mal forme.
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        let [minute, hour, dom, month, dow] = fields.as_slice() else {
            return Err(CronError::FieldCount);
        };
        Ok(Self {
            minutes: field(minute, "minute", 0, 59, false)?,
            hours: field(hour, "hour", 0, 23, false)?,
            days_of_month: field(dom, "day of month", 1, 31, false)?,
            months: field(month, "month", 1, 12, false)?,
            days_of_week: field(dow, "day of week", 0, 6, true)?,
        })
    }

    /// Plus petit instant **strictement posterieur** a `after` (heure locale « murale ») qui correspond a l'expression, a la
    /// minute pile. Comme un cron classique, le jour doit correspondre au jour du mois **et** au jour de la semaine. `None` si rien ne
    /// correspond dans les huit ans qui suivent (expression impossible, par exemple le 31 avril).
    #[must_use]
    pub fn next_after(&self, after: NaiveDateTime) -> Option<NaiveDateTime> {
        // Minute pile suivante.
        let start = after.with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
        let mut date = start.date();
        let mut first_day = true;
        for _ in 0..(366 * 8) {
            let weekday = u8::try_from(date.weekday().num_days_from_sunday()).ok()?;
            let day_ok = self.months.contains(&u8::try_from(date.month()).ok()?)
                && self.days_of_month.contains(&u8::try_from(date.day()).ok()?)
                && self.days_of_week.contains(&weekday);
            if day_ok {
                let (from_h, from_m) = if first_day {
                    (start.hour(), start.minute())
                } else {
                    (0, 0)
                };
                for &h in &self.hours {
                    for &m in &self.minutes {
                        if (u32::from(h), u32::from(m)) >= (from_h, from_m) {
                            return Some(NaiveDateTime::new(
                                date,
                                NaiveTime::from_hms_opt(u32::from(h), u32::from(m), 0)?,
                            ));
                        }
                    }
                }
            }
            first_day = false;
            date = NaiveDate::succ_opt(&date)?;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, sec: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, sec)
            .unwrap()
    }

    #[test]
    fn the_next_occurrence_is_strictly_after_and_on_the_minute() {
        let c = CronSpec::parse("5 0 * * *").unwrap();
        assert_eq!(
            c.next_after(at(2026, 11, 2, 12, 0, 0)),
            Some(at(2026, 11, 3, 0, 5, 0))
        );
        assert_eq!(
            c.next_after(at(2026, 11, 3, 0, 4, 59)),
            Some(at(2026, 11, 3, 0, 5, 0))
        );
        assert_eq!(
            c.next_after(at(2026, 11, 3, 0, 5, 0)),
            Some(at(2026, 11, 4, 0, 5, 0))
        );
        assert_eq!(
            c.next_after(at(2026, 11, 3, 0, 5, 30)),
            Some(at(2026, 11, 4, 0, 5, 0))
        );
        // Jour du mois ET jour de la semaine (cron classique) : le premier lundi qui est un 1er.
        let c = CronSpec::parse("0 7 1 * mon").unwrap();
        assert_eq!(
            c.next_after(at(2026, 1, 15, 0, 0, 0)),
            Some(at(2026, 6, 1, 7, 0, 0))
        );
        // Le 29 fevrier, et une expression impossible.
        let c = CronSpec::parse("0 0 29 2 *").unwrap();
        assert_eq!(
            c.next_after(at(2028, 3, 1, 0, 0, 0)),
            Some(at(2032, 2, 29, 0, 0, 0))
        );
        assert_eq!(
            CronSpec::parse("0 0 31 4 *")
                .unwrap()
                .next_after(at(2026, 1, 1, 0, 0, 0)),
            None
        );
    }

    #[test]
    fn the_usual_expressions_expand_like_the_reference() {
        let c = CronSpec::parse("*/15 6-8 * * mon-fri").unwrap();
        assert_eq!(c.minutes, [0, 15, 30, 45]);
        assert_eq!(c.hours, [6, 7, 8]);
        assert_eq!(c.days_of_month.len(), 31);
        assert_eq!(c.months.len(), 12);
        assert_eq!(c.days_of_week, [1, 2, 3, 4, 5]);
        let c = CronSpec::parse("5 0 1,15 * 0").unwrap();
        assert_eq!(
            (c.minutes, c.hours, c.days_of_month, c.days_of_week),
            (vec![5], vec![0], vec![1, 15], vec![0])
        );
        assert_eq!(
            CronSpec::parse("0 22-2 * * *").unwrap().hours,
            [0, 1, 2, 22, 23],
            "plage a l'envers"
        );
        assert_eq!(
            CronSpec::parse("10-30/10 * * * *").unwrap().minutes,
            [10, 20, 30]
        );
        assert_eq!(
            CronSpec::parse("0 0 * * 6,SUN").unwrap().days_of_week,
            [0, 6]
        );
    }

    #[test]
    fn malformed_expressions_are_refused() {
        for bad in [
            "",
            "* * * *",
            "* * * * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * 32 * *",
            "* * * 13 *",
            "* * * * 7",
            "*/0 * * * *",
            "a * * * *",
            "1- * * * *",
            ",1 * * * *",
            "* * * * funday",
        ] {
            assert!(CronSpec::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(CronSpec::parse("1 2 3").unwrap_err(), CronError::FieldCount);
        assert_eq!(
            CronSpec::parse("61 * * * *").unwrap_err(),
            CronError::Field { field: "minute" }
        );
    }
}
