<div align="center">

# ✦ rust-discord

**A little Rust. A lot of personality.**

A native Discord client for Windows & Linux, with a cozy dark UI
and local cosmetics to make it feel like yours.

Rust · GPUI · MIT · Early development

[Try the demo](#-take-a-peek) · [Customize your look](docs/COSMETICS.md) · [Project status](docs/PROGRESS.md) · [The plan](blueprint.md)

</div>

## 🌙 A small client with big plans

Built for the owner and a few friends: native desktop rendering, no Electron
or webview, and no telemetry. The layout is our own GPUI implementation,
inspired by the familiar Discord desktop experience.

This is an **unofficial development build**. The owner has verified an earlier
connection/history build on Linux. The new sending, settings, media rendering,
and plugin features are covered by headless tests and still need live testing.
Voice calls and screen sharing are not available yet; their implementation is
tracked in [BUILD_PLAN](docs/BUILD_PLAN.md).

> Third-party clients violate Discord’s Terms of Service and can put your
> account at risk. This project is not affiliated with or endorsed by Discord.

## 🎨 Make yourself at home

Implemented in the current build:

- **Find your people:** server rail, channel navigation, DMs, and member pages.
- **Chat:** multiline drafts per channel, replies, sending, manual retry,
  file attachments, editing/deleting your messages, and older history pages.
- **Read comfortably:** grouped messages, Markdown, reply previews, inline
  Discord images, and search across loaded messages.
- **Pick an emoji:** searchable Unicode/server picker and FakeNitro-lite image
  links. Sending still requires an explicit press of Enter or Send.
- **Make it yours:** local names, colors, gradients, bios, banners, avatars,
  and decorations. Open a profile and choose “Customize this profile locally,”
  or use the bottom-left settings button. [Cosmetics guide](docs/COSMETICS.md).
- **Keep control:** OS keyring credentials, opt-in per-account SQLite history,
  cache erasure, a first-run risk notice, and no telemetry.
- **Small extensions:** explicitly enabled QuickJS outgoing-text transforms,
  with memory/time limits and no filesystem/network bindings.
  [Plugin guide](docs/PLUGINS.md).

Cosmetics only change **your view**. They do not update your Discord profile
or grant purchases. Voice packet/audio foundations exist, but are not wired
as working calls. Capture, Go Live, Rich Presence IPC, and broader plugin
capabilities remain unfinished; this is not the complete blueprint yet.

## ✨ Take a peek

You can explore the window with synthetic data — **no account or network needed**:

```bash
cargo run -p dc-ui --example demo
```

Add a splash of sample cosmetics:

```bash
DC_COSMETICS_DEMO=1 cargo run -p dc-ui --example demo
```

The pinned Rust toolchain is installed by rustup. On Debian/Ubuntu, GPUI’s
build libraries are:

```bash
sudo apt-get install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libvulkan-dev libxcb1-dev libfontconfig-dev libasound2-dev libopus-dev cmake
```

A working Vulkan driver is needed to open the Linux window. Windows builds
and tests run in CI; visual behavior on Windows still needs local verification.

Start with [LOCAL_VERIFY §8](docs/LOCAL_VERIFY.md#8-the-window-with-fake-data-no-account-no-network)
for a window walkthrough. For your own account, follow that guide’s token
setup and §9, then run `cargo run -p dc-app`.

**Never put your token in a file, command argument, issue, or screenshot.**
The app reads `DC_TOKEN` at runtime or loads the OS keyring. With `DC_TOKEN`
set for your shell session, run `dc-client --store-token` (or
`cargo run -p dc-app -- --store-token`) to save it securely, then unset the
environment variable. `--forget-token` removes that stored credential.
There is no plaintext fallback.
Keep screenshots containing other people’s details out of this public repo.

## 🛠 Under the hood

Small Rust crates keep protocol, networking, state, cosmetics, and rendering
apart. The UI receives state and sends commands; it never receives your token.
CDN downloads are public and unauthenticated. The app displays 12-hour message
times using the local UTC offset detected at startup; the offline demo uses UTC.
The new native input is adapted from GPUI’s Apache-2.0 example; see
[third-party notices](docs/THIRD_PARTY.md).

With [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) installed:

```bash
cargo fmt --all -- --check && \
cargo clippy --workspace --all-targets -- -D warnings && \
cargo test --workspace && \
cargo deny check
```

[Blueprint](blueprint.md) · [Protocol references](ref.md) ·
[Progress & known limits](docs/PROGRESS.md) · [Continuation notes](docs/HANDOFF.md)

## ♡ License

[MIT](LICENSE) — use, modify, and redistribute under its terms.
Copyright © 2026 staraepp.
