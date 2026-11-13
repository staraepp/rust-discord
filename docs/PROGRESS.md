# PROGRESS

Last updated: 2026-11-13 — interactive text client, local settings and limited
plugins are integrated; audio/voice packet foundations are implemented.
**The full app requested by the owner is not finished.** See BUILD_PLAN for
remaining voice, DAVE, capture, Rich Presence and broader plugin/text work.

## Verified headlessly on Linux

`cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`
passes: **148 tests**, one existing ignored mock-transport doctest. Also built
`dc-app`/`dc-cli` and ran `dc-client --help` / `--version` successfully.
No Discord credential, live account or private screenshot was used.
Duplicate-version warnings in the existing dependency tree remain permitted.

Implemented and integrated:

- REST text create/edit/delete, JSON and multipart bodies, stable nonces and
  Retry-After handling. Ambiguous transport failures do not auto-resend.
  Requests redact credentials and body content; the real transport disables
  redirects and bounds response bodies to 32 MiB. Multipart content-type and
  sensitive authorization are tested at the real request-builder boundary.
- Native multiline editor adapted from GPUI’s Apache-2.0 example: clipboard,
  selection, undo/redo, UTF-16 IME ranges, Enter/Shift+Enter. Drafts are per
  channel and remain intact on failure; a manual retry retains its nonce.
- Sending/replies, native file selection, bounded attachment reads off the
  network thread, own-message edit/delete controls with confirmation, manual
  history paging, loaded-message search, Home/Guild selection memory, and
  scroll-to-latest. The store keeps its 500-message/channel bound.
- Native Markdown, reply previews, grouped messages, inline CDN images, and
  searchable Unicode/server emoji with opt-in/out FakeNitro-lite link insertion.
- Visible member-page subscriptions and bounded SYNC/INSERT/UPDATE/DELETE/
  INVALIDATE reducer; group headings, role colors and profile actions. Member
  pages are not a full-server fetch. Additional channel/user/delete dispatches
  update the store without dropping partial channel fields.
- Self/per-user cosmetics editor, atomic JSON replacement, appearance settings,
  cache erasure and local startup time offset. Blank fields remove overrides.
  Appearance preferences are separate from the cosmetics format.
- OS keyring load/store/forget, no plaintext fallback. Token environment input
  remains supported. First-run risk acceptance happens before connecting.
- Opt-in account-scoped SQLite history (credentials/READY/unknown message fields
  are not stored). Live create/delete events update the cache; partial edits
  still need fuller cache reconciliation.
- Limited QuickJS outgoing-text transforms: explicit per-package grant, no IO
  bindings or automatic-send API, memory/stack/time limits, failure preserves
  the draft. Installation and limitations: `docs/PLUGINS.md`.
- Offline demo simulates sends, a first-send failure flag, member pages, custom
  emoji entries and in-memory cosmetics. It never downloads images or writes
  configuration. Edit/delete demo simulation remains limited.
- CI now builds development binaries/artifacts for Linux/Windows, including
  license texts collected with `cargo xtask licenses`. Arch build
  dependencies and credential-free `--help` smoke tests are updated.

Implemented foundations **not wired as working calls**:

- `dc-audio`: CPAL device/duplex adapter behind a device trait, bounded callback
  queues, mute/deafen flags, Opus encoder/decoder. Only in-memory codec tests ran.
- `dc-voice`: RTP framing, UDP discovery request/response validation and bounded
  jitter buffer with sequence wrap/reordering/loss tests. Crypto/voice WS/DAVE
  and app call controls are not implemented. Raw VOICE_SERVER_UPDATE payloads
  are filtered before UI delivery so voice credentials do not enter the UI.

## Earlier owner verification (2026-11-13)

The earlier read-only client connected on Linux, listed categorized channels,
loaded history with authors/times/wrapping, and showed `Connected as <name>`.
That establishes the earlier connection/history path, not the new features.

## UNVERIFIED / remaining

- U4–U7: live MESSAGE_CREATE, sustained heartbeat, resume/reconnect and real
  rate-limit headers. LOCAL_VERIFY §3–§7.
