# PROGRESS

Last updated: 2026-11-13 — end of **M0 (protocol spike, headless)**.

## Done (verified in the sandbox)

`cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`
passes (also available as `cargo xtask ci` / `just ci`).

- **Workspace**: `rust-toolchain.toml` (1.97.0), `deny.toml` (allow-list of
  permissive licenses only → every GPL/AGPL/LGPL is denied; `dave`,
  `openssl-sys`, `native-tls` banned explicitly), `xtask` + `justfile`,
  GitHub Actions workflow (Linux full checks, Windows build + test),
  `#![forbid(unsafe_code)]` workspace-wide.
- **`dc-protocol`** (pure): all 44 gateway opcodes; receive envelope
  (`GatewayReceive`), typed dispatches (`READY`, `RESUMED`, `MESSAGE_CREATE`,
  `MESSAGE_UPDATE`, `MESSAGE_DELETE`, `GUILD_CREATE`, passthrough `Unknown`);
  send payloads (Heartbeat, Identify, Resume); close-code classification;
  models `User`, `Channel`, `ChannelType`, `Guild`, `Message`, `Attachment`,
  `Ready`; `Snowflake`; `Secret<T>` (redacting Debug/Display, zeroize on
  drop, no `Serialize`); `Clock` trait + `FakeClock`.
- **`dc-gateway`**: sans-IO `Session` state machine (hello timeout, jittered
  first heartbeat, heartbeat schedule, heartbeat requests, zombie detection,
  identify vs resume, opcode 7, opcode 9, close codes, exponential backoff
  with jitter and cap); async `GatewayClient` driver over
  `Connector`/`Transport`/`Clock`; in-memory `MockConnector`; real
  `WsConnector` (tokio-tungstenite + rustls, `wss://` only).
- **`dc-api`**: `ApiClient` over `HttpTransport`/`Clock`; `RateLimiter` with
  per-route + major-parameter buckets, `X-RateLimit-Bucket` mapping, global
  limit, `Retry-After` (header, then JSON body); no retry without
  `Retry-After`; bounded retries; latches on 401 and stops sending;
  `get_gateway`, `get_messages`; `MockHttp`; real `ReqwestTransport`
  (rustls, https only, `Authorization` marked sensitive).
- **`dc-cli`**: `guilds`, `watch <channel_id>`, `history <channel_id> [N]`;
  token from `DC_TOKEN` only (token-looking arguments are refused); terminal
  output sanitized against escape-sequence injection.
- **Tests** (93): fixture parser tests, session state-machine tests, driver
  tests on mock transport + fake clock (no real sleeping), rate-limiter and
  API-client tests, redaction tests in every crate that touches the token.
  All fixtures are synthetic (`fixtures/README.md`).
- **CI**: the GitHub Actions workflow ran on the M0 pull request; both the
  Linux job (`cargo xtask ci`) and the Windows job (`cargo test --workspace`)
  passed.

## UNVERIFIED (needs the owner, see `docs/LOCAL_VERIFY.md`)

Nothing below has ever talked to Discord.

- U1. Real WebSocket transport (`dc-gateway::ws`): TLS connect, frame
  handling, close codes. → LOCAL_VERIFY §3, §5
- U2. Real HTTPS transport (`dc-api::http::real`). → §3, §4
- U3. Live Identify is accepted and READY parses with real data. → §3
- U4. Live `MESSAGE_CREATE` parsing and channel filtering. → §5
- U5. Heartbeat cadence keeps a real session alive for minutes. → §5, §7
- U6. Resume / re-identify after a real network drop. → §6
- U7. Real rate-limit headers from user-token requests match the parser.

## Assumptions (docs did not pin these down)

Each is marked `// ASSUMPTION:` in code unless noted.

- A1. **Identify without `capabilities` / `client_state`** is accepted for a
  user account and yields the plain READY shape (guild fields merged into
  the guild object, inline `recipients` on DMs). The parser also accepts the
  `CLIENT_STATE_V2` shape (`properties`) as a safety net.
  (`dc-protocol/src/gateway.rs`)
- A2. **Honest identity**: `properties = {os, browser: "dc-client", device:
  "dc-client"}` and `User-Agent: dc-client/<version> (...)`, no
  `X-Super-Properties`. The docs say no property field is truly required but
  that the header matters to anti-abuse systems. Imitating the official
  client's properties/build number is fingerprint spoofing (hard rule 2), so
  it is not done. Risk: Discord/Cloudflare may reject or flag such requests.
  (`dc-protocol/src/gateway.rs`, `dc-api/src/http.rs`)
