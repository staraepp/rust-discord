//! Rate-limit bookkeeping. Pure: time comes in as arguments.
//!
//! Model (userdoccers "Rate Limits"):
//! - Limits are per route, keyed additionally by the route's top-level
//!   ("major") resource: channel, guild or webhook ID.
//! - `X-RateLimit-Bucket`, when present, identifies a limit shared by several
//!   routes. User tokens usually do **not** get the `X-RateLimit-*` headers,
//!   only `Retry-After` / `X-RateLimit-Global` / `X-RateLimit-Scope` on 429,
//!   so the limiter must work from 429s alone as well.
//! - A global limit applies across all routes.

use std::collections::HashMap;
use std::time::Duration;

use dc_protocol::Snowflake;

use crate::http::Method;

/// Identifies an endpoint for rate-limiting purposes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Route {
    pub method: Method,
    /// Path template, e.g. `/channels/{channel.id}/messages`.
    pub template: &'static str,
    /// The top-level resource ID in the path, if any.
    pub major: Option<Snowflake>,
    /// The concrete path, e.g. `/channels/123/messages`.
    pub path: String,
}

impl Route {
    /// `GET /gateway`
    pub fn gateway() -> Self {
        Route {
            method: Method::Get,
            template: "/gateway",
            major: None,
            path: "/gateway".to_owned(),
        }
    }

    /// A CDN asset. All CDN downloads share one bucket.
    pub fn cdn() -> Self {
        Route {
            method: Method::Get,
            template: "cdn",
            major: None,
            path: String::new(),
        }
    }

    /// `GET /channels/{channel.id}/messages`
    pub fn channel_messages(channel_id: Snowflake) -> Self {
        Route {
            method: Method::Get,
            template: "/channels/{channel.id}/messages",
            major: Some(channel_id),
            path: format!("/channels/{channel_id}/messages"),
        }
    }

    fn template_key(&self) -> String {
        format!("{} {}", self.method.as_str(), self.template)
    }

    fn major_key(&self) -> String {
        self.major.map(|id| id.to_string()).unwrap_or_default()
    }
}

/// Rate-limit information extracted from a response.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RateLimitHeaders {
    pub limit: Option<u32>,
    pub remaining: Option<u32>,
    /// `X-RateLimit-Reset-After`, seconds.
    pub reset_after: Option<Duration>,
    pub bucket: Option<String>,
    /// `Retry-After` header (or the body's `retry_after` as a fallback).
    pub retry_after: Option<Duration>,
    /// `X-RateLimit-Global: true`, `X-RateLimit-Scope: global`, or the body's
    /// `global: true`.
    pub global: bool,
    /// `X-RateLimit-Scope`: `user`, `global` or `shared`.
    pub scope: Option<String>,
}

impl RateLimitHeaders {
    /// Parses the rate-limit headers. Header names are case-insensitive;
    /// malformed values are ignored.
    ///
    // ASSUMPTION: only the relative `X-RateLimit-Reset-After` is used. The
    // absolute `X-RateLimit-Reset` epoch would need a wall clock that agrees
    // with Discord's; the relative value carries the same information.
    pub fn parse<'a>(headers: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut out = RateLimitHeaders::default();
        for (name, value) in headers {
            let value = value.trim();
            match name.to_ascii_lowercase().as_str() {
                "x-ratelimit-limit" => out.limit = value.parse().ok(),
                "x-ratelimit-remaining" => out.remaining = value.parse().ok(),
                "x-ratelimit-reset-after" => out.reset_after = parse_seconds(value),
                "x-ratelimit-bucket" if !value.is_empty() => out.bucket = Some(value.to_owned()),
                "retry-after" => out.retry_after = parse_seconds(value),
                "x-ratelimit-global" => out.global |= value.eq_ignore_ascii_case("true"),
                "x-ratelimit-scope" => {
                    out.global |= value.eq_ignore_ascii_case("global");
                    out.scope = Some(value.to_ascii_lowercase());
                }
                _ => {}
            }
        }
        out
    }
}

/// Parses a non-negative, finite number of seconds (integer or decimal).
pub fn parse_seconds(value: &str) -> Option<Duration> {
    let seconds: f64 = value.parse().ok()?;
    Duration::try_from_secs_f64(seconds).ok()
}

#[derive(Clone, Debug, Default)]
struct Bucket {
    remaining: Option<u32>,
    /// When `remaining` refills / the block lifts.
    reset_at: Option<Duration>,
}

/// Tracks rate-limit state for every route used so far.
#[derive(Debug, Default)]
pub struct RateLimiter {
    global_until: Option<Duration>,
    /// Route template -> bucket hash from `X-RateLimit-Bucket`.
    bucket_hashes: HashMap<String, String>,
    buckets: HashMap<String, Bucket>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    fn bucket_key(&self, route: &Route) -> String {
        let template = route.template_key();
        let id = self.bucket_hashes.get(&template).unwrap_or(&template);
        format!("{id}|{}", route.major_key())
    }

