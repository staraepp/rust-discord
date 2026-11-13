# HANDOFF — where the work stands (for the next agent)

Read `CLAUDE.md`, `blueprint.md`, `ref.md`, `docs/PROGRESS.md` first. This
file covers what is **in flight** and what the owner asked for in chat,
which is not written down anywhere else.

## Owner's goal (in their words, condensed)

A lightweight, fast, native Rust Discord client (GPUI) that looks and feels
close to the official desktop client, plus client-side-only cosmetics:

- custom banners, custom profile colors, custom display names
- any avatar decoration from the shop (client side)
- custom emojis (FakeNitro-lite style: unusable emoji sent as image links)
- "a ton of profile customization"

Decisions already made by the owner:

- Path 2: build our own client (not a Serein fork, not Vesktop).
- Toolkit: **GPUI** (no Slint bake-off).
- Do **not** translate/copy Discord's own client code (proprietary; we told
  the owner no). Rebuild the *layout* from screenshots with our own code and
  styling; never ship Discord's logos, icons or artwork.
- Spoofing rule (CLAUDE.md rule 2): owner said it may be ignored *only if
  required to work*. It is not required: the honest client already connects
  and loads data (see "Verified live" below). Keep it honest.

## Verified live by the owner (2026-11-13)

The owner ran `dc-client` against their real account and sent a screenshot:
it connected, showed the guild's channel list (with category heading,
selected-channel highlight), loaded message history with author, time and
text, wrapped long lines, and the status bar read `Connected as <name>`.
So U1–U4 and the GPUI rendering on Linux are effectively confirmed;
live MESSAGE_CREATE, resume/reconnect and cosmetics are not yet.

## UI reference (described from the owner's screenshots)

The screenshots themselves are **not** in the repo on purpose: they contain
other people's names, avatars and the owner's partially masked account
details, and this repository is public. What they show:

1. **Window layout, left to right**
   - Server rail (~72px): home button on top, then round server icons
     (squircle on hover/selected), unread pill on the left edge, mention
     badges (red count) at the bottom-right of an icon, folders.
   - Channel column (~240px): header (server name with dropdown, or
     "Find or start a conversation" search box in Home), then
     - Home: Friends / Nitro / Shop / Quests links, "Direct Messages"
       heading with `+`, DM rows = 32px round avatar with status dot,
       name, one-line activity/subtitle ("10 Members" for groups), pin icon.
     - Guild: channels with type icon (`#` text, megaphone announcements,
       speaker voice, forum icon), category headings in small caps with a
       collapse chevron, threads nested with a curved connector under
       their parent, unread channels in bold white with a dot on the left.
   - User bar at the bottom of the channel column: avatar with status,
     display name + custom status, mute / deafen / settings buttons. The
     owner's has a gradient "nameplate" background.
   - Message pane: header bar (channel/DM name with icon, action icons,
     search box on the right), messages, composer at the bottom
     ("Message #channel", `+` on the left, gift/GIF/sticker/emoji/apps
     buttons on the right).
   - Right panel (toggleable): member list grouped by role ("admin — 2"),
     each row avatar + status + colored name + server tag pill + activity
     line; some rows have nameplate gradient backgrounds. In a DM the
     right panel is the other user's profile instead.
2. **Messages**: 40px avatar, name (role color) + server-tag pill + bot
   "APP" pill + time ("1:20 PM"); follow-up messages from the same author
   within a few minutes have no header. Replies show a small line above
   with a curved connector, the replied-to author's mini-avatar, name and a
   truncated one-line preview. Mentions/channel links render as tinted
   pills. "NEW" red divider line for unread. Hover shows a floating toolbar
   (quick reactions, react, reply, forward, more). Messages mentioning you
   have a gold left border and tint.
3. **Profile popup / side profile**: banner (image, or banner color)
   on top; 80px avatar overlapping the banner's bottom-left with a thick
   ring in the card color, status dot, and the avatar decoration drawn
   over it; a speech-bubble custom status next to the avatar; display name
   (large, bold), then `username • pronouns/tagline` and server-tag pill;
   badges row; mutual friends/servers; bio with "View Full Bio"; "Member
   Since"; "Game Collection" strip; activity card ("Playing …" with
   elapsed time); action button at the bottom (Edit Profile for self,
   "Message @user" input for others). With profile themes the whole card
   is a vertical gradient from the primary color (top) to the accent color
   (bottom) — e.g. pink→magenta, or dark brown.
4. **Pickers**: emoji picker (search, category rail on the left with server
   icons, "Favorites", "Frequently Used", per-server sections, hover
   preview footer), sticker picker (same shape, larger tiles), GIF picker
   (category tiles; third-party search — out of scope for now).
5. **Settings**: modal with a left nav (Account, Data & Privacy, Voice &
   Video, Appearance…) and a content pane. Owner runs Vencord, which adds
   "Vencord / Plugins / Themes" entries there. We only plan a small
   settings screen of our own (cosmetics editor, appearance); account
   security settings stay in the official client.
6. **Profile editor** (official): nameplate, avatar & decoration, banner
   color, profile effect & frame, display name style — this is the
   feature list our local cosmetics editor should cover.

Palette observed (dark theme): app background ~#1a1a1e, channel column
~#121214, message pane ~#1a1a1e, hover ~#2a2a30, selected ~#35353c, text
~#dbdee1, muted ~#949ba4, blurple accent ~#5865f2, mention pill bg
~rgba(88,101,242,0.3), unread red #f23f43, online #23a55a, idle #f0b232,
dnd #f23f43.

## Completed on branch `claude/affectionate-ramanujan-a2dkra`

The unfinished view from `6b81c9f` now builds. The app and demo use
`UiOptions`; the backend answers `FetchImage` with `get_cdn` bytes or
`ImageFailed`. The demo always answers `ImageFailed` and stays offline.
Both app and demo pass UTC offset zero: timestamps are 12-hour **UTC**, not
local time yet. No new dependency was added.

The existing groundwork is integrated: CDN URL builders; user banner/accent/
decoration fields; the store’s user index and guild icons; local
`profile_color`, `bio`, and decoration asset hashes; Home/Guild scopes;
grouped messages and reply previews; profile popup and DM side panel.

Fixes include the profile gradient’s `Color`/RGB mismatch, duplicated prefixes
for unmatched `<` text, and a reversed DM-order test expectation. Full Linux
checks pass with Rust 1.97.0: fmt, clippy with warnings denied, 126 tests,
and cargo-deny. Image-command tests use only mock HTTP and synthetic data.
No account credentials or screenshots were used in this repair.

COSMETICS, PROGRESS, LOCAL_VERIFY §8, and README are updated. The latest UI
has **not been visually verified**. Do not confuse the earlier owner-verified
connection/history screenshot with verification of this new layout/images.
The composer remains a placeholder; settings and member lists are absent.

Next features, explicitly ordered by the owner:

1. Message composer (sending).
2. Emoji picker with FakeNitro-lite.
3. Member list (needs guild subscriptions, opcode 37).
4. Settings / cosmetics editor.

First have the owner inspect the window using LOCAL_VERIFY §8–§9. Keep
secrets/screenshots out of the public repo, and ask before adding dependencies
(including any timezone library).

## Build notes

- GPUI 0.2.2 needs `libc = "=0.2.189"` (pinned in `dc-ui`) and
  `mio 1.2.4`; see PROGRESS A11.
- Linux build deps: `libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev
  libvulkan-dev libxcb1-dev libfontconfig-dev`.
- Headless screenshots do not work in the cloud sandbox (Xvfb + lavapipe
  renders nothing); the window does start. Visual checks need the owner.
