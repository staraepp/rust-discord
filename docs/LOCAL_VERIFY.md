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

### Arch Linux: installed package instead of `cargo run`

If you installed the `dc-cli` package (release asset, or `makepkg -si` in
`packaging/arch/`), replace `cargo run -q -p dc-cli --` with `dc-cli` in every
command below, e.g. `dc-cli guilds`.

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

## 8. The window, with fake data (no account, no network)

Needs a working Vulkan driver. On Arch, build dependencies are
`libxkbcommon libxkbcommon-x11 wayland libxcb fontconfig vulkan-icd-loader`.

```bash
cargo run -p dc-ui --example demo
```

Expected: a dark window titled `dc-client`, about 1280×760.

1. **Home:** a 72px server rail with `DM`, `FF` (Fake Friends Server), and
   a dimmed unavailable server. A separate channel column lists `Fake Friend`
   first, then `Fake Friend, other_fake` with `3 Members`. The user bar at
   its bottom shows `Fake Owner`. Initials stand in for remote images: this
   demo does not download anything.
2. **Guild:** click `FF`. The column shows `Voice`, a dimmed
   `channel-type-from-the-future`, `TEXT CHANNELS`, and `general`.
   Click `general`: the title becomes `# general`, its row highlights, and
   three messages appear oldest first, ending with `newest message`.
   Times use **UTC in 12-hour format**, not your computer’s timezone yet.
   Attachments appear as filenames; the composer is a placeholder.
3. **Profiles:** click a message author’s avatar or name. A profile card
   opens over the window; click outside it to dismiss. Click your avatar
   in the user bar to open your own card.
4. **DM panel:** click `DM`, then `Fake Friend`. A profile panel appears on
   the right. The group DM has no one-person profile panel. Demo history
   comes from the same synthetic fixture for each selection; it is not
   representative DM content.

Then with sample cosmetics:

```bash
DC_COSMETICS_DEMO=1 cargo run -p dc-ui --example demo
```

Expected differences: your user bar has a pink/plum gradient and the name
`me but cooler` in yellow. Your popup includes `A local-only demo profile.`
and a matching card gradient. The friend’s message name and one-to-one DM
label read `bestie`; message names are light blue. Their profile has a teal
card and `Synthetic friend, local cosmetics.` bio.

Please check on your machine (the new layout is **UNVERIFIED visually**):

- Text wraps; channel names and profile text do not overlap neighboring panes.
- Messages scroll with the wheel and open at the bottom; long channel lists
  scroll without hiding the user bar.
- Popups sit above the content, stay clickable, and dismiss from the backdrop.
- Resize the window, especially with the DM side panel and a popup open;
  report clipping or controls pushed out of reach.
- In the real app (§9), check CDN avatars and server icons replace initials.
  Use `docs/COSMETICS.md` to check a local banner/avatar, `profile_color`,
  `bio`, and a known decoration asset hash. Check the decoration’s alignment
  around the avatar. CDN downloads, image scaling, and cosmetics are not
  verified by the headless demo.
- If available, compare Wayland and X11; Windows visual behavior also needs
  a local check. Report symptoms without committing private screenshots.

Grouping and reply previews have synthetic unit tests. The bundled demo
history has three different authors and no reply, so inspect those features
in a suitable live conversation (§9). Sending, emoji pickers, member lists,
and settings buttons are not implemented yet.

## 9. The real app

Set the token as in §2, optionally create a cosmetics file
(`docs/COSMETICS.md`), then:

```bash
cargo run -p dc-app
```

(or `dc-client` if you installed the package).

Expected: the status line goes `Fetching gateway URL…` →
`Connecting to the gateway…` → `Connected as <you>`, and the left column
fills with your servers and DMs. Click a text channel: its last 50 messages
load, and new messages appear live at the bottom. Closing the window exits
within a few seconds.

If the status line shows an error instead, copy it. If the window never
appears, run with `DC_LOG=debug` and copy stderr.

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
