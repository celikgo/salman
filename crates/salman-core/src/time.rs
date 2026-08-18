//! IEC 61131-3 time and date values.
//!
//! # Representation, and why
//!
//! * `TIME` and `LTIME` are both held as a signed count of **nanoseconds** in
//!   an `i64`, giving a range of roughly ±292 years at nanosecond resolution.
//!   IEC 61131-3 leaves the range and resolution of `TIME` implementation-
//!   defined; salman picks one representation for both so that the runtime
//!   never silently loses precision converting between them. A dialect may
//!   narrow what a *literal* is allowed to express (some vendors' `TIME` is a
//!   32-bit millisecond count), and that is a dialect rule, checked in
//!   `salman-lang`, not a property of the value.
//! * `DATE` is a day count relative to 1970-01-01.
//! * `TIME_OF_DAY` is nanoseconds since midnight, always less than 24 h.
//! * `DATE_AND_TIME` is nanoseconds relative to 1970-01-01T00:00:00.
//!
//! # What is deliberately not modelled
//!
//! Leap seconds, time zones and daylight saving. IEC 61131-3 date and time
//! values carry no zone, every day here is exactly 86 400 s, and salman does no
//! conversion that would need a zone. A controller that needs civil time
//! handling gets it from the plant, not from this module.
//!
//! # Determinism
//!
//! Every operation here is integer arithmetic with explicit overflow handling.
//! Nothing in this module reads a clock, and no operation can produce a
//! different answer on a different platform.

use std::fmt;
use std::fmt::Write as _;

/// Nanoseconds in a microsecond.
const NS_PER_US: i64 = 1_000;
/// Nanoseconds in a millisecond.
const NS_PER_MS: i64 = 1_000_000;
/// Nanoseconds in a second.
const NS_PER_S: i64 = 1_000_000_000;
/// Nanoseconds in a minute.
const NS_PER_MIN: i64 = 60 * NS_PER_S;
/// Nanoseconds in an hour.
const NS_PER_HOUR: i64 = 60 * NS_PER_MIN;
/// Nanoseconds in a day.
const NS_PER_DAY: i64 = 24 * NS_PER_HOUR;

/// An IEC `TIME` / `LTIME` duration: signed nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Duration(i64);

impl Duration {
    /// Zero duration, `T#0s`.
    pub const ZERO: Self = Self(0);
    /// The most negative representable duration.
    pub const MIN: Self = Self(i64::MIN);
    /// The largest representable duration.
    pub const MAX: Self = Self(i64::MAX);

    /// A duration of exactly `nanos` nanoseconds.
    #[must_use]
    pub const fn from_nanos(nanos: i64) -> Self {
        Self(nanos)
    }

    /// A duration of `millis` milliseconds, or `None` on overflow.
    #[must_use]
    pub const fn from_millis(millis: i64) -> Option<Self> {
        match millis.checked_mul(NS_PER_MS) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }

    /// A duration of `secs` seconds, or `None` on overflow.
    #[must_use]
    pub const fn from_secs(secs: i64) -> Option<Self> {
        match secs.checked_mul(NS_PER_S) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }

    /// Builds a duration from IEC literal components.
    ///
    /// Every component is added, so `T#90s` and `T#1m30s` are the same value —
    /// IEC duration literals permit a unit to overflow its usual range, and the
    /// value is the sum of the parts.
    ///
    /// Returns `None` if the total does not fit in an `i64` of nanoseconds.
    #[must_use]
    pub fn from_parts(parts: DurationParts) -> Option<Self> {
        let mut total: i64 = 0;
        for (value, scale) in [
            (parts.days, NS_PER_DAY),
            (parts.hours, NS_PER_HOUR),
            (parts.minutes, NS_PER_MIN),
            (parts.seconds, NS_PER_S),
            (parts.millis, NS_PER_MS),
            (parts.micros, NS_PER_US),
            (parts.nanos, 1),
        ] {
            let scaled = i64::try_from(value).ok()?.checked_mul(scale)?;
            total = total.checked_add(scaled)?;
        }
        if parts.negative {
            total = total.checked_neg()?;
        }
        Some(Self(total))
    }

    /// The duration in whole nanoseconds.
    #[must_use]
    pub const fn nanos(self) -> i64 {
        self.0
    }

    /// The duration in whole milliseconds, truncated toward zero.
    #[must_use]
    pub const fn as_millis(self) -> i64 {
        self.0 / NS_PER_MS
    }

