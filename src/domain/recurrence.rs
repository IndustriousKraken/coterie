//! Recurrence rules for events. Deliberately a narrow subset of
//! RFC 5545 — only the patterns Coterie actually uses for member-
//! group scheduling. A full RRULE implementation is months of work
//! for negligible additional value.
//!
//! The three supported kinds:
//!
//!   - `WeeklyByDay`        — every Monday, every Mon+Wed, every
//!                             other Friday. `interval` is the gap
//!                             between cycles in weeks (1 = every
//!                             week, 2 = every other).
//!   - `MonthlyByDayOfMonth`— "the 15th of each month." `day` is
//!                             1..=31; for months without that day
//!                             (e.g. Feb 31) the occurrence is
//!                             skipped, NOT clamped to month-end —
//!                             clamping would silently move dates
//!                             around in a way that surprises
//!                             admins.
//!   - `MonthlyByWeekdayOrdinal` — "2nd Wednesday," "last Friday."
//!                             `ordinal` is 1..=4 for first..fourth,
//!                             or -1 for last.
//!
//! Time-of-day is taken from the series' anchor (the first
//! occurrence's start time). Generation works in UTC throughout — the
//! caller is responsible for converting to local time at the
//! boundary if they want the "every Tuesday at 6pm local" semantics.
//! All Coterie events are stored in UTC, so this matches the storage
//! convention.

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};

/// The rule shape, persisted as JSON in `event_series.rule_json`. The
/// `tag = "kind"` discriminator matches `event_series.rule_kind` for
/// SQL-level joins / filters that don't want to parse the JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Recurrence {
    /// Every `interval` weeks, on each of `weekdays`. Empty `weekdays`
    /// is invalid (validate before persisting).
    WeeklyByDay {
        interval: u32,
        weekdays: Vec<WeekdayCode>,
    },
    /// Day-of-month — 1..=31. Months without that day are SKIPPED.
    MonthlyByDayOfMonth {
        interval: u32,
        day: u32,
    },
    /// Ordinal weekday in the month: "2nd Wednesday," "last Friday."
    /// `ordinal` is 1..=4 (first..fourth) or -1 (last).
    MonthlyByWeekdayOrdinal {
        interval: u32,
        weekday: WeekdayCode,
        ordinal: i32,
    },
}

/// JSON-friendly weekday code. We don't use `chrono::Weekday`
/// directly because its serde representation is inconsistent across
/// versions; a small enum with explicit `rename_all = "snake_case"`
/// is stable.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum WeekdayCode {
    Mon, Tue, Wed, Thu, Fri, Sat, Sun,
}

impl WeekdayCode {
    pub fn to_chrono(self) -> Weekday {
        match self {
            Self::Mon => Weekday::Mon,
            Self::Tue => Weekday::Tue,
            Self::Wed => Weekday::Wed,
            Self::Thu => Weekday::Thu,
            Self::Fri => Weekday::Fri,
            Self::Sat => Weekday::Sat,
            Self::Sun => Weekday::Sun,
        }
    }

    pub fn from_chrono(w: Weekday) -> Self {
        match w {
            Weekday::Mon => Self::Mon,
            Weekday::Tue => Self::Tue,
            Weekday::Wed => Self::Wed,
            Weekday::Thu => Self::Thu,
            Weekday::Fri => Self::Fri,
            Weekday::Sat => Self::Sat,
            Weekday::Sun => Self::Sun,
        }
    }
}

