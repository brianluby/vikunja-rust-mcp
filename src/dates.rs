//! Resolution of smart date shortcuts (`tomorrow`, `next friday`,
//! `in 2 weeks`, ...) into RFC 3339 timestamps.
//!
//! Resolution is deterministic: every function takes an explicit reference
//! instant, and the resolved timestamp is built in the reference's timezone
//! (the server's local timezone in production, a fixed offset when the
//! caller supplies one). Only the documented grammar is accepted — arbitrary
//! natural language is rejected with an actionable error.

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveTime, TimeZone, Weekday};

use crate::error::Error;

/// The zero date Vikunja uses to clear a date field on update.
pub const CLEAR_DATE_RFC3339: &str = "0001-01-01T00:00:00Z";

/// Times of day applied when a shortcut resolves to a calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateConfig {
    /// Time of day used by most shortcuts (`VIKUNJA_DATE_DEFAULT_TIME`).
    pub default_time: NaiveTime,
    /// Time of day used by `end of week` (`VIKUNJA_DATE_END_OF_DAY_TIME`).
    pub end_of_day_time: NaiveTime,
}

impl Default for DateConfig {
    fn default() -> Self {
        Self {
            default_time: NaiveTime::from_hms_opt(9, 0, 0).expect("09:00 is a valid time"),
            end_of_day_time: NaiveTime::from_hms_opt(23, 59, 0).expect("23:59 is a valid time"),
        }
    }
}

/// Parses a `HH:MM` time-of-day string (e.g. for config values).
pub fn parse_time_of_day(raw: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(raw.trim(), "%H:%M")
        .map_err(|_| format!("`{raw}` is not a valid time of day: use HH:MM, e.g. 09:00"))
}

/// Outcome of resolving a shortcut expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution<Tz: TimeZone> {
    /// The expression asks to clear the date (omit on create, send the zero
    /// date on update).
    Clear,
    /// The expression resolved to a timestamp; `time_of_day` reports which
    /// configured time was applied.
    Timestamp {
        datetime: DateTime<Tz>,
        time_of_day: NaiveTime,
    },
}

/// Resolves a shortcut expression against a reference instant.
///
/// Supported grammar (case-insensitive): `today`, `tomorrow`, `yesterday`,
/// `in N days|weeks|months`, `next monday`..`next sunday` (strictly after
/// today), bare `monday`..`sunday` (next occurrence, today included only
/// while the resolved time is still ahead of the reference), `end of week`
/// (upcoming Sunday at the end-of-day time), `YYYY-MM-DD`, and the clear
/// words `clear` / `none` / `no due date` / `unset`.
pub fn resolve<Tz: TimeZone>(
    expression: &str,
    reference: &DateTime<Tz>,
    config: &DateConfig,
) -> Result<Resolution<Tz>, Error> {
    let normalized = expression
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if matches!(
        normalized.as_str(),
        "clear" | "none" | "no due date" | "unset"
    ) {
        return Ok(Resolution::Clear);
    }

    let today = reference.date_naive();
    let (date, time_of_day) = match normalized.as_str() {
        "today" => (today, config.default_time),
        "tomorrow" => (add_days(today, 1)?, config.default_time),
        "yesterday" => (
            today
                .checked_sub_days(Days::new(1))
                .ok_or_else(|| out_of_range(expression))?,
            config.default_time,
        ),
        "end of week" => (
            upcoming_weekday(reference, Weekday::Sun, config.end_of_day_time)?,
            config.end_of_day_time,
        ),
        other => {
            if let Some(weekday) = parse_weekday(other) {
                (
                    upcoming_weekday(reference, weekday, config.default_time)?,
                    config.default_time,
                )
            } else if let Some(rest) = other.strip_prefix("next ") {
                let weekday = parse_weekday(rest).ok_or_else(|| unsupported(expression))?;
                (next_weekday_after(today, weekday)?, config.default_time)
            } else if let Some(rest) = other.strip_prefix("in ") {
                (in_relative(expression, today, rest)?, config.default_time)
            } else if looks_like_date(other) {
                let date = NaiveDate::parse_from_str(other, "%Y-%m-%d").map_err(|e| {
                    Error::InvalidArgument(format!(
                        "`{expression}` is not a valid YYYY-MM-DD date: {e}"
                    ))
                })?;
                (date, config.default_time)
            } else {
                return Err(unsupported(expression));
            }
        }
    };

    Ok(Resolution::Timestamp {
        datetime: local_timestamp(reference, date, time_of_day)?,
        time_of_day,
    })
}