    /// Whether the duration is negative.
    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Sum, or `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }

    /// Difference, or `None` on overflow.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }

    /// Sum, saturating at the representable bounds.
    ///
    /// Used by the runtime's elapsed-time accumulators, where a timer that has
    /// been running for 292 years must stop counting rather than wrap into a
    /// negative elapsed time.
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// Product with an integer, or `None` on overflow.
    #[must_use]
    pub const fn checked_mul(self, factor: i64) -> Option<Self> {
        match self.0.checked_mul(factor) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }

    /// Quotient by an integer, or `None` for a zero divisor or on overflow.
    ///
    /// `i64::MIN / -1` overflows, which is why this is checked rather than a
    /// plain division.
    #[must_use]
    pub const fn checked_div(self, divisor: i64) -> Option<Self> {
        match self.0.checked_div(divisor) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }

    /// Renders the canonical IEC literal for this duration, e.g. `T#1d2h3m4s`.
    ///
    /// The canonical form emits the largest units first, omits zero components,
    /// and renders zero as `T#0s`. It round-trips through salman's lexer.
    #[must_use]
    pub fn to_iec_literal(self) -> String {
        let mut out = String::from("T#");
        if self.0 == 0 {
            out.push_str("0s");
            return out;
        }
        if self.0 < 0 {
            out.push('-');
        }
        // Taking the magnitude of i64::MIN would overflow, so work in u64.
        let mut rest = self.0.unsigned_abs();
        for (scale, suffix) in [
            (NS_PER_DAY as u64, "d"),
            (NS_PER_HOUR as u64, "h"),
            (NS_PER_MIN as u64, "m"),
            (NS_PER_S as u64, "s"),
            (NS_PER_MS as u64, "ms"),
            (NS_PER_US as u64, "us"),
            (1u64, "ns"),
        ] {
            let units = rest / scale;
            if units > 0 {
                let _ = write!(out, "{units}{suffix}");
                rest -= units * scale;
            }
        }
        out
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_iec_literal())
    }
}

/// The components of an IEC duration literal, before they are summed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DurationParts {
    /// Whether the literal carried a leading minus sign.
    pub negative: bool,
    /// Days component.
    pub days: u64,
    /// Hours component.
    pub hours: u64,
    /// Minutes component.
    pub minutes: u64,
    /// Seconds component.
    pub seconds: u64,
    /// Milliseconds component.
    pub millis: u64,
    /// Microseconds component.
    pub micros: u64,
    /// Nanoseconds component.
    pub nanos: u64,
}

/// An IEC `DATE`: a whole number of days relative to 1970-01-01.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Date(i32);

impl Date {
    /// 1970-01-01, the base of IEC date values.
    pub const EPOCH: Self = Self(0);

    /// A date from a proleptic Gregorian calendar date.
    ///
    /// Returns `None` if the date does not exist — 2023-02-29, for instance.
    #[must_use]
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Self> {
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self(days_from_civil(year, month, day)?))
    }

    /// Days relative to 1970-01-01, negative before it.
    #[must_use]
    pub const fn days_since_epoch(self) -> i32 {
        self.0
    }

    /// A date a given number of days from the epoch.
    #[must_use]
    pub const fn from_days_since_epoch(days: i32) -> Self {
        Self(days)
    }

    /// The proleptic Gregorian year, month and day.
    #[must_use]
    pub fn to_ymd(self) -> (i32, u32, u32) {
        civil_from_days(self.0)
    }

    /// Renders the canonical IEC literal, e.g. `D#2024-02-29`.
    #[must_use]
    pub fn to_iec_literal(self) -> String {
        let (year, month, day) = self.to_ymd();
        format!("D#{year:04}-{month:02}-{day:02}")
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_iec_literal())
    }
}

/// An IEC `TIME_OF_DAY`: nanoseconds since midnight, always below 24 h.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TimeOfDay(u64);

impl TimeOfDay {
    /// Midnight.
    pub const MIDNIGHT: Self = Self(0);

    /// Nanoseconds in one day, the exclusive upper bound of a time of day.
    pub const NANOS_PER_DAY: u64 = NS_PER_DAY as u64;

    /// A time of day, or `None` if it is not a real time.
    ///
    /// Hour 24 is rejected: IEC time-of-day values are strictly within a day,
    /// and accepting 24:00:00 would make `TOD#24:00:00` and `TOD#00:00:00`
    /// compare unequal while meaning the same instant.
    #[must_use]
    pub const fn from_hms_nano(hour: u32, minute: u32, second: u32, nano: u32) -> Option<Self> {
        if hour > 23 || minute > 59 || second > 59 || nano > 999_999_999 {
            return None;
        }
        let total = hour as u64 * NS_PER_HOUR as u64
            + minute as u64 * NS_PER_MIN as u64
            + second as u64 * NS_PER_S as u64
            + nano as u64;
        Some(Self(total))
    }

