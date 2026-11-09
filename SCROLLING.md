# GlazeWM Scrolling

A fork of [GlazeWM](https://github.com/glzr-io/glazewm) that adds a PaperWM-style scrolling layout. Workspaces hold a horizontal strip of columns that scrolls to keep the focused column in view, so new windows open side-by-side instead of being squeezed to fit.

Everything else (workspaces, monitors, window rules, Zebar) behaves as in upstream GlazeWM.

## Side-by-side with upstream GlazeWM

GlazeWM Scrolling installs and stores its files separately, so both can be installed on one machine:

| | GlazeWM | GlazeWM Scrolling |
| --- | --- | --- |
| Install path | `C:\Program Files\glzr.io\GlazeWM\` | `C:\Program Files\glzr.io\GlazeWM Scrolling\` |
| CLI | `glazewm` | `glazewm-scrolling` |
| Config | `~/.glzr/glazewm/config.yaml` | `~/.glzr/glazewm-scrolling/config.yaml` |
| Config path env var | `GLAZEWM_CONFIG_PATH` | `GLAZEWM_SCROLLING_CONFIG_PATH` |

Only one of them can run at a time (they share the single-instance lock). They also share IPC port 6123, so Zebar works with either one. To switch, exit one from its tray icon, then start the other.

## Layouts

Each workspace has a layout, set with `layout` in its workspace config and changeable at runtime:

- `scrolling` (default): columns in a horizontally scrolling strip.
- `tiling`: upstream GlazeWM's layout, where all windows fit on screen.

```yaml
workspaces:
  - name: '1'
  - name: '2'
    layout: 'tiling'
```

Switching a workspace between layouts keeps its windows and their order.

## How the scrolling layout works

- **Columns.** Each window opens in a new column to the right of the focused one. The column takes `default_column_width` (50% by default) of the workspace width. Columns may extend past the screen.
- **Scrolling follows focus.** The view scrolls just far enough to show the focused column in full. `focus --direction left|right` moves between columns; at either end of the strip it continues to the adjacent monitor, as in upstream GlazeWM.
- **Stacks.** A column can hold several windows stacked vertically. `consume-into-column` stacks the focused window into the neighbouring column, and `expel-from-column` moves it out into its own column. As in PaperWM, `consume-into-column --from right` (`alt+shift+i`) instead pulls the neighbouring window into the focused column, and `expel-from-column --direction right --last` (`alt+shift+o`) pushes the column's bottom window out. `focus --direction up|down` moves within a stack, and `move --direction up|down` reorders it.
- **Moving columns.** `move --direction left|right` swaps the whole focused column with its neighbour.
- **Widths.** `cycle-column-width` steps the focused column through `column_width_presets`. `resize --width` and `size --width` work on whole columns, relative to the workspace width.
- **Off-screen columns** are hidden from the screen but stay in the taskbar. Focusing one (for example from the taskbar) scrolls to it. If GlazeWM Scrolling exits or crashes, all windows are restored.
- **Peeking columns.** A column partly outside the screen is shown according to `peek_mode`:
  - `auto` (default): the real window extends past the screen edge. At an edge shared with another monitor, a live preview of the visible part is shown instead, so the window doesn't spill onto the other monitor. Clicking a preview focuses its window.
  - `preview`: live previews at every screen edge.
  - `hide`: peeking columns are hidden.

  Previews are Windows-only; on macOS peeking windows are always shown.
- **Animation.** When the view scrolls, the strip slides to its new position (Windows-only). During the slide, the windows are briefly replaced by live previews that the compositor moves, since moving real windows every frame is too slow for most apps. The real windows reappear at their new positions when the slide ends.

## Config

```yaml
general:
  scrolling:
    # Width of newly opened columns.
    default_column_width: '50%'

    # Widths cycled through by the `cycle-column-width` command.
    column_width_presets: ['33%', '50%', '67%', '100%']

    # How partially visible columns are shown: 'auto', 'preview' or 'hide'.
    peek_mode: 'auto'

    # Animation of the strip when it scrolls.
    animation:
      enabled: true
      duration_ms: 200
      # 'ease_out_cubic', 'ease_in_out_cubic' or 'linear'.
      easing: 'ease_out_cubic'
```

Percentages are relative to the workspace width (the monitor width minus outer gaps). Pixel values such as `'800px'` also work.

## Commands

| Command | Default binding | Description |
| --- | --- | --- |
| `toggle-layout` | `alt+shift+v` | Toggle the focused workspace between scrolling and tiling. |
| `set-layout --layout scrolling\|tiling` | | Set the focused workspace's layout. |
| `consume-into-column --direction left\|right` | `alt+,` / `alt+.` | Stack the focused window into the column to the left/right. |
| `expel-from-column --direction left\|right` | `alt+shift+,` / `alt+shift+.` | Move the focused window out of its column, into a new column to the left/right. |
| `consume-into-column --from left\|right` | `alt+shift+i` (right) | Pull the top window of the column to the left/right into the bottom of the focused column. Focus stays put. |
| `expel-from-column --direction left\|right --last` | `alt+shift+o` (right) | Move the bottom window of the focused column out into a new column to the left/right. Focus stays put. |
| `cycle-column-width` | `alt+w` | Cycle the focused column's width through `column_width_presets`. |
| `toggle-column-full-width` | `alt+shift+c` | Toggle the focused column between full width and its previous width. |
| `center-column` | `alt+c` | Scroll to center the focused column. It stays centered until focus moves to a column outside the view. Columns near either end of the strip can only be centered as far as the strip allows. |
| `focus --column first\|last` | `alt+home` / `alt+end` | Focus the first/last column. |
| `move --column first\|last` | `alt+shift+home` / `alt+shift+end` | Move the focused column to the start/end of the strip. |
| `scroll --direction left\|right` | | Focus the adjacent column without leaving the workspace or moving the cursor. Meant for bars (mouse wheel). |

Commands can also be run from the CLI, e.g. `glazewm-scrolling command toggle-layout`.

## Zebar bar with scrollbar

`resources/zebar/glazewm-scrolling` is a Zebar widget pack, based on the starter bar, that turns the bar into a scroll bar for the workspace:

- A thin strip along the bottom of the bar shows the workspace's columns. The lighter band is the part on screen, and the highlighted segment is the focused column.
- The mouse wheel anywhere over the bar scrolls one column at a time.
- Clicking the strip focuses the column under the cursor.
- A button toggles the workspace's layout.

See its [README](resources/zebar/glazewm-scrolling/README.md) for installation.

## IPC

`query workspaces` and `query monitors` include each workspace's `layout`, and for scrolling workspaces a `scrolling` object:

```json
{
  "scrollOffset": 640,
  "stripWidth": 3800,
  "focusedColumnId": "…",
  "columns": [{ "id": "…", "start": 0, "width": 1250 }]
}
```

Positions are in pixels along the strip. The viewport is the workspace's `width`, starting at `scrollOffset`. A `workspace_updated` event is emitted whenever the layout or scroll state changes.

## Building an installer

Releases are built and signed by CI (`.github/workflows/package.yaml`). To build an x64 installer locally, run the following in PowerShell from the repo root. It needs [Rust](https://rustup.rs) and the [.NET SDK](https://dotnet.microsoft.com/download).

```powershell
# Build release executables. Without the `ui_access` feature, since
# UIAccess executables only launch when signed.
$env:VERSION_NUMBER = "0.1.0"
cargo build --locked --release --target x86_64-pc-windows-msvc -p wm -p wm-cli -p wm-watcher
New-Item -ItemType Directory -Force out/x64
Copy-Item target/x86_64-pc-windows-msvc/release/glazewm*.exe out/x64/

# Install WiX 5 and its extensions, then build the MSI.
dotnet tool install --global wix --version 5.0.0
wix extension add -g WixToolset.UI.wixext/5 WixToolset.Util.wixext/5
wix build -arch x64 -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext `
  -out out/installer-x64.msi resources/wix/standalone.wxs resources/wix/standalone-ui.wxs `
  -d VERSION_NUMBER="0.1.0" -d EXE_DIR="out/x64"
```

`out/installer-x64.msi` installs to `C:\Program Files\glzr.io\GlazeWM Scrolling\`, next to an existing GlazeWM install. Without UIAccess, GlazeWM Scrolling can't move or focus windows of apps running as administrator.

## Development

See [CLAUDE.md](CLAUDE.md) for the architecture of the scrolling layout, and [CONTRIBUTING.md](CONTRIBUTING.md) for general development.
