//! `ApiClient` tests with the scripted HTTP transport and fake clock.

use std::sync::Arc;
use std::time::Duration;

use dc_api::http::mock::MockHttp;
use dc_api::{ApiClient, ApiError, Method, Secret};
use dc_protocol::Snowflake;
use dc_protocol::time::{Clock, FakeClock};

macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/",
            $path
        ))
    };
}

const FAKE_TOKEN: &str = "fake-token-for-tests-only";
const BASE: &str = "https://api.example.invalid/api/v9";
const CHANNEL: Snowflake = Snowflake(300_000_000_000_000_002);
const MESSAGES: &str = fixture!("rest/messages.json");

type Client = ApiClient<MockHttp, FakeClock>;

fn client(http: &MockHttp, clock: &FakeClock) -> Client {
    ApiClient::new(
        http.clone(),
        clock.clone(),
        Some(Secret::new(FAKE_TOKEN.to_owned())),
    )
    .with_base_url(BASE)
}

async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn get_messages_builds_the_request_and_parses_the_response() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    http.push(200, &[], MESSAGES);
    let api = client(&http, &clock);

    let messages = api
        .get_messages(CHANNEL, 25, Some(Snowflake(400_000_000_000_000_009)))
        .await
        .unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].content, "newest message");

    let requests = http.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::Get);
    assert_eq!(
        requests[0].url,
        "https://api.example.invalid/api/v9/channels/300000000000000002/messages?limit=25&before=400000000000000009"
    );
    // The token is attached, but only reachable through expose_secret().
    let auth = requests[0].authorization.as_ref().expect("authorization");
    assert_eq!(auth.expose_secret(), FAKE_TOKEN);
    assert!(!format!("{requests:?}").contains(FAKE_TOKEN));
}

#[tokio::test]
async fn limit_is_validated_before_any_request() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    let api = client(&http, &clock);
    for bad in [0, 101, 255] {
        assert!(matches!(
            api.get_messages(CHANNEL, bad, None).await,
            Err(ApiError::InvalidArgument(_))
        ));
    }
    assert!(http.requests().is_empty());
}

#[tokio::test]
async fn get_gateway_sends_no_authorization() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    http.push(200, &[], fixture!("rest/gateway.json"));
    let api = client(&http, &clock);
    let info = api.get_gateway().await.unwrap();
    assert_eq!(info.url, "wss://gateway.example.invalid");
    let requests = http.requests();
    assert_eq!(
        requests[0].url,
        "https://api.example.invalid/api/v9/gateway"
    );
    assert!(requests[0].authorization.is_none());
}

#[tokio::test]
async fn retry_after_header_is_honored_before_retrying() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    http.push(
        429,
        &[("Retry-After", "2"), ("X-RateLimit-Scope", "user")],
        fixture!("rest/rate_limited.json"),
    );
    http.push(200, &[], MESSAGES);
    let api = Arc::new(client(&http, &clock));

    let task = tokio::spawn({
        let api = api.clone();
        async move { api.get_messages(CHANNEL, 50, None).await }
    });
    settle().await;
    assert_eq!(http.requests().len(), 1);
    // Header (2s) wins over the body's retry_after (1.5s).
    assert_eq!(clock.next_wakeup_in(), Some(Duration::from_secs(2)));

    clock.advance(Duration::from_millis(1999));
    settle().await;
    assert_eq!(http.requests().len(), 1, "must not retry early");
    assert!(!task.is_finished());

    clock.advance(Duration::from_millis(1));
    settle().await;
    assert_eq!(http.requests().len(), 2);
    assert_eq!(task.await.unwrap().unwrap().len(), 3);
}

#[tokio::test]
async fn retry_after_falls_back_to_the_json_body() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    http.push(429, &[], fixture!("rest/rate_limited.json"));
    http.push(200, &[], MESSAGES);
    let api = Arc::new(client(&http, &clock));

    let task = tokio::spawn({
        let api = api.clone();
        async move { api.get_messages(CHANNEL, 50, None).await }
    });
    settle().await;
    assert_eq!(clock.next_wakeup_in(), Some(Duration::from_millis(1500)));
    clock.advance(Duration::from_millis(1500));
    settle().await;
    assert!(task.await.unwrap().is_ok());
}

#[tokio::test]
async fn a_429_without_retry_after_is_not_retried() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    http.push(429, &[], "<html>cloudflare</html>");
    let api = client(&http, &clock);
    assert!(matches!(
        api.get_messages(CHANNEL, 50, None).await,
        Err(ApiError::RateLimited { retry_after: None })
    ));
    assert_eq!(http.requests().len(), 1);
    assert_eq!(clock.pending_sleeps(), 0);
}