    /// Nanoseconds since midnight.
    #[must_use]
    pub const fn nanos_since_midnight(self) -> u64 {
        self.0
    }

    /// Hour, minute, second and nanosecond components.
    #[must_use]
    pub const fn to_hms_nano(self) -> (u32, u32, u32, u32) {
        let hour = self.0 / NS_PER_HOUR as u64;
        let rest = self.0 % NS_PER_HOUR as u64;
        let minute = rest / NS_PER_MIN as u64;
        let rest = rest % NS_PER_MIN as u64;
        let second = rest / NS_PER_S as u64;
        let nano = rest % NS_PER_S as u64;
        (hour as u32, minute as u32, second as u32, nano as u32)
    }

    /// Renders the canonical IEC literal, e.g. `TOD#12:34:56.789`.
    ///
    /// Fractional seconds are emitted only when non-zero, at the shortest of
    /// millisecond, microsecond or nanosecond precision that is exact.
    #[must_use]
    pub fn to_iec_literal(self) -> String {
        let (hour, minute, second, nano) = self.to_hms_nano();
        let mut out = format!("TOD#{hour:02}:{minute:02}:{second:02}");
        append_fraction(&mut out, nano);
        out
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_iec_literal())
    }
}

/// An IEC `DATE_AND_TIME`: nanoseconds relative to 1970-01-01T00:00:00.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DateTime(i64);

impl DateTime {
    /// 1970-01-01T00:00:00.
    pub const EPOCH: Self = Self(0);

    /// Combines a date and a time of day.
    ///
    /// Returns `None` for dates so far from the epoch that the combination does
    /// not fit in a nanosecond count — roughly beyond ±292 years.
    #[must_use]
    pub fn from_date_time(date: Date, time: TimeOfDay) -> Option<Self> {
        let days = i64::from(date.days_since_epoch());
        let base = days.checked_mul(NS_PER_DAY)?;
        let nanos = i64::try_from(time.nanos_since_midnight()).ok()?;
        Some(Self(base.checked_add(nanos)?))
    }

    /// Nanoseconds relative to the epoch.
    #[must_use]
    pub const fn nanos_since_epoch(self) -> i64 {
        self.0
    }

    /// A value a given number of nanoseconds from the epoch.
    #[must_use]
    pub const fn from_nanos_since_epoch(nanos: i64) -> Self {
        Self(nanos)
    }

    /// Splits into a date and a time of day.
    ///
    /// Uses floor division so that instants before the epoch land on the
    /// correct day rather than being truncated toward zero.
    #[must_use]
    pub fn split(self) -> (Date, TimeOfDay) {
        let days = self.0.div_euclid(NS_PER_DAY);
        let rest = self.0.rem_euclid(NS_PER_DAY);
        let days = i32::try_from(days).unwrap_or(if days < 0 { i32::MIN } else { i32::MAX });
        (Date(days), TimeOfDay(rest.unsigned_abs()))
    }

    /// Renders the canonical IEC literal, e.g. `DT#2024-02-29-12:34:56.789`.
    #[must_use]
    pub fn to_iec_literal(self) -> String {
        let (date, tod) = self.split();
        let (year, month, day) = date.to_ymd();
        let (hour, minute, second, nano) = tod.to_hms_nano();
        let mut out = format!("DT#{year:04}-{month:02}-{day:02}-{hour:02}:{minute:02}:{second:02}");
        append_fraction(&mut out, nano);
        out
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_iec_literal())
    }
}

/// Appends `.mmm`, `.uuuuuu` or `.nnnnnnnnn`, whichever is exact, or nothing.
fn append_fraction(out: &mut String, nanos: u32) {
    if nanos == 0 {
        return;
    }
    if nanos.is_multiple_of(1_000_000) {
        let _ = write!(out, ".{:03}", nanos / 1_000_000);
    } else if nanos.is_multiple_of(1_000) {
        let _ = write!(out, ".{:06}", nanos / 1_000);
    } else {
        let _ = write!(out, ".{nanos:09}");
    }
}

/// Whether `year` is a leap year in the proleptic Gregorian calendar.
#[must_use]
pub const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days in `month` of `year`, or 0 for a month outside 1..=12.
#[must_use]
pub const fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days from 1970-01-01 to a civil date, proleptic Gregorian.
///
/// Howard Hinnant's `days_from_civil`, which is in the public domain and is the
/// algorithm the C++20 `<chrono>` calendar is specified against. Reference:
/// <https://howardhinnant.github.io/date_algorithms.html#days_from_civil>
fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i32> {
    let y = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = i32::try_from(month).ok()?;
    let d = i32::try_from(day).ok()?;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era.checked_mul(146_097)?
        .checked_add(doe)?
        .checked_sub(719_468)
}

