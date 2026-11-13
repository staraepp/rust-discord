//! The REST client.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dc_protocol::model::{ApiErrorBody, CreateMessage, GatewayInfo, Message};
use dc_protocol::time::Clock;
use dc_protocol::{Secret, Snowflake};
use serde::de::DeserializeOwned;

use crate::http::{HttpError, HttpRequest, HttpTransport, Method};
use crate::ratelimit::{RateLimitHeaders, RateLimiter, Route};

/// Maximum `limit` for List Messages.
pub const MAX_MESSAGE_LIMIT: u8 = 100;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    Transport(#[from] HttpError),
    /// HTTP 401. The client then refuses all further authenticated requests:
    /// the docs require stopping once a token is known to be invalid.
    #[error("unauthorized (401): the token was rejected; no further requests will be sent")]
    Unauthorized,
    /// HTTP 429 that we will not (or can no longer) wait out.
    #[error("rate limited (429); retry after {retry_after:?}")]
    RateLimited { retry_after: Option<Duration> },
    /// Any other non-success status.
    #[error("http {status}: {message} (code {code:?})")]
    Status {
        status: u16,
        code: Option<u64>,
        message: String,
    },
    #[error("could not decode response body: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
}

/// REST client. Every request passes through the [`RateLimiter`].
pub struct ApiClient<H, C> {
    http: H,
    clock: C,
    base_url: String,
    token: Option<Secret<String>>,
    limiter: Mutex<RateLimiter>,
    token_rejected: AtomicBool,
    max_retries: u32,
    max_wait: Duration,
}

