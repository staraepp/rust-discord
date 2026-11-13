//! HTTP abstraction: the trait `ApiClient` talks to, a mock for tests, and
//! the real reqwest-based transport (feature `reqwest`).

use std::fmt;
use std::future::Future;

use dc_protocol::Secret;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Patch,
    Delete,
    Put,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
            Method::Put => "PUT",
        }
    }
}

/// An outgoing request.
///
/// The `Authorization` value is kept apart from other data in a [`Secret`],
/// so `Debug` (and therefore logs, panics and test failures) never shows it.
#[derive(Clone)]
pub struct HttpRequest {
    pub method: Method,
    /// Full URL including query string. Must never contain secrets.
    pub url: String,
    pub authorization: Option<Secret<String>>,
    /// JSON request body. Debug intentionally omits message content.
    pub body: Option<Vec<u8>>,
    pub content_type: Option<String>,
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field(
                "authorization",
                &self.authorization.as_ref().map(|_| "[REDACTED]"),
            )
            .field("body_bytes", &self.body.as_ref().map(Vec::len))
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header_pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.headers.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// A transport-level failure (DNS, TLS, timeout, ...). The message must not
/// contain request headers.
#[derive(Clone, Debug, thiserror::Error)]
#[error("http transport error: {0}")]
pub struct HttpError(pub String);

pub trait HttpTransport: Send + Sync + 'static {
    fn execute(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, HttpError>> + Send;
}

pub mod mock {
    //! Scripted HTTP transport for tests.

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex, MutexGuard};

    use super::{HttpError, HttpRequest, HttpResponse, HttpTransport};

    #[derive(Default)]
    struct Shared {
        responses: VecDeque<Result<HttpResponse, HttpError>>,
        requests: Vec<HttpRequest>,
    }

    /// Returns queued responses in order and records every request.
    #[derive(Clone, Default)]
    pub struct MockHttp {
        shared: Arc<Mutex<Shared>>,
    }

    impl MockHttp {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn push(&self, status: u16, headers: &[(&str, &str)], body: &str) {
            self.lock().responses.push_back(Ok(HttpResponse {
                status,
                headers: headers
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
                body: body.as_bytes().to_vec(),
            }));
        }

        pub fn push_error(&self, message: &str) {
            self.lock()
                .responses
                .push_back(Err(HttpError(message.to_owned())));
        }

        /// Requests received so far.
        pub fn requests(&self) -> Vec<HttpRequest> {
            self.lock().requests.clone()
        }

        fn lock(&self) -> MutexGuard<'_, Shared> {
            self.shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        }
    }

    impl HttpTransport for MockHttp {
        async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            let mut shared = self.lock();
            shared.requests.push(request);
            shared
                .responses
                .pop_front()
                .unwrap_or_else(|| Err(HttpError("no scripted response left".into())))
        }
    }
}

#[cfg(feature = "reqwest")]
pub mod real {
    //! reqwest + rustls transport and the tokio clock.
    //!
    //! UNVERIFIED against the live API (cannot reach Discord from the
    //! sandbox); see `docs/LOCAL_VERIFY.md`.

    use std::time::Duration;

    use dc_protocol::time::Clock;
    use reqwest::header::{AUTHORIZATION, HeaderValue};

    use super::{HttpError, HttpRequest, HttpResponse, HttpTransport, Method};

    // ASSUMPTION: an honest, self-describing User-Agent is accepted for user
    // API requests. The docs point user clients at "client properties" /
    // X-Super-Properties (which mirror the official client and feed Discord's
    // anti-abuse systems); imitating those would be fingerprint spoofing and
    // is ruled out by CLAUDE.md hard rule 2.
    pub const USER_AGENT: &str = concat!(
        "dc-client/",
        env!("CARGO_PKG_VERSION"),
        " (unofficial native client; personal use)"
    );

    #[derive(Clone, Debug)]
    pub struct ReqwestTransport {
        client: reqwest::Client,
    }

    impl ReqwestTransport {
        pub fn new() -> Result<Self, HttpError> {
            let client = reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|err| HttpError(err.to_string()))?;
            Ok(ReqwestTransport { client })
        }
        fn build_request(&self, request: HttpRequest) -> Result<reqwest::Request, HttpError> {
            let mut builder = match request.method {
                Method::Get => self.client.get(&request.url),
                Method::Post => self.client.post(&request.url),
                Method::Patch => self.client.patch(&request.url),
                Method::Delete => self.client.delete(&request.url),
                Method::Put => self.client.put(&request.url),
            };
            if let Some(token) = &request.authorization {
                let mut value = HeaderValue::from_str(token.expose_secret())
                    // Do not include the underlying error: it could echo the value.
                    .map_err(|_| HttpError("token is not a valid header value".into()))?;
                value.set_sensitive(true);
                builder = builder.header(AUTHORIZATION, value);
            }
            if let Some(body) = request.body {
                builder = builder
                    .header(
                        "Content-Type",
                        request
                            .content_type
                            .as_deref()
                            .unwrap_or("application/json"),
                    )
                    .body(body);
            }

            builder
                .build()
                .map_err(|err| HttpError(err.without_url().to_string()))
        }
    }

    impl HttpTransport for ReqwestTransport {
        async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            let request = self.build_request(request)?;
            let response = self
                .client
                .execute(request)
                .await
                .map_err(|err| HttpError(err.without_url().to_string()))?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
                })
                .collect();
            let mut response = response;
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|err| HttpError(err.without_url().to_string()))?
            {
                if body.len() + chunk.len() > 32 * 1024 * 1024 {
                    return Err(HttpError("Response exceeds the 32 MiB limit".into()));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn multipart_content_type_and_sensitive_authorization_reach_request_builder() {
            let transport = ReqwestTransport::new().unwrap();
            let request = transport
                .build_request(HttpRequest {
                    method: Method::Post,
                    url: "https://example.invalid".into(),
                    authorization: Some(dc_protocol::Secret::new("synthetic-credential".into())),
                    body: Some(b"synthetic multipart".to_vec()),
                    content_type: Some("multipart/form-data; boundary=synthetic".into()),
                })
                .unwrap();
            assert_eq!(
                request.headers()["Content-Type"],
                "multipart/form-data; boundary=synthetic"
            );
            assert!(request.headers()["Authorization"].is_sensitive());
            assert!(!format!("{:?}", request.headers()).contains("synthetic-credential"));
        }
    }

    /// Monotonic clock backed by tokio's timer.
    #[derive(Clone, Debug)]
    pub struct TokioClock {
        origin: tokio::time::Instant,
    }

    impl TokioClock {
        pub fn new() -> Self {
            TokioClock {
                origin: tokio::time::Instant::now(),
            }
        }
    }

    impl Default for TokioClock {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Clock for TokioClock {
        fn now(&self) -> Duration {
            self.origin.elapsed()
        }

        async fn sleep(&self, duration: Duration) {
            tokio::time::sleep(duration).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_debug_redacts_authorization() {
        let fake = "fake-token-for-tests-only";
        let request = HttpRequest {
            method: Method::Get,
            url: "https://api.example.invalid/x".into(),
            authorization: Some(Secret::new(fake.to_owned())),
            body: None,
            content_type: None,
        };
        let rendered = format!("{request:?} {request:#?}");
        assert!(!rendered.contains(fake), "leaked: {rendered}");
        assert!(rendered.contains("[REDACTED]"));
    }
}