    /// Asks permission to send a request on `route` at time `now`.
    ///
    /// Returns `Duration::ZERO` if the request may go out now (and counts it
    /// against the bucket), otherwise how long to wait before asking again.
    pub fn acquire(&mut self, route: &Route, now: Duration) -> Duration {
        if let Some(until) = self.global_until {
            if until > now {
                return until - now;
            }
            self.global_until = None;
        }

        let key = self.bucket_key(route);
        let bucket = self.buckets.entry(key).or_default();
        if let Some(reset_at) = bucket.reset_at {
            if reset_at <= now {
                // Window rolled over; we know nothing until the next response.
                bucket.remaining = None;
                bucket.reset_at = None;
            } else if bucket.remaining == Some(0) {
                return reset_at - now;
            }
        }
        if let Some(remaining) = bucket.remaining.as_mut() {
            // Count the in-flight request so concurrent callers cannot
            // overshoot before the response headers arrive.
            *remaining = remaining.saturating_sub(1);
        }
        Duration::ZERO
    }

    /// Records the outcome of a request sent on `route`.
    pub fn observe(&mut self, route: &Route, status: u16, info: &RateLimitHeaders, now: Duration) {
        if let Some(hash) = &info.bucket {
            let template = route.template_key();
            if self.bucket_hashes.get(&template) != Some(hash) {
                // Carry over what we knew under the provisional key.
                let old_key = self.bucket_key(route);
                self.bucket_hashes.insert(template, hash.clone());
                let new_key = self.bucket_key(route);
                if let Some(state) = self.buckets.remove(&old_key) {
                    self.buckets.entry(new_key).or_insert(state);
                }
            }
        }

        let key = self.bucket_key(route);
        let bucket = self.buckets.entry(key).or_default();

        if let (Some(remaining), Some(reset_after)) = (info.remaining, info.reset_after) {
            bucket.remaining = Some(remaining);
            bucket.reset_at = Some(now + reset_after);
        }

        if status == 429 {
            // No Retry-After means we cannot know when to come back; the
            // client surfaces that as an error instead of guessing.
            let Some(retry_after) = info.retry_after.or(info.reset_after) else {
                return;
            };
            let until = now + retry_after;
            if info.global {
                self.global_until = Some(self.global_until.map_or(until, |g| g.max(until)));
            } else {
                // Covers `user` and `shared` scopes: either way this bucket
                // is closed until the time given.
                bucket.remaining = Some(0);
                bucket.reset_at = Some(bucket.reset_at.map_or(until, |r| r.max(until)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn chan(id: u64) -> Route {
        Route::channel_messages(Snowflake(id))
    }

    fn headers(pairs: &[(&str, &str)]) -> RateLimitHeaders {
        RateLimitHeaders::parse(pairs.iter().copied())
    }

    #[test]
    fn parses_headers_case_insensitively() {
        let info = headers(&[
            ("X-RateLimit-Limit", "5"),
            ("x-ratelimit-remaining", "0"),
            ("X-RATELIMIT-RESET-AFTER", "1.5"),
            ("X-RateLimit-Bucket", "abcd1234"),
            ("Retry-After", "2"),
            ("X-RateLimit-Scope", "user"),
            ("Content-Type", "application/json"),
        ]);
        assert_eq!(info.limit, Some(5));
        assert_eq!(info.remaining, Some(0));
        assert_eq!(info.reset_after, Some(Duration::from_millis(1500)));
        assert_eq!(info.bucket.as_deref(), Some("abcd1234"));
        assert_eq!(info.retry_after, Some(S(2)));
        assert_eq!(info.scope.as_deref(), Some("user"));
        assert!(!info.global);
    }

    #[test]
    fn global_flag_from_either_header() {
        assert!(headers(&[("X-RateLimit-Global", "true")]).global);
        assert!(headers(&[("X-RateLimit-Scope", "global")]).global);
        assert!(!headers(&[("X-RateLimit-Scope", "shared")]).global);
    }

    #[test]
    fn malformed_values_are_ignored() {
        let info = headers(&[
            ("X-RateLimit-Remaining", "lots"),
            ("Retry-After", "-5"),
            ("X-RateLimit-Reset-After", "NaN"),
        ]);
        assert_eq!(info, RateLimitHeaders::default());
        assert_eq!(parse_seconds("inf"), None);
        assert_eq!(parse_seconds("0.25"), Some(Duration::from_millis(250)));
    }

    #[test]
    fn unknown_route_is_allowed() {
        let mut limiter = RateLimiter::new();
        assert_eq!(limiter.acquire(&chan(1), S(0)), Duration::ZERO);
    }

    #[test]
    fn exhausted_bucket_waits_until_reset() {
        let mut limiter = RateLimiter::new();
        let route = chan(1);
        assert_eq!(limiter.acquire(&route, S(0)), Duration::ZERO);
        limiter.observe(
            &route,
            200,
            &headers(&[
                ("X-RateLimit-Remaining", "0"),
                ("X-RateLimit-Reset-After", "5"),
            ]),
            S(0),
        );
        assert_eq!(limiter.acquire(&route, S(1)), S(4));
        assert_eq!(limiter.acquire(&route, S(4)), S(1));
        // After the reset the bucket is open again.
        assert_eq!(limiter.acquire(&route, S(5)), Duration::ZERO);
    }

    #[test]
    fn remaining_is_counted_down_for_in_flight_requests() {
        let mut limiter = RateLimiter::new();
        let route = chan(1);
        limiter.observe(
            &route,
            200,
            &headers(&[
                ("X-RateLimit-Remaining", "2"),
                ("X-RateLimit-Reset-After", "10"),
            ]),
            S(0),
        );
        assert_eq!(limiter.acquire(&route, S(1)), Duration::ZERO);
        assert_eq!(limiter.acquire(&route, S(1)), Duration::ZERO);
        // Third concurrent request must wait even though no response has
        // reported remaining=0 yet.
        assert_eq!(limiter.acquire(&route, S(1)), S(9));
    }

    #[test]
    fn major_parameters_are_limited_independently() {
        let mut limiter = RateLimiter::new();
        limiter.observe(
            &chan(1),
            429,
            &headers(&[("Retry-After", "30"), ("X-RateLimit-Scope", "user")]),
            S(0),
        );
        assert_eq!(limiter.acquire(&chan(1), S(0)), S(30));
        assert_eq!(limiter.acquire(&chan(2), S(0)), Duration::ZERO);
    }

    #[test]
    fn retry_after_only_429_blocks_the_route() {
        // What user tokens usually get: no X-RateLimit-* headers at all.
        let mut limiter = RateLimiter::new();
        let route = chan(1);
        limiter.observe(&route, 429, &headers(&[("Retry-After", "1.5")]), S(10));
        assert_eq!(limiter.acquire(&route, S(10)), Duration::from_millis(1500));
        assert_eq!(limiter.acquire(&route, S(12)), Duration::ZERO);
    }

    #[test]
    fn global_429_blocks_every_route() {
        let mut limiter = RateLimiter::new();
        limiter.observe(
            &chan(1),
            429,
            &headers(&[("Retry-After", "65"), ("X-RateLimit-Global", "true")]),
            S(0),
        );
        assert_eq!(limiter.acquire(&chan(2), S(5)), S(60));
        assert_eq!(limiter.acquire(&Route::gateway(), S(64)), S(1));
        assert_eq!(limiter.acquire(&Route::gateway(), S(65)), Duration::ZERO);
        // The per-route bucket was not marked exhausted by a global limit.
        assert_eq!(limiter.acquire(&chan(1), S(65)), Duration::ZERO);
    }

    #[test]
    fn shared_scope_429_still_blocks_the_bucket() {
        let mut limiter = RateLimiter::new();
        let route = chan(1);
        limiter.observe(
            &route,
            429,
            &headers(&[
                ("Retry-After", "7"),
                ("X-RateLimit-Scope", "shared"),
                ("X-RateLimit-Remaining", "9"),
                ("X-RateLimit-Reset-After", "7"),
            ]),
            S(0),
        );
        assert_eq!(limiter.acquire(&route, S(0)), S(7));
    }

    #[test]
    fn a_429_without_retry_after_does_not_invent_a_wait() {
        let mut limiter = RateLimiter::new();
        let route = chan(1);
        limiter.observe(&route, 429, &RateLimitHeaders::default(), S(0));
        assert_eq!(limiter.acquire(&route, S(0)), Duration::ZERO);
    }

    #[test]
    fn bucket_hash_keeps_state_from_provisional_key() {
        let mut limiter = RateLimiter::new();
        let route = chan(1);
        limiter.observe(&route, 429, &headers(&[("Retry-After", "10")]), S(0));
        // A later response names the bucket; the block must survive.
        limiter.observe(
            &route,
            429,
            &headers(&[("Retry-After", "10"), ("X-RateLimit-Bucket", "abcd1234")]),
            S(1),
        );
        assert_eq!(limiter.acquire(&route, S(2)), S(9));
        // Same bucket hash, different channel: independent.
        assert_eq!(limiter.acquire(&chan(2), S(2)), Duration::ZERO);
    }

    #[test]
    fn longer_block_wins() {
        let mut limiter = RateLimiter::new();
        let route = chan(1);
        limiter.observe(&route, 429, &headers(&[("Retry-After", "10")]), S(0));
        limiter.observe(&route, 429, &headers(&[("Retry-After", "2")]), S(1));
        assert_eq!(limiter.acquire(&route, S(1)), S(9));
    }
}