#[tokio::test]
async fn retries_are_bounded() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    for _ in 0..3 {
        http.push(429, &[("Retry-After", "1")], "{}");
    }
    let api = Arc::new(client(&http, &clock).with_max_retries(2));
    let task = tokio::spawn({
        let api = api.clone();
        async move { api.get_messages(CHANNEL, 50, None).await }
    });
    for _ in 0..2 {
        settle().await;
        clock.advance(Duration::from_secs(1));
    }
    settle().await;
    let result = task.await.unwrap();
    assert!(matches!(
        result,
        Err(ApiError::RateLimited {
            retry_after: Some(d)
        }) if d == Duration::from_secs(1)
    ));
    assert_eq!(http.requests().len(), 3);
}

#[tokio::test]
async fn exhausted_bucket_delays_the_next_request_without_a_429() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    let limited = [
        ("X-RateLimit-Limit", "5"),
        ("X-RateLimit-Remaining", "0"),
        ("X-RateLimit-Reset-After", "3.5"),
        ("X-RateLimit-Bucket", "abcd1234"),
    ];
    http.push(200, &limited, MESSAGES);
    http.push(200, &[], MESSAGES);
    http.push(200, &[], "[]");
    let api = Arc::new(client(&http, &clock));

    api.get_messages(CHANNEL, 50, None).await.unwrap();

    // Same channel: held back until the bucket resets.
    let task = tokio::spawn({
        let api = api.clone();
        async move { api.get_messages(CHANNEL, 50, None).await }
    });
    settle().await;
    assert_eq!(http.requests().len(), 1);
    assert_eq!(clock.next_wakeup_in(), Some(Duration::from_millis(3500)));

    // A different channel is a different bucket and goes straight through.
    api.get_messages(Snowflake(300_000_000_000_000_999), 50, None)
        .await
        .unwrap();
    assert_eq!(http.requests().len(), 2);

    clock.advance(Duration::from_millis(3500));
    settle().await;
    assert!(task.await.unwrap().is_ok());
    assert_eq!(http.requests().len(), 3);
}

#[tokio::test]
async fn global_rate_limit_blocks_other_routes_too() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    http.push(
        429,
        &[("Retry-After", "3"), ("X-RateLimit-Global", "true")],
        fixture!("rest/rate_limited_global.json"),
    );
    http.push(200, &[], fixture!("rest/gateway.json"));
    let api = Arc::new(client(&http, &clock).with_max_retries(0));

    assert!(matches!(
        api.get_messages(CHANNEL, 50, None).await,
        Err(ApiError::RateLimited { .. })
    ));

    let task = tokio::spawn({
        let api = api.clone();
        async move { api.get_gateway().await }
    });
    settle().await;
    assert_eq!(http.requests().len(), 1, "global limit holds /gateway back");
    clock.advance(Duration::from_secs(3));
    settle().await;
    assert!(task.await.unwrap().is_ok());
}

#[tokio::test]
async fn waits_longer_than_max_wait_are_returned_to_the_caller() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    http.push(429, &[("Retry-After", "1337")], "{}");
    let api = client(&http, &clock);
    let result = api.get_messages(CHANNEL, 50, None).await;
    assert!(matches!(
        result,
        Err(ApiError::RateLimited { retry_after: Some(d) }) if d == Duration::from_secs(1337)
    ));
    assert_eq!(http.requests().len(), 1);
    assert_eq!(clock.now(), Duration::ZERO, "did not sleep");
}

#[tokio::test]
async fn unauthorized_latches_and_stops_further_requests() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    http.push(401, &[], fixture!("rest/unauthorized.json"));
    let api = client(&http, &clock);

    assert!(matches!(
        api.get_messages(CHANNEL, 50, None).await,
        Err(ApiError::Unauthorized)
    ));
    // Second call never reaches the network.
    let second = api.get_messages(CHANNEL, 50, None).await;
    assert!(matches!(second, Err(ApiError::Unauthorized)));
    assert_eq!(http.requests().len(), 1);
    let rendered = format!("{:?} {}", second, second.as_ref().unwrap_err());
    assert!(!rendered.contains(FAKE_TOKEN));
}