/// Civil date from a day count relative to 1970-01-01, proleptic Gregorian.
///
/// Howard Hinnant's `civil_from_days`, the exact inverse of
/// [`days_from_civil`]. Reference:
/// <https://howardhinnant.github.io/date_algorithms.html#civil_from_days>
fn civil_from_days(days: i32) -> (i32, u32, u32) {
    let z = days.saturating_add(719_468);
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_literal_components_are_summed_so_a_unit_may_overflow() {
        // T#90s and T#1m30s denote the same duration.
        let ninety = Duration::from_parts(DurationParts {
            seconds: 90,
            ..Default::default()
        });
        let one_thirty = Duration::from_parts(DurationParts {
            minutes: 1,
            seconds: 30,
            ..Default::default()
        });
        assert_eq!(ninety, one_thirty);
        assert_eq!(ninety.unwrap().nanos(), 90 * NS_PER_S);
    }

    #[test]
    fn duration_supports_every_iec_unit_down_to_nanoseconds() {
        let d = Duration::from_parts(DurationParts {
            days: 1,
            hours: 2,
            minutes: 3,
            seconds: 4,
            millis: 5,
            micros: 6,
            nanos: 7,
            negative: false,
        })
        .unwrap();
        let expected = NS_PER_DAY
            + 2 * NS_PER_HOUR
            + 3 * NS_PER_MIN
            + 4 * NS_PER_S
            + 5 * NS_PER_MS
            + 6 * NS_PER_US
            + 7;
        assert_eq!(d.nanos(), expected);
    }

    #[test]
    fn negative_durations_are_representable() {
        let d = Duration::from_parts(DurationParts {
            negative: true,
            seconds: 5,
            ..Default::default()
        })
        .unwrap();
        assert!(d.is_negative());
        assert_eq!(d.nanos(), -5 * NS_PER_S);
        assert_eq!(d.to_iec_literal(), "T#-5s");
    }

    #[test]
    fn duration_literal_round_trips_through_its_canonical_form() {
        let cases = [
            (Duration::ZERO, "T#0s"),
            (Duration::from_nanos(1), "T#1ns"),
            (Duration::from_nanos(NS_PER_MS), "T#1ms"),
            (
                Duration::from_nanos(NS_PER_DAY + NS_PER_HOUR * 2 + NS_PER_MIN * 3),
                "T#1d2h3m",
            ),
            (Duration::from_nanos(1_500_000), "T#1ms500us"),
        ];
        for (value, text) in cases {
            assert_eq!(value.to_iec_literal(), text);
        }
    }

    #[test]
    fn the_most_negative_duration_formats_without_overflowing() {
        // Negating i64::MIN overflows; formatting must not attempt it.
        let text = Duration::MIN.to_iec_literal();
        assert!(text.starts_with("T#-"), "{text}");
    }

    #[test]
    fn duration_arithmetic_reports_overflow_rather_than_wrapping() {
        assert_eq!(Duration::MAX.checked_add(Duration::from_nanos(1)), None);
        assert_eq!(Duration::MIN.checked_sub(Duration::from_nanos(1)), None);
        assert_eq!(Duration::MAX.checked_mul(2), None);
        // i64::MIN / -1 is the classic overflow; it must not abort.
        assert_eq!(Duration::MIN.checked_div(-1), None);
        assert_eq!(Duration::from_nanos(10).checked_div(0), None);
    }

    #[test]
    fn saturating_add_is_what_elapsed_time_accumulators_use() {
        assert_eq!(
            Duration::MAX.saturating_add(Duration::from_nanos(1)),
            Duration::MAX
        );
    }

    #[test]
    fn the_date_epoch_is_1970_01_01() {
        assert_eq!(Date::from_ymd(1970, 1, 1), Some(Date::EPOCH));
        assert_eq!(Date::EPOCH.to_ymd(), (1970, 1, 1));
        assert_eq!(Date::EPOCH.to_iec_literal(), "D#1970-01-01");
    }

    #[test]
    fn leap_years_follow_the_gregorian_rule() {
        assert!(is_leap_year(2024));
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(1900));
        assert!(!is_leap_year(2023));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert!(Date::from_ymd(2024, 2, 29).is_some());
        assert_eq!(Date::from_ymd(2023, 2, 29), None);
        assert_eq!(Date::from_ymd(1900, 2, 29), None);
    }

    #[test]
    fn impossible_dates_are_rejected_rather_than_normalised() {
        assert_eq!(Date::from_ymd(2024, 13, 1), None);
        assert_eq!(Date::from_ymd(2024, 0, 1), None);
        assert_eq!(Date::from_ymd(2024, 4, 31), None);
        assert_eq!(Date::from_ymd(2024, 1, 0), None);
    }

    #[test]
    fn date_conversion_round_trips_over_a_wide_range() {
        // Every day across two centuries, including all leap-year edge cases.
        for days in -73_000i32..=73_000 {
            let date = Date::from_days_since_epoch(days);
            let (y, m, d) = date.to_ymd();
            assert_eq!(
                Date::from_ymd(y, m, d),
                Some(date),
                "round trip failed at day {days} ({y}-{m}-{d})"
            );
        }
    }

    #[test]
    fn dates_before_the_epoch_work() {
        let d = Date::from_ymd(1969, 12, 31).unwrap();
        assert_eq!(d.days_since_epoch(), -1);
        assert_eq!(d.to_iec_literal(), "D#1969-12-31");
    }

    #[test]
    fn time_of_day_rejects_hour_24_so_midnight_has_one_representation() {
        assert!(TimeOfDay::from_hms_nano(23, 59, 59, 999_999_999).is_some());
        assert_eq!(TimeOfDay::from_hms_nano(24, 0, 0, 0), None);
        assert_eq!(TimeOfDay::from_hms_nano(0, 60, 0, 0), None);
        // Leap seconds are not modelled, so second 60 does not exist.
        assert_eq!(TimeOfDay::from_hms_nano(12, 0, 60, 0), None);
        assert_eq!(TimeOfDay::from_hms_nano(0, 0, 0, 1_000_000_000), None);
    }

    #[test]
    fn time_of_day_round_trips_and_renders_the_shortest_exact_fraction() {
        let t = TimeOfDay::from_hms_nano(12, 34, 56, 789_000_000).unwrap();
        assert_eq!(t.to_hms_nano(), (12, 34, 56, 789_000_000));
        assert_eq!(t.to_iec_literal(), "TOD#12:34:56.789");

        let micro = TimeOfDay::from_hms_nano(1, 2, 3, 4_000).unwrap();
        assert_eq!(micro.to_iec_literal(), "TOD#01:02:03.000004");

        let nano = TimeOfDay::from_hms_nano(1, 2, 3, 7).unwrap();
        assert_eq!(nano.to_iec_literal(), "TOD#01:02:03.000000007");

        assert_eq!(TimeOfDay::MIDNIGHT.to_iec_literal(), "TOD#00:00:00");
    }

    #[test]
    fn date_and_time_splits_correctly_before_the_epoch() {
        // Floor division: one nanosecond before the epoch is the last
        // nanosecond of 1969-12-31, not "day zero, negative time".
        let dt = DateTime::from_nanos_since_epoch(-1);
        let (date, tod) = dt.split();
        assert_eq!(date.to_ymd(), (1969, 12, 31));
        assert_eq!(tod.to_hms_nano(), (23, 59, 59, 999_999_999));
        assert_eq!(dt.to_iec_literal(), "DT#1969-12-31-23:59:59.999999999");
    }

    #[test]
    fn date_and_time_round_trips_through_its_parts() {
        let date = Date::from_ymd(2024, 2, 29).unwrap();
        let tod = TimeOfDay::from_hms_nano(12, 34, 56, 789_000_000).unwrap();
        let dt = DateTime::from_date_time(date, tod).unwrap();
        assert_eq!(dt.split(), (date, tod));
        assert_eq!(dt.to_iec_literal(), "DT#2024-02-29-12:34:56.789");
    }

    #[test]
    fn every_day_is_exactly_86400_seconds_because_leap_seconds_are_not_modelled() {
        let a =
            DateTime::from_date_time(Date::from_ymd(2016, 12, 31).unwrap(), TimeOfDay::MIDNIGHT)
                .unwrap();
        let b = DateTime::from_date_time(Date::from_ymd(2017, 1, 1).unwrap(), TimeOfDay::MIDNIGHT)
            .unwrap();
        // A leap second was inserted at the end of 2016-12-31 in civil time.
        // salman does not model it, and says so rather than being subtly wrong.
        assert_eq!(
            b.nanos_since_epoch() - a.nanos_since_epoch(),
            86_400 * NS_PER_S
        );
    }

    #[test]
    fn duration_from_parts_reports_overflow_instead_of_wrapping() {
        assert_eq!(
            Duration::from_parts(DurationParts {
                days: u64::MAX,
                ..Default::default()
            }),
            None
        );
    }
}