/// Builds a timestamp for `date` at `time` in the reference's timezone.
///
/// DST edges: during a fall-back fold (the local time occurs twice) the
/// earlier, pre-fold occurrence is chosen; during a spring-forward gap (the
/// local time does not exist) an error asks for an explicit RFC 3339 value.
fn local_timestamp<Tz: TimeZone>(
    reference: &DateTime<Tz>,
    date: NaiveDate,
    time: NaiveTime,
) -> Result<DateTime<Tz>, Error> {
    let naive = date.and_time(time);
    reference
        .timezone()
        .from_local_datetime(&naive)
        .earliest()
        .ok_or_else(|| {
            Error::InvalidArgument(format!(
                "the resolved local time {naive} does not exist in the server timezone \
                 (daylight saving gap); pass an explicit RFC 3339 timestamp instead"
            ))
        })
}

/// Days from `today` until the next `target` weekday (0 when today is it).
fn days_until(today: NaiveDate, target: Weekday) -> u64 {
    u64::from((target.num_days_from_monday() + 7 - today.weekday().num_days_from_monday()) % 7)
}

/// Next occurrence of `weekday`, including today only while `time` today is
/// still ahead of the reference instant.
fn upcoming_weekday<Tz: TimeZone>(
    reference: &DateTime<Tz>,
    weekday: Weekday,
    time: NaiveTime,
) -> Result<NaiveDate, Error> {
    let today = reference.date_naive();
    let ahead = days_until(today, weekday);
    if ahead == 0 {
        let candidate = local_timestamp(reference, today, time)?;
        if candidate > *reference {
            return Ok(today);
        }
        return add_days(today, 7);
    }
    add_days(today, ahead)
}

/// First occurrence of `weekday` strictly after today.
fn next_weekday_after(today: NaiveDate, weekday: Weekday) -> Result<NaiveDate, Error> {
    let ahead = days_until(today, weekday);
    add_days(today, if ahead == 0 { 7 } else { ahead })
}

/// Parses the `N days|weeks|months` part of an `in ...` expression.
fn in_relative(expression: &str, today: NaiveDate, rest: &str) -> Result<NaiveDate, Error> {
    let mut parts = rest.split(' ');
    let (Some(count), Some(unit), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(unsupported(expression));
    };
    let count: u32 = count.parse().ok().filter(|n| *n > 0).ok_or_else(|| {
        Error::InvalidArgument(format!(
            "`{expression}`: N must be a positive integer, e.g. `in 3 days`"
        ))
    })?;
    let date = match unit {
        "day" | "days" => today.checked_add_days(Days::new(u64::from(count))),
        "week" | "weeks" => today.checked_add_days(Days::new(7 * u64::from(count))),
        // Calendar-aware: Jan 31 + 1 month clamps to the end of February.
        "month" | "months" => today.checked_add_months(Months::new(count)),
        _ => return Err(unsupported(expression)),
    };
    date.ok_or_else(|| out_of_range(expression))
}

fn add_days(date: NaiveDate, days: u64) -> Result<NaiveDate, Error> {
    date.checked_add_days(Days::new(days))
        .ok_or_else(|| out_of_range("the shortcut"))
}

