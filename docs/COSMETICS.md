# Local cosmetics

Cosmetics are **client-side only**. They change how *your* copy of the client
draws people; nothing is sent to Discord and nobody else sees them.

## Where the file lives

`dc-client` reads one JSON file at startup:

1. the path in `$DC_COSMETICS`, if set; otherwise
2. `$XDG_CONFIG_HOME/dc-client/cosmetics.json`; otherwise
3. `~/.config/dc-client/cosmetics.json` (`%APPDATA%\dc-client\cosmetics.json`
   on Windows).

No file means no cosmetics. A file with a mistake in it (unknown key, bad
color, bad user ID) stops the app with an error that names the problem, so a
typo never silently does nothing. Restart the app after editing.

## Format

```json
{
  "me": {
    "display_name": "me but cooler",
    "name_color": "#ffd166",
    "accent_color": "#ff66aa",
    "banner": "/home/you/Pictures/banner.png",
    "avatar": "/home/you/Pictures/avatar.png",
    "avatar_decoration": "/home/you/Pictures/decoration.png"
  },
  "users": {
    "123456789012345678": {
      "display_name": "bestie",
      "name_color": "#00ccff"
    }
  }
}
```

- `me` applies to whichever account is logged in.
- `users` applies to specific people, keyed by their user ID. If the same
  person is covered by both, the `users` entry wins field by field.
- Every field is optional.

| Field | What it does | Where it shows today |
| --- | --- | --- |
| `display_name` | Replaces the name | Messages, your profile card |
| `name_color` | Colors the name (`#rrggbb` or `#rgb`) | Messages, your profile card |
| `accent_color` | Profile accent color | Your profile card (top-left) |
| `banner` | Profile banner image | Your profile card |
| `avatar` | Avatar image | Your profile card |
| `avatar_decoration` | Image drawn on top of the avatar | Your profile card |

## Current limits

- Images must be **local files** (PNG, JPEG, GIF, WebP…). `https://` URLs are
  ignored for now.
- Banner, avatar and decoration are only drawn on your own card; there is no
  profile popup for other users yet, so for them only the name and name
  color are visible.
- Decorations from Discord's shop are not downloaded automatically yet: save
  the image yourself and point `avatar_decoration` at it.
- Custom emoji / stickers ("FakeNitro-lite") are not implemented yet.
