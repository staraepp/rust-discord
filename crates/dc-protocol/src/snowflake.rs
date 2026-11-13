//! Discord snowflake IDs.

use std::fmt;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A 64-bit Discord ID.
///
/// On the wire snowflakes are strings (to survive JS number precision), but
/// some payloads carry them as integers, so both are accepted. Always
/// serialized as a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Snowflake(pub u64);

impl Snowflake {
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for Snowflake {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Snowflake)
    }
}

impl From<u64> for Snowflake {
    fn from(value: u64) -> Self {
        Snowflake(value)
    }
}

impl Serialize for Snowflake {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Snowflake {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SnowflakeVisitor;

        impl Visitor<'_> for SnowflakeVisitor {
            type Value = Snowflake;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a snowflake as a decimal string or unsigned integer")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Snowflake, E> {
                v.parse().map(Snowflake).map_err(E::custom)
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Snowflake, E> {
                Ok(Snowflake(v))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Snowflake, E> {
                u64::try_from(v).map(Snowflake).map_err(E::custom)
            }
        }

        deserializer.deserialize_any(SnowflakeVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_string_and_integer() {
        let a: Snowflake = serde_json::from_str("\"100000000000000001\"").unwrap();
        let b: Snowflake = serde_json::from_str("100000000000000001").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.get(), 100_000_000_000_000_001);
    }

    #[test]
    fn serializes_as_string() {
        let json = serde_json::to_string(&Snowflake(42)).unwrap();
        assert_eq!(json, "\"42\"");
    }

    #[test]
    fn rejects_garbage() {
        assert!(serde_json::from_str::<Snowflake>("\"abc\"").is_err());
        assert!(serde_json::from_str::<Snowflake>("-1").is_err());
    }
}
