# Ideas (not merged)

Each idea lives on its own `idea/*` branch, based on `feat/scrolling-layout`. None are merged; keep, adapt, or delete them.

| Branch | What | Status |
| --- | --- | --- |
| `idea/center-column` | `center-column` command (default `alt+c`), like PaperWM's "center window horizontally". The column stays centered until focus moves to a column that isn't fully visible. | Unit tested; live-tested via CLI. |
| `idea/focus-sync-new-windows` | Fixes an upstream bug: after a new window opens, `focus_follows_cursor` does nothing until focus changes manually. The OS focuses the window before it's managed, so the WM marks focus as unsynced and never gets another focus event. | A/B live-tested with Explorer windows: without the fix 0/3 runs worked, with it 2/3. Also worth reporting upstream. |
| `idea/bar-focused-title` | Zebar bar shows the focused window's title next to the workspaces, as PaperWM's top bar does. Only the bar on the focused workspace's monitor shows it. | Node unit tests; checked live in a screenshot (with upstream GlazeWM). The title only refreshes on focus and workspace events, not when a window renames itself. |
| `idea/center-focused-column` | Builds on `idea/center-column`. Adds the `general.scrolling.center_focused_column` setting, `never` (default) \| `always` \| `on_overflow`, similar to niri's option. `on_overflow` centers a column you focus only when it isn't fully visible. | Unit tested; `on_overflow` live-tested via CLI focus commands. |

## For future exploration

- **Rounded corners (and shadows) on slide previews.** Previews are DWM thumbnails, which have square corners and no shadow, since DWM applies those when composing the real window. One overlay window per preview with `DWMWA_WINDOW_CORNER_PREFERENCE` set to round was too slow (~20 ms per frame vs ~12 ms for the single overlay, at 90 Hz). The remaining route is DirectComposition: one visual per window with a rounded `IDCompositionRectangleClip`, hosting the window's thumbnail via the undocumented `DwmpCreateSharedThumbnailVisual`. Being undocumented, it may break with Windows updates, so it would need a fallback to the current square previews.

## Considered, not implemented

- **Modifier + mouse wheel anywhere scrolls the workspace.** Raw input (used for focus-follows-cursor) can't swallow wheel events, so this needs a low-level mouse hook. That hook would see every mouse move on the WM's event-loop thread, so any stall there would lag the cursor system-wide. Also, after a swallowed wheel event, releasing Alt opens app menus and releasing Win opens Start, which needs key-injection workarounds. Scrolling over the bar covers this without those risks.