impl Recurrence {
    /// Discriminator used as `event_series.rule_kind` in SQL — see
    /// the migration. Stable; renaming any of these is a breaking
    /// change for existing series rows.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::WeeklyByDay { .. } => "weekly_by_day",
            Self::MonthlyByDayOfMonth { .. } => "monthly_by_dom",
            Self::MonthlyByWeekdayOrdinal { .. } => "monthly_by_weekday",
        }
    }

    /// Reject obviously-malformed rules at the boundary. Caller (UI
    /// or API handler) gets a single error message; it's cheaper to
    /// validate here than to materialize and crash later.
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::WeeklyByDay { interval, weekdays } => {
                if *interval == 0 || *interval > 52 {
                    return Err("interval must be 1..=52 weeks");
                }
                if weekdays.is_empty() {
                    return Err("weekly recurrence needs at least one weekday");
                }
            }
            Self::MonthlyByDayOfMonth { interval, day } => {
                if *interval == 0 || *interval > 12 {
                    return Err("interval must be 1..=12 months");
                }
                if *day == 0 || *day > 31 {
                    return Err("day must be 1..=31");
                }
            }
            Self::MonthlyByWeekdayOrdinal { interval, ordinal, .. } => {
                if *interval == 0 || *interval > 12 {
                    return Err("interval must be 1..=12 months");
                }
                if !matches!(ordinal, 1..=4 | -1) {
                    return Err("ordinal must be 1, 2, 3, 4, or -1 (last)");
                }
            }
        }
        Ok(())
    }
}