#[tokio::test]
async fn other_errors_carry_status_code_and_message() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    http.push(403, &[], fixture!("rest/missing_access.json"));
    http.push(502, &[], "<html>bad gateway</html>");
    http.push_error("connection reset");
    http.push(200, &[], "{not json");
    let api = client(&http, &clock);

    match api.get_messages(CHANNEL, 50, None).await {
        Err(ApiError::Status {
            status,
            code,
            message,
        }) => {
            assert_eq!((status, code), (403, Some(50001)));
            assert_eq!(message, "Missing Access");
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert!(matches!(
        api.get_messages(CHANNEL, 50, None).await,
        Err(ApiError::Status { status: 502, .. })
    ));
    assert!(matches!(
        api.get_messages(CHANNEL, 50, None).await,
        Err(ApiError::Transport(_))
    ));
    assert!(matches!(
        api.get_messages(CHANNEL, 50, None).await,
        Err(ApiError::Decode(_))
    ));
}

#[tokio::test]
async fn authenticated_endpoint_without_token_makes_no_request() {
    let (http, clock) = (MockHttp::new(), FakeClock::new());
    let api: Client = ApiClient::new(http.clone(), clock, None).with_base_url(BASE);
    assert!(matches!(
        api.get_messages(CHANNEL, 50, None).await,
        Err(ApiError::InvalidArgument(_))
    ));
    assert!(http.requests().is_empty());
}

#[tokio::test]
async fn create_message_posts_json_and_preserves_nonce_across_429() {
    use dc_protocol::model::{AllowedMentions, CreateMessage};
    let http = MockHttp::new();
    let clock = FakeClock::new();
    http.push(429, &[("Retry-After", "0")], "{}");
    let response = serde_json::from_str::<Vec<dc_protocol::model::Message>>(MESSAGES)
        .unwrap()
        .remove(0);
    http.push(200, &[], &serde_json::to_string(&response).unwrap());
    let message = CreateMessage {
        content: "hello 😀".into(),
        nonce: "12345".into(),
        message_reference: None,
        allowed_mentions: AllowedMentions::default(),
    };
    let result = client(&http, &clock)
        .create_message(CHANNEL, &message)
        .await
        .unwrap();
    assert_eq!(result.id, response.id);
    let requests = http.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, Method::Post);
    assert_eq!(requests[0].body, requests[1].body);
    let body: serde_json::Value =
        serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
    assert_eq!(body["content"], "hello 😀");
    assert_eq!(body["nonce"], "12345");
    assert!(requests[0].authorization.is_some());
    assert!(!format!("{:?}", requests[0]).contains("hello"));
}

#[tokio::test]
async fn invalid_sends_never_touch_transport_and_transport_failures_are_not_retried() {
    use dc_protocol::model::{AllowedMentions, CreateMessage};
    let http = MockHttp::new();
    let clock = FakeClock::new();
    let api = client(&http, &clock);
    let mut message = CreateMessage {
        content: " \n".into(),
        nonce: "12345".into(),
        message_reference: None,
        allowed_mentions: AllowedMentions::default(),
    };
    assert!(api.create_message(CHANNEL, &message).await.is_err());
    message.content = "😀".repeat(1001);
    assert!(api.create_message(CHANNEL, &message).await.is_err());
    assert!(http.requests().is_empty());
    http.push_error("offline");
    message.content = "hello".into();
    assert!(api.create_message(CHANNEL, &message).await.is_err());
    assert_eq!(http.requests().len(), 1);
}

#[tokio::test]
async fn multipart_uploads_keep_json_nonce_and_binary_bytes_and_sanitize_headers() {
    use dc_protocol::model::{AllowedMentions, CreateMessage};
    let http = MockHttp::new();
    let clock = FakeClock::new();
    let response = serde_json::from_str::<Vec<dc_protocol::model::Message>>(MESSAGES)
        .unwrap()
        .remove(0);
    http.push(200, &[], &serde_json::to_string(&response).unwrap());
    let message = CreateMessage {
        content: String::new(),
        nonce: "12345".into(),
        message_reference: None,
        allowed_mentions: AllowedMentions::default(),
    };
    let files = [dc_api::UploadFile {
        filename: "hello\"\r\n.png".into(),
        bytes: vec![0, 255, 1],
    }];
    client(&http, &clock)
        .create_message_with_files(CHANNEL, &message, &files)
        .await
        .unwrap();
    let requests = http.requests();
    let request = &requests[0];
    let content_type = request.content_type.as_deref().unwrap();
    assert!(content_type.starts_with("multipart/form-data; boundary=dc-upload-"));
    let body = request.body.as_ref().unwrap();
    let text = String::from_utf8_lossy(body);
    assert!(text.contains("name=\"files[0]\""));
    assert!(text.contains("hello___.png"));
    assert!(text.contains("\"nonce\":\"12345\""));
    assert!(body.windows(3).any(|part| part == [0, 255, 1]));
}
