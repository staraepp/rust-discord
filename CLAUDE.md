# CLAUDE.md — native Discord client (working title: (TBD)

Read `blueprint.md` (the plan) and `ref.md` (protocol + library references) before starting any milestone.

## What we're building
A fast, native Discord desktop client in Rust (no webview, no Electron) for **Windows and Linux**:
- text chat, voice (UDP voice gateway + DAVE E2EE), later Go Live screen share
- a plugin system (Vencord-*inspired* features, built natively; Vencord itself cannot run here)
- local-only cosmetic overlays (profile color / banner / avatar decoration) and "FakeNitro-lite" (client-side tricks only)

Personal project for the owner and a few friends. Not a product.

## Hard rules (never break, even if a task seems to ask)
1. **Secrets.** Never read, print, log, commit, or ask for a Discord token. Tokens exist only in the OS keyring or a local env var on the owner's machine at runtime. No tokens in fixtures, tests, logs, panics, crash reports, or example configs. Wrap secrets in a `Secret<T>` newtype whose Debug/Display redact. Redact `Authorization` headers everywhere.
2. **No evasion or automation features.** Do not build anti-detection, fingerprint spoofing, CAPTCHA solving, rate-limit evasion, self-bot automation, or mass-action features. Follow the unofficial user-API docs in `ref.md` for required payloads, and respect every rate limit, bucket header, and `Retry-After`. The client must behave like a normal human-driven client.
3. **No telemetry.** Only talk to Discord and (later, optionally) the owner's own cosmetics-sync server.
4. **Licenses.** `cargo-deny` runs in CI. Do not add AGPL/GPL dependencies without asking. Do not copy code from Vencord (GPL-3.0); reimplementing *behavior* from docs is fine. The `dave` crate on crates.io is AGPL-3.0-only: do not add it without explicit approval.
5. **Crypto.** Never hand-roll crypto or MLS. Use audited crates (openmls, RustCrypto, ring) or Discord's libdave via FFI.
6. **`unsafe`** only inside `dc-platform-*` / `dc-ffi-*` crates, each block with a `// SAFETY:` comment.

## Environment reality (important)
You normally run in a cloud sandbox: restricted network, no display, no audio devices, no GPU, Linux only. So:
- You **cannot** connect to Discord, test audio, capture a screen, or look at the GUI. Never claim those work.
- Put network, audio, clock, capture, and filesystem behind traits and provide in-memory fakes so logic is testable headless.
- For anything needing the real world, write the code plus exact steps in `docs/LOCAL_VERIFY.md` (commands to run locally + expected output) and label it **UNVERIFIED** in `docs/PROGRESS.md`.
- Windows-only code lives behind `cfg(windows)`. Compile-check if you can; otherwise flag as unverified.
- Unknown protocol detail? Don't guess silently. Mark `// ASSUMPTION:` in code and list it in `docs/PROGRESS.md`.

## Definition of done
All of these pass:
`cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`
New logic has tests (unit + fixture/replay where applicable). End every session by updating `docs/PROGRESS.md` (done / unverified / assumptions / next).

## Conventions
- Rust stable, pinned in `rust-toolchain.toml`. Async: tokio. Errors: `thiserror` in libs, `anyhow` only in binaries. Logging: `tracing`.
- Workspace crates prefixed `dc-` (rename when the project gets a name). Small crates, clear boundaries (see blueprint §4).
- UI is a pure function of store state + overlay. No business logic in UI crates.
- Small commits. One milestone item at a time. Don't start the next milestone until the owner says so.
- Ask before: adding a heavy dependency, changing a crate boundary, or deviating from the blueprint.