- A3. Initial presence in Identify is `status: "unknown"`, no activities, as
  the docs recommend for a first identify. Effect on how the account's
  status appears to others is unknown.
- A4. With `CLIENT_STATE_V2` the guild `id` is at the top level and/or in
  `properties`; we read either. (`dc-protocol/src/model.rs`)
- A5. Close-code handling where the user docs give no reconnect table:
  4004, 4010–4014 fatal; **4015 (too many sessions) and 4016 fatal**; 4003,
  4006, 4007, 4009 → fresh Identify; **server-sent 1000/1001 → fresh
  Identify**; everything else (4000–4002, 4005, 4008, 1006, none) → Resume.
  (`dc-protocol/src/close.rs`)
- A6. Opcode 9 (`d: false`): close with 1000, wait 1–5 s (official bot docs),
  reconnect to the base URL and Identify, rather than re-identifying on the
  same socket (which the user docs allow). (`dc-gateway/src/session.rs`)
- A7. Only `X-RateLimit-Reset-After` (relative) is used, not the absolute
  `X-RateLimit-Reset` epoch. (`dc-api/src/ratelimit.rs`)
- A8. No client-side pre-emptive global limit (docs: 50 req/s). A human-driven
  CLI is far below it; the limiter reacts to a global 429. Not marked in
  code. Revisit when the UI can fan out requests.
- A9. If the *connection attempt* for a resume fails, the session is dropped
  and the next attempt identifies from scratch, as the docs' "Handling a
  Disconnect" section literally says. A short network blip can therefore
  cost a full re-identify. Not marked `ASSUMPTION` (it follows the docs) but
  worth a look after LOCAL_VERIFY §6.
- A10. The 30 s Hello timeout and the 1 s → 60 s backoff curve are our own
  choices; the docs give no numbers.

## Decisions for the owner to confirm

- D1. (Owner left this to me: keeping it as is for now.)
  **`Secret<T>` and the `Clock` trait live in `dc-protocol`**, because
  both `dc-gateway` and `dc-api` need them and neither should depend on the
  other. `dc-api` re-exports `Secret`. This stretches "serde types, pure, no
  IO" a little (both are still pure). Alternative: a tiny `dc-util` crate —
  a crate-boundary change, so not done without asking.
- D2. **No spoofing of the official client** (A2). Owner's answer
  (2026-11-13): the rule may be set aside *if it turns out to be required
  for the client to work*. That is not established: nothing has been tested
  live, so the client stays honest. Revisit only with evidence from
  LOCAL_VERIFY (the A2 rows of its failure table), and decide then what the
  smallest truthful change would be.
- D3. Gateway **transport compression is not implemented** (blueprint §4
  lists it for `dc-gateway`). Plain JSON is simpler to verify first; READY
  will be a few MB uncompressed. Proposed for early M1 or once U3 passes.
- D4. `cargo xtask` is the task runner (needs nothing installed); the
  `justfile` only forwards to it.
- D5. New dependencies (all MIT/Apache-2.0/ISC/BSD family, checked by
  cargo-deny): `tokio`, `tokio-tungstenite`, `reqwest` (rustls + ring, no
  OpenSSL), `serde`, `serde_json`, `thiserror`, `anyhow`, `tracing`,
  `tracing-subscriber`, `zeroize`, `fastrand`, `futures-util`.
  `reqwest` + the TLS stack are the heavy ones.
- D6. Timestamps are kept as strings (no `chrono`/`time` yet).
- D7. Workspace crates are licensed MIT to match the repo `LICENSE`.

## Next (not started — waiting for the owner's go)

1. Owner runs `docs/LOCAL_VERIFY.md`; fix whatever U1–U7 turn up; add
   reduced, sanitized fixtures for any payload that surprised the parser.
2. Then M1: `dc-store`, `dc-core` (store, event bus, `LocalOverlay`, plugin
   traits, built-ins).
3. Carry-overs: gateway compression (D3), QoS heartbeat / opcode 41 (docs:
   recommended, not required), keyring-backed token storage
   (`dc-platform-*`, replaces `DC_TOKEN`).
