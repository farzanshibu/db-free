// SOT: workflow-schedule-math, next-fire-time, schedule-summary, dst-aware-schedule

use crate::model::workflow::{IntervalUnit, ScheduleConfig, ScheduleMode, Weekday};
use chrono::{DateTime, Days, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

// ============================================================================
// SCHEDULE MATH
//
// WHAT:  When does a Schedule node fire next, and how is it described in words?
// WHY:   The scheduler only needs "the next instant after now"; keeping the
//        arithmetic here, free of clocks and tasks, makes the DST edge cases
//        testable.
// HOW:   Interval mode adds the interval (never less than 10 s). At-time mode
//        walks the next 14 days, keeps the chosen weekdays, and converts the
//        wall-clock time in the chosen IANA zone (blank = the machine's zone).
//        A wall time the zone skips (spring forward) fires right after the gap;
//        one it repeats (fall back) fires the first time.
// WHERE: src-tauri/src/services/workflows/scheduler.rs
// ============================================================================

/// An interval shorter than this is raised to it.
pub const MIN_INTERVAL_SECONDS: u64 = 10;

fn interval_seconds(config: &ScheduleConfig) -> Option<u64> {
    if config.interval_amount == 0 {
        return None;
    }
    let unit = match config.interval_unit {
        IntervalUnit::Seconds => 1,
        IntervalUnit::Minutes => 60,
        IntervalUnit::Hours => 3_600,
        IntervalUnit::Days => 86_400,
    };
    Some((u64::from(config.interval_amount) * unit).max(MIN_INTERVAL_SECONDS))
}

fn parse_time(text: &str) -> Option<(u32, u32)> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

fn chrono_weekday(day: Weekday) -> chrono::Weekday {
    match day {
        Weekday::Mon => chrono::Weekday::Mon,
        Weekday::Tue => chrono::Weekday::Tue,
        Weekday::Wed => chrono::Weekday::Wed,
        Weekday::Thu => chrono::Weekday::Thu,
        Weekday::Fri => chrono::Weekday::Fri,
        Weekday::Sat => chrono::Weekday::Sat,
        Weekday::Sun => chrono::Weekday::Sun,
    }
}

/// Why a schedule can never fire; `None` when it is fine.
pub fn problem(config: &ScheduleConfig) -> Option<String> {
    match config.mode {
        ScheduleMode::Interval => interval_seconds(config).is_none().then(|| "The interval must be at least 1.".to_string()),
        ScheduleMode::AtTime => {
            if parse_time(&config.time).is_none() {
                Some("The time must be HH:MM, 24-hour.".to_string())
            } else if config.days.is_empty() {
                Some("Pick at least one weekday.".to_string())
            } else if !config.timezone.trim().is_empty() && config.timezone.trim().parse::<Tz>().is_err() {
                Some(format!("\"{}\" is not a time zone name.", config.timezone.trim()))
            } else {
                None
            }
        }
    }
}

// WHAT:  The first fire time strictly after `after`.
pub fn next_fire(config: &ScheduleConfig, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match config.mode {
        ScheduleMode::Interval => {
            let seconds = i64::try_from(interval_seconds(config)?).ok()?;
            after.checked_add_signed(chrono::Duration::try_seconds(seconds)?)
        }
        ScheduleMode::AtTime => {
            problem(config).is_none().then_some(())?;
            let (hour, minute) = parse_time(&config.time)?;
            let days: Vec<chrono::Weekday> = config.days.iter().map(|d| chrono_weekday(*d)).collect();
            let zone = config.timezone.trim();
            if zone.is_empty() {
                next_at_time(&Local, hour, minute, &days, after)
            } else {
                next_at_time(&zone.parse::<Tz>().ok()?, hour, minute, &days, after)
            }
        }
    }
}

fn next_at_time<Z: TimeZone>(zone: &Z, hour: u32, minute: u32, days: &[chrono::Weekday], after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    use chrono::Datelike;
    let today: NaiveDate = after.with_timezone(zone).date_naive();
    for offset in 0..15_u64 {
        let date = today.checked_add_days(Days::new(offset))?;
        if !days.contains(&date.weekday()) {
            continue;
        }
        let wall = date.and_hms_opt(hour, minute, 0)?;
        if let Some(instant) = resolve(zone, wall) {
            if instant > after {
                return Some(instant);
            }
        }
    }
    None
}

// WHAT:  A wall-clock time in a zone as an instant.
// HOW:   Ambiguous (clocks go back): the first occurrence. Missing (clocks jump
//        forward): the first minute after the gap that exists.
fn resolve<Z: TimeZone>(zone: &Z, wall: NaiveDateTime) -> Option<DateTime<Utc>> {
    let mut candidate = wall;
    for _ in 0..=8 {
        match zone.from_local_datetime(&candidate) {
            chrono::LocalResult::Single(instant) => return Some(instant.with_timezone(&Utc)),
            chrono::LocalResult::Ambiguous(first, _) => return Some(first.with_timezone(&Utc)),
            chrono::LocalResult::None => candidate = candidate.checked_add_signed(chrono::Duration::try_minutes(15)?)?,
        }
    }
    None
}

fn day_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

// WHAT:  The schedule in words, for Settings and the canvas.
pub fn summarize(config: &ScheduleConfig) -> String {
    match config.mode {
        ScheduleMode::Interval => {
            let n = config.interval_amount;
            let unit = match config.interval_unit {
                IntervalUnit::Seconds => "second",
                IntervalUnit::Minutes => "minute",
                IntervalUnit::Hours => "hour",
                IntervalUnit::Days => "day",
            };
            if n == 1 {
                format!("Every {unit}")
            } else {
                format!("Every {n} {unit}s")
            }
        }
        ScheduleMode::AtTime => {
            let order = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun];
            let chosen: Vec<&str> = order.iter().filter(|d| config.days.contains(d)).map(|d| day_name(*d)).collect();
            let days = if chosen.len() == 7 { "Every day".to_string() } else { chosen.join(", ") };
            let zone = if config.timezone.trim().is_empty() { "local time".to_string() } else { config.timezone.trim().to_string() };
            format!("{days} at {} ({zone})", config.time.trim())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339).unwrap().with_timezone(&Utc)
    }

    fn interval(amount: u32, unit: IntervalUnit) -> ScheduleConfig {
        ScheduleConfig { mode: ScheduleMode::Interval, interval_amount: amount, interval_unit: unit, ..ScheduleConfig::default() }
    }

    fn daily(time: &str, zone: &str, days: Vec<Weekday>) -> ScheduleConfig {
        ScheduleConfig { mode: ScheduleMode::AtTime, time: time.into(), timezone: zone.into(), days, ..ScheduleConfig::default() }
    }

    const ALL_DAYS: [Weekday; 7] = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun];

    #[test]
    fn intervals_add_and_never_go_below_ten_seconds() {
        let now = at("2026-01-01T00:00:00Z");
        assert_eq!(next_fire(&interval(5, IntervalUnit::Minutes), now), Some(at("2026-01-01T00:05:00Z")));
        assert_eq!(next_fire(&interval(2, IntervalUnit::Days), now), Some(at("2026-01-03T00:00:00Z")));
        assert_eq!(next_fire(&interval(1, IntervalUnit::Seconds), now), Some(at("2026-01-01T00:00:10Z")));
        assert_eq!(next_fire(&interval(0, IntervalUnit::Hours), now), None);
        assert!(problem(&interval(0, IntervalUnit::Hours)).is_some());
    }

    #[test]
    fn at_time_picks_the_next_chosen_weekday() {
        // 2026-03-04 is a Wednesday.
        let cfg = daily("09:00", "UTC", vec![Weekday::Mon, Weekday::Fri]);
        assert_eq!(next_fire(&cfg, at("2026-03-04T12:00:00Z")), Some(at("2026-03-06T09:00:00Z")));
        let every = daily("09:00", "UTC", ALL_DAYS.to_vec());
        assert_eq!(next_fire(&every, at("2026-03-04T08:59:59Z")), Some(at("2026-03-04T09:00:00Z")));
        assert_eq!(next_fire(&every, at("2026-03-04T09:00:00Z")), Some(at("2026-03-05T09:00:00Z")), "strictly after");
    }

    #[test]
    fn at_time_honours_the_zone() {
        let cfg = daily("09:00", "Asia/Kolkata", ALL_DAYS.to_vec());
        assert_eq!(next_fire(&cfg, at("2026-03-04T00:00:00Z")), Some(at("2026-03-04T03:30:00Z")));
        let paris = daily("08:00", "Europe/Paris", ALL_DAYS.to_vec());
        // Paris is UTC+1 in winter and UTC+2 in summer.
        assert_eq!(next_fire(&paris, at("2026-01-10T00:00:00Z")), Some(at("2026-01-10T07:00:00Z")));
        assert_eq!(next_fire(&paris, at("2026-07-10T00:00:00Z")), Some(at("2026-07-10T06:00:00Z")));
    }

    #[test]
    fn dst_gaps_and_repeats_resolve() {
        // US spring forward: 2026-03-08, 02:30 does not exist in New York.
        let gap = daily("02:30", "America/New_York", ALL_DAYS.to_vec());
        let fire = next_fire(&gap, at("2026-03-08T00:00:00Z")).unwrap();
        assert_eq!(fire, at("2026-03-08T07:00:00Z"), "fires at 03:00 EDT, right after the gap");
        // Fall back: 2026-11-01, 01:30 happens twice; the first one is EDT (UTC-4).
        let repeat = daily("01:30", "America/New_York", ALL_DAYS.to_vec());
        assert_eq!(next_fire(&repeat, at("2026-11-01T00:00:00Z")), Some(at("2026-11-01T05:30:00Z")));
    }

    #[test]
    fn invalid_schedules_have_a_reason_and_never_fire() {
        let now = at("2026-03-04T00:00:00Z");
        assert!(problem(&daily("25:00", "UTC", ALL_DAYS.to_vec())).is_some());
        assert!(problem(&daily("09:00", "UTC", vec![])).is_some());
        assert!(problem(&daily("09:00", "Mars/Olympus", ALL_DAYS.to_vec())).is_some());
        assert_eq!(next_fire(&daily("09:00", "Mars/Olympus", ALL_DAYS.to_vec()), now), None);
        assert!(problem(&daily("9:05", "UTC", ALL_DAYS.to_vec())).is_none());
        assert!(next_fire(&daily("9:05", "", ALL_DAYS.to_vec()), now).is_some(), "a blank zone is the machine's own");
    }

    #[test]
    fn summaries_read_naturally() {
        assert_eq!(summarize(&interval(1, IntervalUnit::Hours)), "Every hour");
        assert_eq!(summarize(&interval(15, IntervalUnit::Minutes)), "Every 15 minutes");
        assert_eq!(summarize(&daily("09:00", "Europe/Paris", vec![Weekday::Wed, Weekday::Mon])), "Mon, Wed at 09:00 (Europe/Paris)");
        assert_eq!(summarize(&daily("18:30", "", ALL_DAYS.to_vec())), "Every day at 18:30 (local time)");
    }
}