- U10–U12: latest visual/input behavior on Linux X11/Wayland and Windows;
  all new REST sends/replies/uploads/edits/deletes, member subscriptions,
  CDN rendering, settings and keyring behavior on a real desktop. §8–§10.
- Audio devices, actual mixing/playout, voice WS, transport crypto, DAVE,
  PTT/noise suppression, capture/video/Go Live, Rich Presence IPC: unfinished.
- Remaining event bus/plugin APIs, incoming hooks, UI slots, capability-backed
  IO and per-plugin storage/hot reload: unfinished. The limited text-transform
  host does not implement all of M4.
- Sticker picker, reactions, full-profile fetch, live presence, thread tree,
  variable-height message virtualization and shop catalog browser: pending.
- No measured performance, production release, or real hardware/media claim.

## Scope and dependency authorization

The owner now asks to build the whole app first, superseding earlier pauses
for visual review and individual milestone approval. Keep working within the
blueprint’s non-goals and existing privacy/licensing rules.

Approved dependencies: pulldown-cmark, keyring, rusqlite, chrono, rquickjs,
cpal, audiopus, native libdave. Integrated dependencies pass cargo-deny.
The separate media/capture/Unicode dependency question and optional switch
from libdave to `davey` are pending; see BUILD_PLAN. Do not infer approval.

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

- A11. GPUI is used from crates.io at 0.2.2, the newest published release.
  It needs `libc < 0.2.190` on Linux (one of its dependencies no longer
  compiles otherwise), so `dc-ui` pins `libc = "=0.2.189"`. That holds the
  whole workspace's libc back until a newer GPUI is released; the
  alternative is depending on GPUI from Zed's git repository.

## Decisions for the owner to confirm

- D1. (Owner left this to me: keeping it as is for now.)
  **`Secret<T>` and the `Clock` trait live in `dc-protocol`**, because
  both `dc-gateway` and `dc-api` need them and neither should depend on the
  other. `dc-api` re-exports `Secret`. This stretches "serde types, pure, no
  IO" a little (both are still pure). Alternative: a tiny `dc-util` crate —
  a crate-boundary change, so not done without asking.
- D2. **Keep the honest client identity** (A2). The owner’s earlier live
  connection and history check succeeded without spoofing, as recorded in
  HANDOFF. There is no evidence that changing the identity is needed.
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
- D6. Wire timestamps remain strings. The app now uses approved `chrono`
  to detect its local UTC offset at startup; historical DST transitions
  within the message list are not calculated per message.
- D7. Workspace crates are licensed MIT to match the repo `LICENSE`.

- D8. `deny.toml` changes forced by GPUI's dependency tree: licenses `Zlib`,
  `NCSA`, `CC0-1.0` and `MPL-2.0` are now allowed (none is GPL/AGPL/LGPL;
  MPL-2.0 is file-level copyleft), and "unmaintained" advisories are only
  enforced for direct dependencies. Vulnerability and unsoundness advisories
  are still denied everywhere.
- D9. GPUI is a heavy dependency: roughly 500 extra crates, a debug binary of
  several hundred MB, and multi-minute clean builds.


- A12. Opcode 37’s lazy member subscription shape and two-stage guild/page
  initialization follow observed behavior in Serein’s MIT-licensed client.
  Commands wait for READY and are spaced by at least 750 ms. The initial
  page is 0–99, and paging is explicit and bounded to 2000 slots. Verify live.
- A13. Local time uses the current offset at startup, not a per-date timezone
  conversion. Long-running sessions and old messages spanning DST may differ.
- A14. FakeNitro-lite conservatively inserts selected custom emoji as links
  when enabled; turning it off uses ordinary markup subject to server/account
  permissions. There is no entitlement bypass or spoofing.
- A15. Attachments use the documented multipart message path. Cloud pre-upload
  URLs are not implemented. User-account acceptance and the account’s actual
  upload limit need live verification; the client caps 10 files/25 MiB total.

## Continuation

Continue the full-app work from BUILD_PLAN. Do not ask the owner to test a
partial window as a prerequisite for implementing the rest. Resolve the
pending media-library choice before adding those dependencies; do not
hand-roll crypto or present unfinished call/share controls as working.
