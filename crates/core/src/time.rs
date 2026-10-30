//! UTC timestamps stored as RFC 3339 strings (`2026-10-30T16:58:01.123Z`)
//! and the UTC day keys used for event file names.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

const MS_PER_DAY: i64 = 86_400_000;

/// Milliseconds since the Unix epoch, UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Timestamp(pub u64);

/// A UTC calendar day, used for `data/events/YYYY-MM-DD.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl Timestamp {
    pub fn day(self) -> Day {
        Day::from_days_since_epoch((self.0 as i64).div_euclid(MS_PER_DAY))
    }
}

impl Day {
    /// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    pub fn from_days_since_epoch(z: i64) -> Day {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Day { year, month, day }
    }

    pub fn days_since_epoch(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let m = i64::from(self.month);
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn offset(self, days: i64) -> Day {
        Day::from_days_since_epoch(self.days_since_epoch() + days)
    }

    /// Path of this day's event file, relative to the repo root.
    pub fn events_path(self) -> String {
        format!("data/events/{self}.json")
    }

    fn is_valid(self) -> bool {
        (1..=12).contains(&self.month)
            && self.day >= 1
            && Day::from_days_since_epoch(self.days_since_epoch()) == self
    }
}

impl fmt::Display for Day {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl FromStr for Day {
    type Err = InvalidTime;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || InvalidTime(s.to_owned());
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return Err(err());
        }
        let day = Day {
            year: digits(&s[0..4]).ok_or_else(err)? as i32,
            month: digits(&s[5..7]).ok_or_else(err)? as u8,
            day: digits(&s[8..10]).ok_or_else(err)? as u8,
        };
        if day.is_valid() { Ok(day) } else { Err(err()) }
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = self.0 % MS_PER_DAY as u64;
        let (h, m, s, frac) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
        write!(f, "{}T{h:02}:{m:02}:{s:02}.{frac:03}Z", self.day())
    }
}

impl FromStr for Timestamp {
    type Err = InvalidTime;

    /// Accepts `YYYY-MM-DDTHH:MM:SS[.fff…]Z`. Only UTC is allowed.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || InvalidTime(s.to_owned());
        let b = s.as_bytes();
        if b.len() < 20 || b[10] != b'T' || b[13] != b':' || b[16] != b':' || !s.ends_with('Z') {
            return Err(err());
        }
        let day: Day = s[0..10].parse()?;
        let (h, m, sec) = (
            digits(&s[11..13]).ok_or_else(err)?,
            digits(&s[14..16]).ok_or_else(err)?,
            digits(&s[17..19]).ok_or_else(err)?,
        );
        if h > 23 || m > 59 || sec > 59 {
            return Err(err());
        }
        let rest = &s[19..s.len() - 1];
        let frac_ms = match rest.strip_prefix('.') {
            None if rest.is_empty() => 0,
            Some(f) if !f.is_empty() && f.len() <= 9 => {
                let padded = format!("{f:0<3}");
                digits(&padded[..3]).ok_or_else(err)?
            }
            _ => return Err(err()),
        };
        let days = day.days_since_epoch();
        if days < 0 {
            return Err(err());
        }
        Ok(Timestamp(
            days as u64 * MS_PER_DAY as u64 + (h * 3600 + m * 60 + sec) * 1000 + frac_ms,
        ))
    }
}

fn digits(s: &str) -> Option<u64> {
    if s.bytes().all(|c| c.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidTime(pub String);

impl fmt::Display for InvalidTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid UTC time {:?}", self.0)
    }
}

impl std::error::Error for InvalidTime {}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_parses_known_instants() {
        let cases = [
            (0, "1970-01-01T00:00:00.000Z"),
            (951_782_400_000, "2000-02-29T00:00:00.000Z"),
            (1_761_843_481_123, "2025-10-30T16:58:01.123Z"),
            (4_107_542_399_999, "2100-02-28T23:59:59.999Z"),
        ];
        for (ms, s) in cases {
            assert_eq!(Timestamp(ms).to_string(), s);
            assert_eq!(s.parse::<Timestamp>().unwrap(), Timestamp(ms));
        }
    }

    #[test]
    fn accepts_seconds_only_and_long_fractions() {
        assert_eq!(
            "1970-01-01T00:00:01Z".parse::<Timestamp>().unwrap(),
            Timestamp(1000)
        );
        assert_eq!(
            "1970-01-01T00:00:01.5Z".parse::<Timestamp>().unwrap(),
            Timestamp(1500)
        );
        assert_eq!(
            "1970-01-01T00:00:01.123456Z".parse::<Timestamp>().unwrap(),
            Timestamp(1123)
        );
    }

    #[test]
    fn rejects_bad_times() {
        for s in [
            "2026-10-30T16:58:01+05:30",
            "2026-02-30T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-10-30T24:00:00Z",
            "2026-10-30 16:58:01Z",
            "2026-10-30T16:58:01.Z",
            "1969-12-31T23:59:59Z",
            "",
        ] {
            assert!(s.parse::<Timestamp>().is_err(), "{s} should be rejected");
        }
    }

    #[test]
    fn day_round_trips_over_a_wide_range() {
        for z in (-700_000..2_900_000).step_by(997) {
            let d = Day::from_days_since_epoch(z);
            assert_eq!(d.days_since_epoch(), z);
            assert_eq!(d.to_string().parse::<Day>().unwrap(), d);
        }
    }

    #[test]
    fn utc_day_boundaries_and_paths() {
        let last_ms = "2026-10-30T23:59:59.999Z".parse::<Timestamp>().unwrap();
        assert_eq!(last_ms.day().to_string(), "2026-10-30");
        assert_eq!(Timestamp(last_ms.0 + 1).day().to_string(), "2026-10-31");
        let d: Day = "2026-12-31".parse().unwrap();
        assert_eq!(d.offset(1).to_string(), "2027-01-01");
        assert_eq!(d.offset(-365).to_string(), "2025-12-31");
        assert_eq!(d.events_path(), "data/events/2026-12-31.json");
    }
}
