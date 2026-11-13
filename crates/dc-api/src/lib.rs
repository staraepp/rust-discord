//! Discord REST client (user API v9).
//!
//! - [`ratelimit`]: pure per-bucket + global rate-limit bookkeeping.
//! - [`http`]: the `HttpTransport` trait, a mock, and (feature `reqwest`) the
//!   real HTTPS transport.
//! - [`client`]: `ApiClient`, which never sends a request the limiter says to
//!   hold back, and honors `Retry-After` on 429.
//!
//! The token only ever lives in a [`Secret`]; `HttpRequest`'s `Debug` output
//! redacts the `Authorization` header.

pub mod client;
pub mod http;
pub mod ratelimit;

pub use client::{ApiClient, ApiError};
pub use dc_protocol::Secret;
pub use http::{HttpError, HttpRequest, HttpResponse, HttpTransport, Method};
pub use ratelimit::{RateLimitHeaders, RateLimiter, Route};

/// Default REST base URL.
pub const API_BASE: &str = "https://discord.com/api/v9";

pub mod upload;
pub use upload::UploadFile;
