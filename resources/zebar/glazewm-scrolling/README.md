# glazewm-scrolling (Zebar widget pack)

Top bar for GlazeWM Scrolling, based on Zebar's `glzr-io.starter` `with-glazewm` widget.

- A thin scrollbar along the bottom edge of the bar represents the displayed workspace's strip of columns. The lighter band is the part of the strip that's on-screen, and the highlighted segment is the focused column.
- Scrolling the mouse wheel (or touchpad) anywhere over the bar scrolls the workspace one column at a time (`scroll --direction left|right`).
- Clicking the scrollbar focuses the column under the cursor.
- The layout button toggles the workspace between the scrolling and tiling layouts.

## Install

Copy this folder to `%USERPROFILE%\.glzr\zebar\glazewm-scrolling`. GlazeWM Scrolling's default config starts this bar when the WM starts:

```yaml
general:
  startup_commands: ['shell-exec zebar start-widget-preset --pack glazewm-scrolling --widget-name scrolling-bar --preset default']
```

This leaves Zebar's own startup settings alone, so upstream GlazeWM keeps starting its usual bar with `shell-exec zebar`.

Requires GlazeWM Scrolling, which exposes each workspace's `layout` and `scrolling` state over IPC.

## Tests

The scrollbar geometry and wheel handling live in `scrollbar.js` and can be tested with Node: `node --test` in this folder.
