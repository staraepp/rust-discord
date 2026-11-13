# Local cosmetics

Cosmetics are **client-side only**. They change how *your* copy of the client
draws people; no profile changes are sent to Discord and nobody else sees them.
CDN images are downloaded publicly, without your token.

## Where the file lives

`dc-client` reads one JSON file at startup:

1. the path in `$DC_COSMETICS`, if set; otherwise
2. `$XDG_CONFIG_HOME/dc-client/cosmetics.json`; otherwise
3. `~/.config/dc-client/cosmetics.json` (`%APPDATA%\dc-client\cosmetics.json`
   on Windows).

No file means no cosmetics. A file with a mistake in it (unknown key, bad
color, bad user ID) stops the app with an error that names the problem.
Restart the app after editing.

## Format

```json
{
  "me": {
    "display_name": "me but cooler",
    "name_color": "#ffd166",
    "accent_color": "#ff66aa",
    "profile_color": "#522244",
    "bio": "A little corner of the internet, just for me.",
    "banner": "/home/you/Pictures/banner.png",
    "avatar": "/home/you/Pictures/avatar.png",
    "avatar_decoration": "/home/you/Pictures/decoration.png"
  },
  "users": {
    "123456789012345678": {
      "display_name": "bestie",
      "name_color": "#00ccff",
      "profile_color": "#123344",
      "bio": "This bio only appears on my machine."
    }
  }
}
```

- `me` applies to whichever account is logged in.
- `users` applies to specific people, keyed by their user ID. If the same
  person is covered by both, the `users` entry wins field by field.
- Every field is optional. Colors accept `#rrggbb` or `#rgb`.
- Use absolute file paths; `~` and environment variables are not expanded.
  In JSON, escape Windows backslashes (`C:\\Users\\you\\Pictures\\avatar.png`).

| Field | What it does | Where it shows today |
| --- | --- | --- |
| `display_name` | Replaces the name | Message headers, one-to-one DM labels, user bar, profiles |
| `name_color` | Colors the name | Message headers, reply authors, user bar, profiles |
| `accent_color` | Sets the banner fallback color and gradient accent | Profiles and user bar |
| `profile_color` | Sets the card’s primary color; blends toward a darkened accent | Profile card gradient and user bar nameplate |
| `bio` | Adds local About Me text | Profile popup and DM profile panel |
| `banner` | Uses a local banner image | Profile popup and DM profile panel |
| `avatar` | Uses a local avatar image | Message headers, one-to-one DM rows, user bar, profiles |
| `avatar_decoration` | Overlays a local file or shop asset | User bar and profile avatars |

Click a message author’s avatar or name, or your avatar in the bottom-left
user bar, to open a profile popup. Click the backdrop to close it. One-to-one
DMs also show the other person’s profile on the right. Profiles can only use
users and fields already received; no separate full-profile request is made.

## Shop decorations by asset hash

Set `avatar_decoration` to the decoration’s **asset hash**, not its SKU/product
ID or a shop page URL:

```json
{
  "me": {
    "avatar_decoration": "a_REPLACE_WITH_ASSET_HASH"
  }
}
```

Replace the placeholder with a known asset hash. The backend fetches
`https://cdn.discordapp.com/avatar-decoration-presets/<asset>.png?size=96&passthrough=false`
without authentication. This changes your local rendering only; it does not
purchase or equip anything on your account. There is no shop catalog browser
or automatic catalog lookup yet.

A value containing `/`, `\`, or `.` is treated as a local path; otherwise it
is treated as an asset hash. Give files without extensions an explicit path
such as `./decoration`. Decorations overlap the avatar at 120% of its size.
The demo never downloads CDN assets, so asset hashes do not load in demo mode.

## Current limits

- `banner` and `avatar` overrides require local files (PNG, JPEG, GIF, WebP);
  arbitrary HTTPS image URLs are not supported.
- Without overrides, avatars, guild icons, banners and decorations use
  Discord data when present. CDN builders request static PNGs; animated
  shop effects, profile effects, frames and badges are not implemented.
- Missing CDN images keep initials or the banner color. Local file loading
  and image scaling still need visual verification on your machine.
- Nameplate colors come from the two profile colors; arbitrary nameplate
  artwork and a settings editor are not implemented.
- Custom emoji / stickers (FakeNitro-lite) are next after message sending.
