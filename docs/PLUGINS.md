# Local plugins (API v1)

This build supports pure **outgoing text transforms**, not the full plugin
system from the blueprint. Plugins only run on text you explicitly submit.
No token, transport, mutable server state, filesystem, clipboard, network,
module loader, or automatic-send function is exposed.

Create a folder under the configuration directory’s `plugins/`, for example
`~/.config/dc-client/plugins/sparkle/` (beside `cosmetics.json`). Two files:

`plugin.json`:

```json
{
  "id": "sparkle",
  "name": "A little sparkle",
  "api_version": 1,
  "capabilities": ["outgoing-transform"]
}
```

`plugin.js`:

```javascript
function transformOutgoing(text) {
  return text + " ✦";
}
```

Restart the app to discover a new package. Open settings, explicitly select
its outgoing-text grant, then **Save appearance**. This permits the plugin
to read and change submitted text. Unselected packages never execute.

The host limits each runtime to 16 MiB, 256 KiB stack, 256 KiB source,
50 ms initial execution and 20 ms per transform. At most 16 packages are
loaded; filesystem adapters reject paths escaping the plugins directory.
An interrupted/failed transform preserves your draft and does not send it.
Output is still checked against message limits. A manually retried message
keeps its nonce and previously transformed content in the backend.

This is an early extension boundary, not a claim of a formally audited
sandbox. Other capabilities, incoming hooks, UI slots, per-plugin storage,
and hot reload are still pending. Never install untrusted plugins merely
because this host restricts IO.
