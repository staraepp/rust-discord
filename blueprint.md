# Blueprint — native Discord client (working title TBD)

## 1. Goals
- Fast and light: native Rust, no webview. Measure RAM/CPU/startup against the official client; don't promise numbers up front.
- Windows + Linux first-class (Linux: X11 and Wayland; x86_64 first, aarch64 later).
- Voice that just works on Linux, without Chromium/WebRTC quirks.
- Native plugin system (see §5).
- Local-only cosmetic overlays + FakeNitro-lite.
- Daily-drivable for one owner + a few friends.

## 2. Non-goals (for now)
- Running Vencord/BetterDiscord JS plugins or CSS themes (impossible without Discord's JS bundle; Vencord works by regex-patching it).
- Stage channels, forum/thread polish, polls, automod UI, bot/app-command UI, mobile, macOS.
- Anything resembling automation, spoofing, or evasion.
- Public distribution.

## 3. Key technical decisions
| Topic | Decision | Notes |
|---|---|---|
| Language | Rust | Ecosystem (tokio, wgpu, cpal, openmls, pipewire) + existing Rust prior art (Serein, Concord). |
| Voice transport | Native **UDP voice path**, not libwebrtc | Desktop clients/bots use UDP voice; browsers use WebRTC. UDP path has no ICE/DTLS. Verify against official voice docs in `ref.md`. |
| E2EE | **DAVE is mandatory** for calls/voice channels/Go Live (not stage) since 2026-03-01 | Choose in M3 spike: libdave via FFI (reference impl) vs own implementation on `openmls`. Check licenses first. |
| "WebRTC IP policy" (Vesktop setting) | Replace with "bind voice UDP socket to interface/IP" option | No ICE means VPN (e.g. Tailscale) issues become a socket-binding choice. |
| UI | Bake-off in M2: **Slint vs GPUI(+gpui-component)** | Both list Windows + Linux. Criteria in M2. egui rejected for polished/translucent look + IME/bidi doubts. |
| Plugins | Tier A in-tree Rust modules; Tier B **QuickJS (rquickjs)** with capability permissions | WASM (wasmtime/Extism) possible later for stronger isolation. No native .so/.dll plugin loading. |
| Storage | SQLite (rusqlite) cache; tokens in OS keyring only | No plaintext fallback. |

## 4. Architecture (Cargo workspace)
| Crate | Responsibility |
|---|---|
| `dc-protocol` | serde types: gateway opcodes/events, REST models. Pure, no IO. |
| `dc-gateway` | WebSocket client: heartbeat, resume, reconnect, compression, event stream. IO behind a transport trait. |
| `dc-api` | REST client + rate-limit buckets (honor Retry-After). |
| `dc-store` | Normalized in-memory state + SQLite persistence. Flux-like stores. |
| `dc-core` | Event bus, `LocalOverlay`, plugin traits, app state machine. |
| `dc-voice` | Voice WS (v8), UDP, IP discovery, transport crypto, RTP, Opus, jitter buffer. |
| `dc-dave` | DAVE wrapper (libdave FFI or openmls-based). |
| `dc-audio` | Device IO (cpal), RNNoise/AEC, mixer, PTT logic. Device IO behind traits. |
| `dc-capture` | Screen/app-audio capture + encode (platform backends). |
| `dc-plugin-host` | QuickJS runtime, manifest, capabilities, hot reload. |
| `dc-platform-win` / `dc-platform-linux` | keyring, hotkeys, IPC pipe/socket, notifications. |
| `dc-ui` | Toolkit shell (chosen in M2). Pure view over store + overlay. |
| `dc-app` | Binary, wiring, config. `dc-cli` = headless dev tools. |

Rule: **UI = f(store, overlay)**. Plugins and cosmetics never mutate server data; they transform view-models.

## 5. Plugin design
Vencord can patch anything because Discord is JS. Native plugins can only touch what we expose, so the extension points are the product:
1. **Events** (read-only): message create/update/delete, voice state, presence, etc.
2. **Transforms** (pipeline hooks): outgoing message, incoming message render, user/channel view-model overlay.
3. **UI slots** (declarative): message toolbar buttons, context-menu items, settings pages, sidebar panels. Plugins return a UI description; the host renders it.
4. **Commands, settings schema, per-plugin storage.**
5. **Capabilities** (prompted, per plugin): network (allowlisted hosts), send-message, clipboard, filesystem-scoped. **Plugins never receive the token.** The host owns all Discord networking.

Tiers: A = compiled-in Rust modules (first-party features, dogfood the API). B = QuickJS scripts (TS-friendly) with a versioned host API; WASM later if needed.

First built-ins (as Tier A plugins): **FakeNitroLite**, **LocalCosmetics**, **MessageLogger** (local only).

### LocalOverlay (cosmetics)
Between store and renderer: `view_user(User) -> User'` applying local-only profile accent color, banner, avatar decoration (from the shop catalog assets; verify endpoints in userdoccers). Nobody else sees it. Optional later: friends-only sync via the owner's VPS.

### FakeNitroLite (client-side only)
- Outgoing: custom emoji/stickers the account can't use → image links (configurable size).
- Incoming: recognize those links → render as emoji/sticker.
- **Excluded for now:** any stream-quality/premium-gating bypass. Unverified whether servers honor it; decide later, opt-in only.

## 6. Platform matrix
| Concern | Windows | Linux |
|---|---|---|
| Audio device IO | cpal (WASAPI) | cpal (ALSA/PipeWire) — verify PipeWire path |
| App-audio capture for streams | WASAPI process loopback (Win10 2004+) via `wasapi` crate | venmic sidecar (REST server mode) → later `pipewire` crate |
| Screen capture | Windows.Graphics.Capture (`windows-capture` / `wgc`) | xdg-desktop-portal ScreenCast + PipeWire (`ashpd`, `pipewire`) |
| H.264 encode | Media Foundation; NVIDIA may not expose a HW MFT → NVENC direct or openh264 fallback | VA-API; openh264 fallback |
| Global PTT | Win32 hotkey / low-level hook | X11 grab; Wayland: portal GlobalShortcuts (verify per compositor) |
| Rich Presence IPC | named pipe (verify name) | unix socket `discord-ipc-N` |
| Secrets | Credential Manager | Secret Service / libsecret |
| Packaging | signed installer + auto-update | Flatpak and/or AppImage |

## 7. Milestones
"Cloud" = verifiable by the agent headlessly. "Local" = owner verifies on real machine/accounts via `docs/LOCAL_VERIFY.md`.

**M0 — Protocol spike (headless)**
Crates: `dc-protocol`, `dc-gateway`, `dc-api`, `dc-cli`.
Done when: gateway connect/heartbeat/resume/reconnect state machine works against a mock transport; READY and message events parse into typed models from sanitized fixtures; REST client with rate-limit buckets; `dc-cli` prints live messages for one channel.
Cloud: fixture-parsing tests, state-machine tests, rate-limit tests. Local: live run.

**M1 — Store, overlay, plugin API v0 (headless)**
Crates: `dc-store`, `dc-core`. Normalized store, event bus, `LocalOverlay`, plugin traits, built-ins: FakeNitroLite, LocalCosmetics, MessageLogger.
Cloud: everything (pure logic + tests).

**M2 — UI bake-off + text MVP**
Build the *same* screen in Slint and GPUI: virtualized variable-height message list + composer with mentions/emoji. Criteria: scroll smoothness with 10k messages, text input/IME, Windows + Linux (X11/Wayland) behavior, theming (rounded/translucent), licensing, build time. Pick one, then text MVP: server/channel tree, messages, markdown, images, send.
Cloud: view-model/layout tests. Local: all visual/feel judgment.

**M3 — Voice (headless core first)**
Voice WS v8, UDP IP discovery, transport crypto, RTP, Opus, jitter buffer, DAVE. CLI `dc-voice-cli` joins and echoes. Then device IO (cpal), PTT, RNNoise.
Cloud: crypto round-trips, RTP packetization, jitter-buffer simulation (loss/reorder), DAVE state machine vs test vectors if available. Local: live voice, audio devices, Linux + Windows.

**M4 — Plugin runtime**
QuickJS host, manifest, capability prompts, hot reload, a few ported-in-spirit plugins. Cloud: host tests; sandbox-escape tests.

**M5 — Go Live + Rich Presence**
Per §6 matrix. Local-only verification (capture, encode, per-app audio). Rich Presence socket/pipe.

**M6 — Friends release**
Signed Windows installer, Linux packaging, auto-update, first-run risk notice, plugin permission UI, crash handling that never captures tokens, optional cosmetics-sync service on the VPS.

## 8. Risks
- **ToS:** third-party clients violate Discord's ToS and can get accounts banned. Friends running this are risking *their* accounts and must be told plainly (first-run notice + README).
- **Unofficial API:** user API can change without notice; DAVE will evolve. Keep protocol code isolated + fixture-tested.
- **Scope creep:** the target is "daily driver for a friend server", not feature parity.
- **Licenses:** Vencord GPL-3.0 (don't copy), `dave` crate AGPL-3.0-only, Slint GPLv3/royalty-free terms, check every new dep with cargo-deny.
- **Token safety:** a leaked token = account takeover. Keyring only, no plaintext fallback, redaction everywhere, plugins never see it.
