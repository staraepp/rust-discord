//! `Secret<T>`: a wrapper that keeps tokens out of logs, panics and debug
//! output (CLAUDE.md hard rule 1).

use std::fmt;

use serde::Serializer;
use zeroize::Zeroize;

const REDACTED: &str = "[REDACTED]";

/// Holds a secret value. `Debug` and `Display` never reveal it, it does not
/// implement `Serialize`, and the memory is zeroed on drop.
///
/// The only way to read the value is [`Secret::expose_secret`]; grep for that
/// name to audit every place a token leaves its wrapper.
#[derive(Clone)]
pub struct Secret<T: Zeroize>(T);

impl<T: Zeroize> Secret<T> {
    pub fn new(value: T) -> Self {
        Secret(value)
    }

    /// Returns the wrapped value. Only call this at the exact point the
    /// secret is written to the wire (TLS socket). Never log the result.
    pub fn expose_secret(&self) -> &T {
        &self.0
    }
}

impl<T: Zeroize> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl<T: Zeroize> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl<T: Zeroize> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl From<String> for Secret<String> {
    fn from(value: String) -> Self {
        Secret::new(value)
    }
}

/// `serialize_with` helper for the few wire payloads that must carry the
/// token (gateway Identify / Resume). Not for anything that could be logged.
pub fn serialize_exposed<S: Serializer>(
    secret: &Secret<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE: &str = "fake-token-for-tests-only";

    #[test]
    fn debug_and_display_redact() {
        let secret = Secret::new(FAKE.to_owned());
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
        assert_eq!(format!("{secret}"), "[REDACTED]");
        assert_eq!(format!("{secret:#?}"), "[REDACTED]");
    }

    #[test]
    fn redacts_inside_containers() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            token: Secret<String>,
            maybe: Option<Secret<String>>,
        }
        let holder = Holder {
            token: Secret::new(FAKE.to_owned()),
            maybe: Some(Secret::new(FAKE.to_owned())),
        };
        let rendered = format!("{holder:?}");
        assert!(!rendered.contains(FAKE), "leaked: {rendered}");
    }

    #[test]
    fn expose_returns_value() {
        let secret = Secret::new(FAKE.to_owned());
        assert_eq!(secret.expose_secret(), FAKE);
    }
}
