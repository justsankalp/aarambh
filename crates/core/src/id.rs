//! Time-sortable IDs (ULID: 48-bit millisecond timestamp + 80 random bits,
//! Crockford base32, 26 characters).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

/// An event, edge or risk ID. Sorts by creation time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(Ulid);

impl Id {
    /// Builds an ID from a millisecond timestamp and caller-supplied randomness.
    /// Randomness comes from the caller so that core stays free of RNG
    /// dependencies (the web app uses `crypto.getRandomValues`).
    pub fn new(ts_ms: u64, random: u128) -> Self {
        Id(Ulid::from_parts(ts_ms, random))
    }

    /// Milliseconds since the Unix epoch encoded in the ID.
    pub fn timestamp_ms(&self) -> u64 {
        self.0.timestamp_ms()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for Id {
    type Err = InvalidId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_string(s)
            .map(Id)
            .map_err(|_| InvalidId(s.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidId(pub String);

impl fmt::Display for InvalidId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid id {:?}", self.0)
    }
}

impl std::error::Error for InvalidId {}

impl Serialize for Id {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sort_by_time() {
        let a = Id::new(1_000, u128::MAX >> 48);
        let b = Id::new(1_001, 0);
        assert!(a < b);
        assert!(
            a.to_string() < b.to_string(),
            "string order matches time order"
        );
    }

    #[test]
    fn round_trips_through_string_and_json() {
        let id = Id::new(1_761_843_600_000, 0x1234_5678_9abc_def0_1234);
        let s = id.to_string();
        assert_eq!(s.len(), 26);
        assert_eq!(s.parse::<Id>().unwrap(), id);
        assert_eq!(id.timestamp_ms(), 1_761_843_600_000);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{s}\""));
        assert_eq!(serde_json::from_str::<Id>(&json).unwrap(), id);
    }

    #[test]
    fn rejects_garbage() {
        assert!("not-an-id".parse::<Id>().is_err());
        assert!(serde_json::from_str::<Id>("\"<script>\"").is_err());
    }
}
