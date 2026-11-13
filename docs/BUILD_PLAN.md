# Full-app implementation tracker

The owner’s latest instruction is to build the whole app before asking them
to test individual slices. Continue across milestones; do not reintroduce
an approval gate between already authorized features. The blueprint remains
the scope, including its non-goals.

## Implemented and integrated

- Text drafts/sending/replies with stable manual-retry nonces.
- Human-selected multipart attachments (10 files, 25 MiB total), edit/delete
  controls for own messages, history paging, loaded-message search.
- Native editable multiline input, IME plumbing, clipboard and undo/redo.
- Markdown and inline Discord images; Unicode/server emoji picker with
  FakeNitro-lite insertion; Home/Guild selection memory.
- Visible member-page subscriptions through opcode 37, bounded slot reducer,
  role-colored member rows, profile popups and DM profile panel.
- Self/per-user cosmetics editor, atomic saves, appearance preferences.
- OS keyring commands, local time offset, opt-in per-account SQLite cache
  and erasure, first-run risk acceptance before connecting.
- Limited, explicitly granted outgoing QuickJS transforms.
- CI builds/tests Linux and Windows and uploads development binaries.

These are implementation claims, not live/visual verification claims.

## Implemented foundations, not connected as calls

- CPAL device/duplex adapter and Opus encode/decode.
- RTP framing, UDP discovery encoding/validation and bounded jitter buffer.
- In-memory tests cover codec round-trips, reorder/wrap/loss, discovery and
  malformed packets. No microphone/speaker, live voice, DAVE, or capture test
  has been performed here.

## Remaining to finish the blueprint

- Voice WS/session lifecycle and app wiring, audited transport encryption,
  DAVE integration/transition handling, mixing, reconnect and device UI.
- Screen/app-audio capture, video encode/transport and Go Live.
- Rich Presence IPC; remaining event bus/plugin hooks/capabilities/storage.
- Broader text polish: sticker picker, full-profile lookup, live presence,
  thread navigation/virtualization, reactions, shop catalog browser.
- Packaging/release hardening, real platform/device verification and measured
  performance. Do not publish a production release as if these were done.

## Dependency authorization

The owner approved: pulldown-cmark, keyring, rusqlite, chrono, rquickjs,
cpal, audiopus and native libdave. The first seven are integrated. libdave
is not yet linked.

A separate question is pending for chacha20poly1305/aes-gcm, platform capture
libraries (ashpd/pipewire/windows-capture/wasapi/windows), openh264,
unicode-segmentation, and whether to use MIT-licensed Rust `davey` instead
of native libdave. Do not treat an unanswered question as approval. The
native libdave alternative needs its C++/MLS/SSL build integrated; crypto
must never be hand-written to avoid that decision.

No screenshot or account credential belongs in the public repository.
