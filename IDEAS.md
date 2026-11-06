# Ideas (not merged)

Each idea lives on its own `idea/*` branch, based on `feat/scrolling-layout`. None are merged; keep, adapt, or delete them.

| Branch | What | Status |
| --- | --- | --- |
| `idea/center-column` | `center-column` command (default `alt+c`), like PaperWM's "center window horizontally". The column stays centered until focus moves to a column that isn't fully visible. | Unit tested; live-tested via CLI. |
| `idea/focus-sync-new-windows` | Fixes an upstream bug: after a new window opens, `focus_follows_cursor` does nothing until focus changes manually. The OS focuses the window before it's managed, so the WM marks focus as unsynced and never gets another focus event. | A/B live-tested with Explorer windows: without the fix 0/3 runs worked, with it 2/3. Also worth reporting upstream. |
