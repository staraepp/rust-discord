# ref.md — references (verify details; the user API is unofficial and changes)

## Protocol
- Unofficial Discord *user* API docs (gateway, REST, voice for user accounts): https://discord-userdoccers.pages.dev
- Official voice docs (UDP voice, voice gateway v8, DAVE opcodes): https://docs.discord.com/developers/topics/voice-connections
- DAVE protocol whitepaper (MLS-based A/V E2EE, v1.x): https://daveprotocol.com/
- Discord's DAVE blog (libdave + whitepaper released): https://discord.com/blog/meet-dave-e2ee-for-audio-video
- libdave (Discord's C++ reference impl, pure-C API): https://github.com/discord/libdave
- Spacebar voice reverse-engineering (UDP vs WebRTC voice, SFU notes): https://docs.spacebar.chat/contributing/reverse-engineering/voice/
- Discord blog on voice at scale (browser = WebRTC): https://discord.com/blog/how-discord-handles-two-and-half-million-concurrent-voice-users-using-webrtc

## Prior art (read the code; mind licenses)
- Serein — Rust + egui + wgpu native client; voice + DAVE v1, screen share, plugin engine w/ permission prompts. README claims; Linux capture/IME/bidi unverified. MIT or Apache-2.0: https://github.com/ViceVerse-cz/Serein
- Concord — Rust TUI; voice, PTT, noise suppression, screen share via PipeWire/portal + VA-API; serves discord-ipc for Rich Presence: https://github.com/chojs23/concord
- Dissent — Go/GTK4; no voice by design: https://github.com/diamondburned/dissent
- Vencord (understand the concepts; **GPL-3.0, don't copy code**): https://docs.vencord.dev
- venmic (Linux per-app audio for screenshare; REST-server mode): https://github.com/Vencord/venmic

## Crates / tech
- `dave` (pure-Rust DAVE crypto on OpenMLS; **AGPL-3.0-only**, no transport/codec): https://docs.rs/crate/dave/0.2.0
- `openmls` (use this or libdave; never hand-roll MLS)
- `rquickjs` (QuickJS bindings for plugins): https://github.com/DelSkayn/rquickjs/
- `cpal` (audio device IO), Opus bindings, RNNoise
- Linux capture: `ashpd` (portals), `pipewire`
- Windows capture: `windows-capture`, `wgc`; encode via Media Foundation (`windows` crate); cross-platform reference: `moq-video` (capture/encode backends matrix)
- Windows per-app audio: WASAPI process loopback — https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/ ; Rust `wasapi` crate (`new_application_loopback_client`)
- UI candidates: Slint https://slint.dev/ · GPUI + https://github.com/longbridge/gpui-component

## Claude Code on the web
- Cloud environments (network access levels, env vars, setup scripts; no dedicated secret store): https://code.claude.com/docs/en/cloud-environments
- Web sessions, `--teleport` / `--remote`: https://code.claude.com/docs/en/claude-code-on-the-web
