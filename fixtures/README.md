# Fixtures

Every file here is **hand-written and synthetic**. IDs, names, session IDs and
URLs are invented (`*.example.invalid`, snowflakes like `1000000000000000xx`).
Shapes follow the unofficial user-API docs (see `ref.md`).

Rules (CLAUDE.md hard rule 1):

- Never paste a real capture here. No real tokens, user IDs, guild names,
  message content, session IDs or analytics tokens.
- If a live payload fails to parse, reduce it to the minimal failing shape and
  replace every value with a fake one before adding it.