impl<H: HttpTransport, C: Clock> ApiClient<H, C> {
    /// `token` may be `None` for unauthenticated endpoints only.
    pub fn new(http: H, clock: C, token: Option<Secret<String>>) -> Self {
        ApiClient {
            http,
            clock,
            base_url: crate::API_BASE.to_owned(),
            token,
            limiter: Mutex::new(RateLimiter::new()),
            token_rejected: AtomicBool::new(false),
            max_retries: 2,
            max_wait: Duration::from_secs(60),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
        self
    }

    /// How many times a 429 is retried (after waiting `Retry-After`).
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Longest single rate-limit wait the client will sit through; anything
    /// longer is returned as [`ApiError::RateLimited`] for the caller to
    /// decide.
    pub fn with_max_wait(mut self, max_wait: Duration) -> Self {
        self.max_wait = max_wait;
        self
    }

    /// `GET /gateway`: the WebSocket URL to connect to. No auth required.
    pub async fn get_gateway(&self) -> Result<GatewayInfo, ApiError> {
        self.get_json(Route::gateway(), &[], false).await
    }

    /// `GET /channels/{channel.id}/messages`: the most recent messages,
    /// newest first. `limit` is 1-100; `before` pages backwards.
    pub async fn get_messages(
        &self,
        channel_id: Snowflake,
        limit: u8,
        before: Option<Snowflake>,
    ) -> Result<Vec<Message>, ApiError> {
        if limit == 0 || limit > MAX_MESSAGE_LIMIT {
            return Err(ApiError::InvalidArgument("limit must be between 1 and 100"));
        }
        let mut query = vec![("limit", limit.to_string())];
        if let Some(before) = before {
            query.push(("before", before.to_string()));
        }
        self.get_json(Route::channel_messages(channel_id), &query, true)
            .await
    }

    /// Downloads a public CDN asset (avatar, icon, banner, decoration).
    ///
    /// Only `https://cdn.discordapp.com/` URLs are accepted, and the token
    /// is never attached: the CDN is public.
    pub async fn get_cdn(&self, url: &str) -> Result<Vec<u8>, ApiError> {
        const PREFIX: &str = "https://cdn.discordapp.com/";
        if !url.starts_with(PREFIX) || url.contains(['\n', '\r', ' ']) {
            return Err(ApiError::InvalidArgument("not a Discord CDN URL"));
        }
        self.execute(Route::cdn(), url.to_owned(), None, None, None)
            .await
    }

    /// Sends one user-submitted message. Only explicit 429 responses are
    /// retried; ambiguous transport failures return to the composer so the
    /// user can retry with the same deduplication nonce.
    pub async fn create_message(
        &self,
        channel: Snowflake,
        message: &CreateMessage,
    ) -> Result<Message, ApiError> {
        validate_content(&message.content)?;
        if message.nonce.is_empty() || message.nonce.len() > 25 {
            return Err(ApiError::InvalidArgument("nonce must contain 1-25 bytes"));
        }
        let body = self
            .write_json(Route::create_message(channel), message)
            .await?;
        serde_json::from_slice(&body).map_err(ApiError::Decode)
    }

    pub async fn edit_message(
        &self,
        channel: Snowflake,
        message: Snowflake,
        content: &str,
    ) -> Result<Message, ApiError> {
        validate_content(content)?;
        let body = self
            .write_json(
                Route::message(channel, message, Method::Patch),
                &serde_json::json!({"content":content}),
            )
            .await?;
        serde_json::from_slice(&body).map_err(ApiError::Decode)
    }

    pub async fn delete_message(
        &self,
        channel: Snowflake,
        message: Snowflake,
    ) -> Result<(), ApiError> {
        self.write_json(
            Route::message(channel, message, Method::Delete),
            &serde_json::json!({}),
        )
        .await?;
        Ok(())
    }

    async fn write_json<T: serde::Serialize>(
        &self,
        route: Route,
        value: &T,
    ) -> Result<Vec<u8>, ApiError> {
        if self.token_rejected.load(Ordering::Relaxed) {
            return Err(ApiError::Unauthorized);
        }
        let token = self
            .token
            .clone()
            .ok_or(ApiError::InvalidArgument("this endpoint needs a token"))?;
        let body = serde_json::to_vec(value).map_err(ApiError::Decode)?;
        let url = format!("{}{}", self.base_url, route.path);
        self.execute(route, url, Some(token), Some(body), None)
            .await
    }

    pub(crate) async fn post_body(
        &self,
        route: Route,
        body: Vec<u8>,
        content_type: String,
    ) -> Result<Vec<u8>, ApiError> {
        if self.token_rejected.load(Ordering::Relaxed) {
            return Err(ApiError::Unauthorized);
        }
        let token = self
            .token
            .clone()
            .ok_or(ApiError::InvalidArgument("this endpoint needs a token"))?;
        let url = format!("{}{}", self.base_url, route.path);
        self.execute(route, url, Some(token), Some(body), Some(content_type))
            .await
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        route: Route,
        query: &[(&str, String)],
        authenticated: bool,
    ) -> Result<T, ApiError> {
        let body = self.request(route, query, authenticated).await?;
        serde_json::from_slice(&body).map_err(ApiError::Decode)
    }

    async fn request(
        &self,
        route: Route,
        query: &[(&str, String)],
        authenticated: bool,
    ) -> Result<Vec<u8>, ApiError> {
        let authorization = if authenticated {
            if self.token_rejected.load(Ordering::Relaxed) {
                return Err(ApiError::Unauthorized);
            }
            match &self.token {
                Some(token) => Some(token.clone()),
                None => return Err(ApiError::InvalidArgument("this endpoint needs a token")),
            }
        } else {
            None
        };

        // Query values here are numeric IDs/limits, so no escaping is needed.
        let mut url = format!("{}{}", self.base_url, route.path);
        for (index, (key, value)) in query.iter().enumerate() {
            url.push(if index == 0 { '?' } else { '&' });
            url.push_str(key);
            url.push('=');
            url.push_str(value);
        }

        self.execute(route, url, authorization, None, None).await
    }

    /// Sends one GET with rate limiting, 429 handling and error mapping.
    async fn execute(
        &self,
        route: Route,
        url: String,
        authorization: Option<Secret<String>>,
        body: Option<Vec<u8>>,
        content_type: Option<String>,
    ) -> Result<Vec<u8>, ApiError> {
        let authenticated = authorization.is_some();
        let mut attempt = 0;
        loop {
            self.wait_for_slot(&route).await?;
            if authenticated && self.token_rejected.load(Ordering::Relaxed) {
                return Err(ApiError::Unauthorized);
            }

            let response = self
                .http
                .execute(HttpRequest {
                    method: route.method,
                    url: url.clone(),
                    authorization: authorization.clone(),
                    body: body.clone(),
                    content_type: content_type.clone(),
                })
                .await?;

            let status = response.status;
            let error_body: ApiErrorBody = if (200..300).contains(&status) {
                ApiErrorBody::default()
            } else {
                serde_json::from_slice(&response.body).unwrap_or_default()
            };

            let mut info = RateLimitHeaders::parse(response.header_pairs());
            if status == 429 {
                // Header first; fall back to the JSON body.
                info.retry_after = info.retry_after.or_else(|| {
                    error_body
                        .retry_after
                        .and_then(|secs| Duration::try_from_secs_f64(secs).ok())
                });
                info.global |= error_body.global.unwrap_or(false);
            }
            self.limiter()
                .observe(&route, status, &info, self.clock.now());

            match status {
                200..300 => return Ok(response.body),
                401 => {
                    if authenticated {
                        self.token_rejected.store(true, Ordering::Relaxed);
                    }
                    return Err(ApiError::Unauthorized);
                }
                429 => {
                    tracing::warn!(
                        route = route.template,
                        retry_after = ?info.retry_after,
                        global = info.global,
                        "rate limited"
                    );
                    // Docs: without Retry-After, do not retry programmatically.
                    if info.retry_after.is_none() || attempt >= self.max_retries {
                        return Err(ApiError::RateLimited {
                            retry_after: info.retry_after,
                        });
                    }
                    attempt += 1;
                    // The limiter now holds the block; the next loop
                    // iteration waits it out before retrying.
                }
                _ => {
                    return Err(ApiError::Status {
                        status,
                        code: error_body.code,
                        message: error_body.message,
                    });
                }
            }
        }
    }

    /// Blocks until the limiter allows a request on `route`.
    async fn wait_for_slot(&self, route: &Route) -> Result<(), ApiError> {
        loop {
            let wait = self.limiter().acquire(route, self.clock.now());
            if wait.is_zero() {
                return Ok(());
            }
            if wait > self.max_wait {
                return Err(ApiError::RateLimited {
                    retry_after: Some(wait),
                });
            }
            tracing::debug!(route = route.template, ?wait, "waiting for rate limit");
            self.clock.sleep(wait).await;
        }
    }

    fn limiter(&self) -> std::sync::MutexGuard<'_, RateLimiter> {
        self.limiter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) fn validate_content(content: &str) -> Result<(), ApiError> {
    if content.trim().is_empty() {
        return Err(ApiError::InvalidArgument("message is empty"));
    }
    if content.encode_utf16().count() > 2000 {
        return Err(ApiError::InvalidArgument("message exceeds 2000 characters"));
    }
    Ok(())
}