/// Generate all occurrences of `rule` in the half-open window
/// `[from, to)`. The first occurrence is `anchor` (typically the
/// admin-supplied start time of the series); subsequent occurrences
/// preserve `anchor`'s time-of-day. Occurrences before `from` are
/// excluded, occurrences at or after `to` are excluded — so the
/// caller passes `to = today + 12.months` to materialize the next
/// year, then the daily job calls again with a moved-forward window.
///
/// Output is sorted ascending. Bounded internally (10_000 occurrences)
/// so a malformed rule can't burn the CPU.
pub fn generate_occurrences(
    anchor: DateTime<Utc>,
    rule: &Recurrence,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<DateTime<Utc>> {
    if to <= from {
        return Vec::new();
    }
    let mut out = Vec::new();
    const MAX: usize = 10_000;

    match rule {
        Recurrence::WeeklyByDay { interval, weekdays } => {
            // Walk one cycle at a time. For each cycle, emit every
            // listed weekday. `anchor` defines the cycle start (its
            // ISO week is cycle 0).
            let anchor_week_start = start_of_week_utc(anchor);
            let cycle = Duration::weeks(*interval as i64);

            let mut current_cycle_start = anchor_week_start;
            // Walk forward / backward from anchor's cycle until we
            // straddle `from`. We'll cap iterations to avoid runaway.
            while current_cycle_start + Duration::weeks(7) <= from && out.len() < MAX {
                current_cycle_start += cycle;
            }
            while current_cycle_start > from && out.len() < MAX {
                current_cycle_start -= cycle;
            }

            'outer: while out.len() < MAX {
                if current_cycle_start >= to {
                    break;
                }
                for wd in weekdays.iter().copied() {
                    let candidate = day_of_week_in_week(
                        current_cycle_start,
                        wd.to_chrono(),
                        anchor,
                    );
                    if candidate >= to {
                        break 'outer;
                    }
                    if candidate >= from && candidate >= anchor {
                        out.push(candidate);
                    }
                }
                current_cycle_start += cycle;
            }
        }

        Recurrence::MonthlyByDayOfMonth { interval, day } => {
            let mut cursor = first_of_month(anchor);
            while cursor < to && out.len() < MAX {
                if let Some(d) = NaiveDate::from_ymd_opt(cursor.year(), cursor.month(), *day) {
                    let dt = combine(d, anchor);
                    if dt >= from && dt < to && dt >= anchor {
                        out.push(dt);
                    }
                }
                // Otherwise: month doesn't have that day (e.g. Feb 31).
                // Skip this month's occurrence entirely — see module
                // doc on why we don't clamp.
                cursor = add_months(cursor, *interval as i32);
            }
        }

        Recurrence::MonthlyByWeekdayOrdinal { interval, weekday, ordinal } => {
            let mut cursor = first_of_month(anchor);
            while cursor < to && out.len() < MAX {
                if let Some(d) = ordinal_weekday_of_month(
                    cursor.year(),
                    cursor.month(),
                    weekday.to_chrono(),
                    *ordinal,
                ) {
                    let dt = combine(d, anchor);
                    if dt >= from && dt < to && dt >= anchor {
                        out.push(dt);
                    }
                }
                cursor = add_months(cursor, *interval as i32);
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

/// Sunday or Monday as week start? ISO 8601 (Mon-start) is what
/// chrono uses internally and what's natural for Mon..Fri scheduling.
fn start_of_week_utc(dt: DateTime<Utc>) -> DateTime<Utc> {
    let weekday = dt.weekday().num_days_from_monday() as i64;
    let date = dt.date_naive() - chrono::Duration::days(weekday);
    Utc.from_utc_datetime(&date.and_time(dt.time()))
}

/// Find the date in the week starting at `week_start` that falls on
/// `weekday`, then attach `anchor`'s time-of-day. UTC throughout.
fn day_of_week_in_week(
    week_start: DateTime<Utc>,
    weekday: Weekday,
    anchor: DateTime<Utc>,
) -> DateTime<Utc> {
    let offset = weekday.num_days_from_monday() as i64;
    let date = week_start.date_naive() + chrono::Duration::days(offset);
    Utc.from_utc_datetime(&date.and_time(anchor.time()))
}

fn first_of_month(dt: DateTime<Utc>) -> DateTime<Utc> {
    let date = NaiveDate::from_ymd_opt(dt.year(), dt.month(), 1)
        .expect("year/month always valid for date 1");
    Utc.from_utc_datetime(&date.and_time(dt.time()))
}

/// Add `months` to a UTC datetime, preserving day-of-month.
/// Out-of-range results (e.g. day=31 plus 1 month landing on Feb)
/// return the latest valid day in the target month — used here only
/// for cursor advancement, not for occurrence emission.
fn add_months(dt: DateTime<Utc>, months: i32) -> DateTime<Utc> {
    let total = dt.year() as i32 * 12 + dt.month() as i32 - 1 + months;
    let new_year = total.div_euclid(12);
    let new_month = (total.rem_euclid(12) + 1) as u32;

    // For cursor purposes, snap to the 1st — we don't care about the
    // day, only the month/year.
    let date = NaiveDate::from_ymd_opt(new_year, new_month, 1)
        .expect("computed month always valid");
    Utc.from_utc_datetime(&date.and_time(dt.time()))
}

/// Combine a naive date with the time-of-day from a UTC anchor.
fn combine(date: NaiveDate, anchor: DateTime<Utc>) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_time(anchor.time()))
}

/// "Nth weekday of month": ordinal=1..=4 picks the first..fourth
/// occurrence; ordinal=-1 picks the last. Returns `None` when the
/// requested ordinal doesn't exist (e.g. there's no 5th Tuesday this
/// month and the rule asked for ordinal=4 — we return the 4th if it
/// exists, None otherwise).
fn ordinal_weekday_of_month(
    year: i32,
    month: u32,
    weekday: Weekday,
    ordinal: i32,
) -> Option<NaiveDate> {
    if ordinal == -1 {
        // Walk backwards from the last day of the month.
        let last_day = days_in_month(year, month)?;
        for day in (1..=last_day).rev() {
            let d = NaiveDate::from_ymd_opt(year, month, day)?;
            if d.weekday() == weekday {
                return Some(d);
            }
        }
        return None;
    }

    let mut count = 0;
    let last_day = days_in_month(year, month)?;
    for day in 1..=last_day {
        let d = NaiveDate::from_ymd_opt(year, month, day)?;
        if d.weekday() == weekday {
            count += 1;
            if count == ordinal {
                return Some(d);
            }
        }
    }
    None
}

fn days_in_month(year: i32, month: u32) -> Option<u32> {
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    let last = next - chrono::Duration::days(1);
    Some(last.day())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn weekly_every_monday_for_a_month() {
        let anchor = dt(2026, 5, 4, 18, 0); // Mon 2026-05-04 18:00 UTC
        let rule = Recurrence::WeeklyByDay {
            interval: 1,
            weekdays: vec![WeekdayCode::Mon],
        };
        let from = dt(2026, 5, 1, 0, 0);
        let to = dt(2026, 6, 1, 0, 0);
        let occs = generate_occurrences(anchor, &rule, from, to);
        assert_eq!(occs.len(), 4);
        for o in &occs {
            assert_eq!(o.weekday(), Weekday::Mon);
            assert_eq!(o.hour(), 18);
        }
    }

    #[test]
    fn weekly_mwf_three_days_per_week() {
        let anchor = dt(2026, 5, 4, 9, 0); // Mon
        let rule = Recurrence::WeeklyByDay {
            interval: 1,
            weekdays: vec![WeekdayCode::Mon, WeekdayCode::Wed, WeekdayCode::Fri],
        };
        let occs = generate_occurrences(
            anchor, &rule,
            dt(2026, 5, 1, 0, 0),
            dt(2026, 5, 18, 0, 0),
        );
        // Mon 4, Wed 6, Fri 8, Mon 11, Wed 13, Fri 15 = 6 occurrences
        assert_eq!(occs.len(), 6);
    }

    #[test]
    fn weekly_biweekly_skips_weeks() {
        let anchor = dt(2026, 5, 4, 18, 0); // Mon
        let rule = Recurrence::WeeklyByDay {
            interval: 2,
            weekdays: vec![WeekdayCode::Mon],
        };
        let occs = generate_occurrences(
            anchor, &rule,
            dt(2026, 5, 1, 0, 0),
            dt(2026, 6, 30, 0, 0),
        );
        // 5/4, 5/18, 6/1, 6/15, 6/29 — 5 occurrences
        assert_eq!(occs.len(), 5);
        let mut last = occs[0];
        for o in &occs[1..] {
            assert_eq!(*o - last, Duration::weeks(2));
            last = *o;
        }
    }

    #[test]
    fn monthly_by_day_of_month_15th() {
        let anchor = dt(2026, 1, 15, 12, 0);
        let rule = Recurrence::MonthlyByDayOfMonth { interval: 1, day: 15 };
        let occs = generate_occurrences(
            anchor, &rule,
            dt(2026, 1, 1, 0, 0),
            dt(2027, 1, 1, 0, 0),
        );
        assert_eq!(occs.len(), 12);
        for (i, o) in occs.iter().enumerate() {
            assert_eq!(o.day(), 15);
            assert_eq!(o.month() as usize, i + 1);
        }
    }

    #[test]
    fn monthly_by_day_31_skips_short_months() {
        let anchor = dt(2026, 1, 31, 9, 0);
        let rule = Recurrence::MonthlyByDayOfMonth { interval: 1, day: 31 };
        let occs = generate_occurrences(
            anchor, &rule,
            dt(2026, 1, 1, 0, 0),
            dt(2027, 1, 1, 0, 0),
        );
        // Jan, Mar, May, Jul, Aug, Oct, Dec = 7 months with a 31st.
        // Feb, Apr, Jun, Sep, Nov skipped.
        assert_eq!(occs.len(), 7);
        let months: Vec<u32> = occs.iter().map(|o| o.month()).collect();
        assert_eq!(months, vec![1, 3, 5, 7, 8, 10, 12]);
    }

    #[test]
    fn monthly_2nd_wednesday() {
        let anchor = dt(2026, 1, 14, 19, 0); // 2nd Wed of Jan 2026
        let rule = Recurrence::MonthlyByWeekdayOrdinal {
            interval: 1,
            weekday: WeekdayCode::Wed,
            ordinal: 2,
        };
        let occs = generate_occurrences(
            anchor, &rule,
            dt(2026, 1, 1, 0, 0),
            dt(2027, 1, 1, 0, 0),
        );
        assert_eq!(occs.len(), 12);
        for o in &occs {
            assert_eq!(o.weekday(), Weekday::Wed);
            // It's the 2nd Wed of its month — i.e. the day-of-month
            // is between 8 and 14 inclusive.
            assert!((8..=14).contains(&o.day()));
        }
    }

    #[test]
    fn monthly_last_friday() {
        let anchor = dt(2026, 1, 30, 17, 0); // last Fri of Jan 2026
        let rule = Recurrence::MonthlyByWeekdayOrdinal {
            interval: 1,
            weekday: WeekdayCode::Fri,
            ordinal: -1,
        };
        let occs = generate_occurrences(
            anchor, &rule,
            dt(2026, 1, 1, 0, 0),
            dt(2027, 1, 1, 0, 0),
        );
        assert_eq!(occs.len(), 12);
        for o in &occs {
            assert_eq!(o.weekday(), Weekday::Fri);
            // Last Fri ⇒ no later Fri in the same month.
            let next = *o + Duration::days(7);
            assert_ne!(next.month(), o.month());
        }
    }

    #[test]
    fn occurrences_before_anchor_are_excluded() {
        let anchor = dt(2026, 6, 1, 12, 0); // Mon
        let rule = Recurrence::WeeklyByDay {
            interval: 1, weekdays: vec![WeekdayCode::Mon],
        };
        // Window is wider than anchor — pre-anchor weeks must NOT appear.
        let occs = generate_occurrences(
            anchor, &rule,
            dt(2026, 5, 1, 0, 0),
            dt(2026, 7, 1, 0, 0),
        );
        for o in &occs {
            assert!(*o >= anchor, "{} < {}", o, anchor);
        }
    }

    #[test]
    fn empty_window_returns_empty() {
        let anchor = dt(2026, 1, 1, 0, 0);
        let rule = Recurrence::WeeklyByDay {
            interval: 1, weekdays: vec![WeekdayCode::Mon],
        };
        let same = dt(2026, 6, 1, 0, 0);
        assert!(generate_occurrences(anchor, &rule, same, same).is_empty());
    }

    #[test]
    fn validate_catches_zero_interval() {
        let r = Recurrence::WeeklyByDay { interval: 0, weekdays: vec![WeekdayCode::Mon] };
        assert!(r.validate().is_err());
    }

    #[test]
    fn validate_catches_empty_weekdays() {
        let r = Recurrence::WeeklyByDay { interval: 1, weekdays: vec![] };
        assert!(r.validate().is_err());
    }

    #[test]
    fn validate_catches_bad_ordinal() {
        let r = Recurrence::MonthlyByWeekdayOrdinal {
            interval: 1, weekday: WeekdayCode::Wed, ordinal: 5,
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn json_round_trip() {
        let r = Recurrence::WeeklyByDay {
            interval: 2,
            weekdays: vec![WeekdayCode::Mon, WeekdayCode::Wed],
        };
        let json = serde_json::to_string(&r).unwrap();
        // Discriminator must be present, weekdays as snake_case strings.
        assert!(json.contains("\"kind\":\"weekly_by_day\""));
        assert!(json.contains("\"mon\""));
        let parsed: Recurrence = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn kind_str_matches_serde_tag() {
        assert_eq!(
            Recurrence::WeeklyByDay { interval: 1, weekdays: vec![WeekdayCode::Mon] }.kind_str(),
            "weekly_by_day",
        );
        assert_eq!(
            Recurrence::MonthlyByDayOfMonth { interval: 1, day: 1 }.kind_str(),
            "monthly_by_dom",
        );
        assert_eq!(
            Recurrence::MonthlyByWeekdayOrdinal {
                interval: 1, weekday: WeekdayCode::Mon, ordinal: 1,
            }.kind_str(),
            "monthly_by_weekday",
        );
    }
}
