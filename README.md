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

This is an **unofficial, early-stage project**, not a daily driver yet.
The owner has verified connecting, browsing channels, and loading history
on Linux. The latest layout and image rendering still need a local visual check.

> Third-party clients violate Discord’s Terms of Service and can put your
> account at risk. This project is not affiliated with or endorsed by Discord.

## 🎨 Make yourself at home

Available in the current build:

- **Find your people:** a server rail, channel list, DMs, and message history.
- **Read comfortably:** grouped messages, reply previews, and readable mentions.
- **A look of your own:** local display names, name colors, profile gradients,
  bios, banners, avatars, and decorations by local file or shop asset hash.
- **Open a profile:** click a message author or your avatar; one-to-one DMs
  also have a profile panel.
- **Pictures with fallbacks:** Discord CDN avatars, server icons, banners,
  and decorations, with initials while downloads are pending or fail.

Cosmetics only change **your view**. They do not update your Discord profile
or grant purchases. See the [cosmetics guide](docs/COSMETICS.md) for the JSON format.

Next, in order: **message sending → emoji picker + FakeNitro-lite → member
list → settings & cosmetics editor**. FakeNitro-lite’s emoji image links are
planned, not implemented. Voice and screen sharing are later milestones.

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
  libvulkan-dev libxcb1-dev libfontconfig-dev
```

A working Vulkan driver is needed to open the Linux window. Windows builds
and tests run in CI; visual behavior on Windows still needs local verification.

Start with [LOCAL_VERIFY §8](docs/LOCAL_VERIFY.md#8-the-window-with-fake-data-no-account-no-network)
for a window walkthrough. For your own account, follow that guide’s token
setup and §9, then run `cargo run -p dc-app`.

**Never put your token in a file, command argument, issue, or screenshot.**
The app currently reads `DC_TOKEN` at runtime; OS keyring storage is planned.
Keep screenshots containing other people’s details out of this public repo.

## 🛠 Under the hood

Small Rust crates keep protocol, networking, state, cosmetics, and rendering
apart. The UI receives state and sends commands; it never receives your token.
CDN downloads are public and unauthenticated. Message times currently use UTC
in 12-hour format; local timezone detection is pending.

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