fn parse_weekday(word: &str) -> Option<Weekday> {
    match word {
        "monday" => Some(Weekday::Mon),
        "tuesday" => Some(Weekday::Tue),
        "wednesday" => Some(Weekday::Wed),
        "thursday" => Some(Weekday::Thu),
        "friday" => Some(Weekday::Fri),
        "saturday" => Some(Weekday::Sat),
        "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

/// Cheap pre-check so clearly non-date words get the grammar error instead
/// of a date-parse error.
fn looks_like_date(word: &str) -> bool {
    word.len() >= 8 && word.chars().all(|c| c.is_ascii_digit() || c == '-')
}

fn unsupported(expression: &str) -> Error {
    Error::InvalidArgument(format!(
        "unsupported date shortcut `{expression}`: use today, tomorrow, yesterday, \
         in N days/weeks/months, [next] monday..sunday, end of week, YYYY-MM-DD, \
         or clear/none/unset/no due date"
    ))
}

fn out_of_range(expression: &str) -> Error {
    Error::InvalidArgument(format!("`{expression}` resolves to a date out of range"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn config() -> DateConfig {
        DateConfig::default()
    }

    /// Wednesday 2026-06-10 12:00 at UTC-4.
    fn reference() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-06-10T12:00:00-04:00").unwrap()
    }

    fn resolve_ok(expression: &str, reference: &DateTime<FixedOffset>) -> String {
        match resolve(expression, reference, &config()).unwrap() {
            Resolution::Timestamp { datetime, .. } => datetime.to_rfc3339(),
            Resolution::Clear => panic!("expected a timestamp for {expression}"),
        }
    }

    #[test]
    fn fixed_day_shortcuts() {
        let reference = reference();
        assert_eq!(resolve_ok("today", &reference), "2026-06-10T09:00:00-04:00");
        assert_eq!(
            resolve_ok("tomorrow", &reference),
            "2026-06-11T09:00:00-04:00"
        );
        assert_eq!(
            resolve_ok("yesterday", &reference),
            "2026-06-09T09:00:00-04:00"
        );
    }

    #[test]
    fn relative_shortcuts() {
        let reference = reference();
        assert_eq!(
            resolve_ok("in 3 days", &reference),
            "2026-06-13T09:00:00-04:00"
        );
        assert_eq!(
            resolve_ok("in 1 day", &reference),
            "2026-06-11T09:00:00-04:00"
        );
        assert_eq!(
            resolve_ok("in 2 weeks", &reference),
            "2026-06-24T09:00:00-04:00"
        );
        assert_eq!(
            resolve_ok("in 1 month", &reference),
            "2026-07-10T09:00:00-04:00"
        );
    }

    #[test]
    fn months_clamp_to_month_end() {
        let end_of_january = DateTime::parse_from_rfc3339("2026-01-31T08:00:00Z").unwrap();
        assert_eq!(
            resolve_ok("in 1 month", &end_of_january),
            "2026-02-28T09:00:00+00:00"
        );
    }

    #[test]
    fn next_weekday_is_strictly_after_today() {
        let reference = reference(); // Wednesday
        assert_eq!(
            resolve_ok("next friday", &reference),
            "2026-06-12T09:00:00-04:00"
        );
        // Today is Wednesday: "next wednesday" skips to next week.
        assert_eq!(
            resolve_ok("next wednesday", &reference),
            "2026-06-17T09:00:00-04:00"
        );
        assert_eq!(
            resolve_ok("next monday", &reference),
            "2026-06-15T09:00:00-04:00"
        );
    }

    #[test]
    fn bare_weekday_includes_today_only_while_still_ahead() {
        // 12:00 reference: today 09:00 already passed -> next week.
        assert_eq!(
            resolve_ok("wednesday", &reference()),
            "2026-06-17T09:00:00-04:00"
        );
        // 08:00 reference: today 09:00 is still ahead -> today.
        let early = DateTime::parse_from_rfc3339("2026-06-10T08:00:00-04:00").unwrap();
        assert_eq!(resolve_ok("wednesday", &early), "2026-06-10T09:00:00-04:00");
        // Other weekdays resolve within the next seven days.
        assert_eq!(
            resolve_ok("friday", &reference()),
            "2026-06-12T09:00:00-04:00"
        );
        assert_eq!(
            resolve_ok("sunday", &reference()),
            "2026-06-14T09:00:00-04:00"
        );
        assert_eq!(
            resolve_ok("tuesday", &reference()),
            "2026-06-16T09:00:00-04:00"
        );
    }

    #[test]
    fn end_of_week_is_upcoming_sunday_at_end_of_day() {
        assert_eq!(
            resolve_ok("end of week", &reference()),
            "2026-06-14T23:59:00-04:00"
        );
        // On a Sunday before 23:59, end of week is still that day.
        let sunday_morning = DateTime::parse_from_rfc3339("2026-06-14T10:00:00-04:00").unwrap();
        assert_eq!(
            resolve_ok("end of week", &sunday_morning),
            "2026-06-14T23:59:00-04:00"
        );
        // After 23:59 it rolls over to the next Sunday.
        let sunday_night = DateTime::parse_from_rfc3339("2026-06-14T23:59:30-04:00").unwrap();
        assert_eq!(
            resolve_ok("end of week", &sunday_night),
            "2026-06-21T23:59:00-04:00"
        );
    }

    #[test]
    fn explicit_date_uses_default_time_and_reference_offset() {
        assert_eq!(
            resolve_ok("2026-07-01", &reference()),
            "2026-07-01T09:00:00-04:00"
        );
    }

    #[test]
    fn clear_words_resolve_to_clear() {
        for expression in ["clear", "none", "no due date", "unset", "  Clear  ", "NONE"] {
            assert!(
                matches!(
                    resolve(expression, &reference(), &config()),
                    Ok(Resolution::Clear)
                ),
                "{expression}"
            );
        }
    }

    #[test]
    fn expressions_are_case_and_whitespace_insensitive() {
        let reference = reference();
        assert_eq!(
            resolve_ok("  Next   Friday ", &reference),
            "2026-06-12T09:00:00-04:00"
        );
        assert_eq!(resolve_ok("TODAY", &reference), "2026-06-10T09:00:00-04:00");
        assert_eq!(
            resolve_ok("In 3 DAYS", &reference),
            "2026-06-13T09:00:00-04:00"
        );
    }

    #[test]
    fn reports_which_time_of_day_was_used() {
        let custom = DateConfig {
            default_time: NaiveTime::from_hms_opt(8, 30, 0).unwrap(),
            end_of_day_time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
        };
        match resolve("tomorrow", &reference(), &custom).unwrap() {
            Resolution::Timestamp {
                datetime,
                time_of_day,
            } => {
                assert_eq!(datetime.to_rfc3339(), "2026-06-11T08:30:00-04:00");
                assert_eq!(time_of_day, custom.default_time);
            }
            Resolution::Clear => panic!("expected timestamp"),
        }
        match resolve("end of week", &reference(), &custom).unwrap() {
            Resolution::Timestamp { time_of_day, .. } => {
                assert_eq!(time_of_day, custom.end_of_day_time);
            }
            Resolution::Clear => panic!("expected timestamp"),
        }
    }

    #[test]
    fn unsupported_expressions_are_rejected_with_guidance() {
        for expression in [
            "someday",
            "next week",
            "in two days",
            "in 0 days",
            "in -2 days",
            "in 3 hours",
            "next",
            "",
            "07/01/2026",
        ] {
            let err = resolve(expression, &reference(), &config()).unwrap_err();
            assert!(
                matches!(err, Error::InvalidArgument(_)),
                "{expression}: {err:?}"
            );
        }
        // Date-shaped but invalid: gets a date-specific error.
        let err = resolve("2026-13-40", &reference(), &config()).unwrap_err();
        assert!(err.to_string().contains("YYYY-MM-DD"), "{err}");
    }

    #[test]
    fn time_of_day_parsing() {
        assert_eq!(
            parse_time_of_day("09:00").unwrap(),
            NaiveTime::from_hms_opt(9, 0, 0).unwrap()
        );
        assert_eq!(
            parse_time_of_day(" 23:59 ").unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap()
        );
        for raw in ["9am", "25:00", "12:60", "", "12:00:30"] {
            assert!(parse_time_of_day(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn default_config_times() {
        let config = DateConfig::default();
        assert_eq!(
            config.default_time,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap()
        );
        assert_eq!(
            config.end_of_day_time,
            NaiveTime::from_hms_opt(23, 59, 0).unwrap()
        );
    }
}
