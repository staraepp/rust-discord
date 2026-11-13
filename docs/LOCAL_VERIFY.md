# LOCAL_VERIFY — things only the owner can check

The cloud sandbox cannot reach Discord, so everything below is **UNVERIFIED**
until you run it on your own machine. Results go back into
`docs/PROGRESS.md`.

> **Heads-up (blueprint §8):** using a third-party client breaks Discord's ToS
> and can get the account restricted or banned. This client identifies itself
> honestly (it does not pretend to be the official app), which may make that
> *more* likely, not less. Consider testing with an account you can afford to
> lose first.

## 0. Token handling rules

- The token is read **only** from the `DC_TOKEN` environment variable.
- Set it in the current shell session only. Do not put it in a file, a
  `.env`, your shell rc, a command-line argument, or a chat with Claude.
- **Never paste the token back to me**, and check anything you paste for it
  first. `dc-cli` never prints it, but your shell history might: the commands
  below avoid that.

## 1. Build and offline checks (no account needed)

```bash
cargo xtask ci
```

Expected: `fmt`, `clippy`, `test`, `deny` all run and the command exits 0
(`advisories ok, bans ok, licenses ok, sources ok` at the end). Needs
`cargo install cargo-deny --locked` once.

```bash
cargo run -q -p dc-cli -- help
```

Expected: the usage text (commands `guilds`, `watch`, `history`).

```bash
env -u DC_TOKEN cargo run -q -p dc-cli -- guilds
```

Expected: `error: DC_TOKEN is not set. ...` and exit code 1. No network
request is made.

## 2. Set the token without leaving it in shell history

Linux (bash/zsh) — prompts without echo, nothing lands in history:

```bash
read -rs DC_TOKEN && export DC_TOKEN
```

Windows PowerShell:

```powershell
$env:DC_TOKEN = Read-Host -MaskInput "token"
```

When finished: `unset DC_TOKEN` (bash) / `Remove-Item Env:DC_TOKEN`
(PowerShell), or just close the terminal.

## 3. List guilds and channels (gateway: connect → Hello → Identify → READY)

```bash
cargo run -q -p dc-cli -- guilds
```

Expected on stderr: `-- connecting to the gateway...`

Expected on stdout, within a few seconds, something shaped like:

```text
Logged in as <your display name> (N guilds, M private channels)

<guild id>  <guild name>
    <channel id>         category  Text Channels
    <channel id>         text      general
    <channel id>         voice     Voice
    ...

Direct messages
    <channel id>         dm        <friend name>
```

Then the program exits with code 0. Pick a text channel ID for the next steps.

## 4. Fetch history over REST

```bash
cargo run -q -p dc-cli -- history <channel_id> 10
```

Expected: up to 10 lines, oldest first, like
`[2026-01-02 03:04:05] Name: message text`, then `-- 10 message(s)` on stderr.
Compare against the official client.

## 5. Watch a channel live

```bash
cargo run -q -p dc-cli -- watch <channel_id>
```

Expected on stderr: `-- connected as <name> (N guilds); watching channel <id>`.

1. Send a message in that channel from another device/account. It should
   appear within about a second as `[timestamp] Name: text`.
2. Messages in *other* channels must not appear.
3. Leave it running for at least 3 minutes (several heartbeats, ~41 s apart).
   It must stay connected with no `-- disconnected` lines.
4. Press Ctrl-C. Expected: `-- interrupted, closing`, exit within ~3 s.

## 6. Reconnect / resume

While `watch` is running:

1. Disable the network (Wi-Fi off / unplug) for ~60–90 seconds.
2. Expected: `-- disconnected; reconnecting in X.Xs` lines with growing delays
   (about 1 s, 2 s, 4 s … capped at 60 s, each scaled by 50–100 % jitter).
3. Re-enable the network. Expected: it reconnects on its own. Either
   `-- session resumed` (the session was resumed and missed messages are
   replayed), or a second `-- connected as ...` line (the resume connection
   failed first, so it re-identified — also correct, see assumption A9).
4. Send a message again; it should appear.

## 7. Heartbeat detail (optional)

```bash
DC_LOG=trace cargo run -q -p dc-cli -- watch <channel_id>
```

Expected: `gateway send op=Identify` once, then `gateway send op=Heartbeat`
roughly every 41 s (first one after a random fraction of that). Trace logs
contain opcodes only — no payloads, no token.

## If something fails — what to paste back

Please send:

1. The exact command (without the token!).
2. Everything printed on stderr, re-run with `DC_LOG=debug` (debug logs
   contain event names, close codes and error text, never payloads or the
   token).
3. OS + `rustc --version`.
4. For gateway failures: the close code if one is shown
   (e.g. `close code 4004`).
5. For a parse failure (`dropping undecodable event` / `could not decode
   response body`): the event name and serde error message from the log are
   usually enough. **Do not paste raw READY payloads** — they contain your
   email, phone, session IDs and analytics tokens.

Likely failure modes and what they would mean:

| Symptom | Probable cause | Which assumption |
| --- | --- | --- |
| `authentication failed (close code 4004)` | token wrong/expired, or copied with quotes/whitespace | — |
| HTTP 403 / HTML body / `http 4xx` on `history` or on fetching the gateway URL | Cloudflare or Discord rejecting the honest User-Agent | A2 |
| Connects, then closes with 4001/4002 right after Identify | Identify payload missing something the gateway insists on | A1, A3 |
| READY arrives but `guilds` prints `?` IDs or no channels | READY shape differs from the fixture | A1, A4 |
| `dropping undecodable event event="READY"` | a required READY field is absent/renamed | A1 |
| Captcha / "verify your account" prompts in the official client afterwards | anti-abuse flagged the session | A2 (stop and tell me) |
