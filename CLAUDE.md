# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

<fork_purpose>

This repo is a fork of GlazeWM whose goal is adding PaperWM-style scrolling layouts. See `scrolling_design.md` for the design intent. Summary:

- Each workspace gets a horizontally scrolling strip of windows (like PaperWM).
- Scrolling along the top bar (zebar) scrolls left/right through windows.
- New windows open side-by-side instead of being squeezed to fit; overflow scrolls off-screen.
- Vertical stacking/unstacking of windows within a column should behave like PaperWM.
- Workspace/desktop and zebar behavior otherwise stays as in upstream GlazeWM.
- Must install side-by-side with upstream GlazeWM (separate install path) so both can be compared on one machine.

Side-by-side identity ("GlazeWM Scrolling"). Never reintroduce upstream identifiers:

- Runtime constants in `wm-common/src/app_identity.rs`: `APP_NAME`, `CLI_BIN_NAME` (`glazewm-scrolling`), `APP_DATA_DIR` (`~/.glzr/glazewm-scrolling/`, config + logs), `CONFIG_PATH_ENV_VAR` (`GLAZEWM_SCROLLING_CONFIG_PATH`). Use these instead of string literals.
- Windows installer (`resources/wix/`): own `UpgradeCode`s/component GUIDs, installs to `C:\Program Files\glzr.io\GlazeWM Scrolling\`, CLI installed as `cli\glazewm-scrolling.exe`, registry `HKLM\SOFTWARE\glzr.io\GlazeWM Scrolling`.
- macOS: bundle ID `io.glzr.glazewm-scrolling` (`resources/Info.plist`), app `GlazeWMScrolling.app`.
- Cargo bin names (`glazewm`, `glazewm-cli`, `glazewm-watcher`) unchanged; only installed names differ.
- Intentionally shared with upstream: single-instance lock (`wm-platform/src/platform_impl/*/single_instance.rs`; prevents both WMs running at once) and `DEFAULT_IPC_PORT` 6123 (`wm-common/src/ipc.rs`; zebar connects to it).

</fork_purpose>

<project_overview>
GlazeWM is a window manager for macOS and Windows, written in Rust.

Crate structure (`packages/*`):

- **wm** (bin): Main application, which implements the core window management logic. Install path on Windows: `C:\Program Files\glzr.io\GlazeWM Scrolling\glazewm.exe` (upstream: `glzr.io\GlazeWM\`)
- **wm-cli** (bin, lib): CLI for interacting with the main application. Added to `$PATH` by default. Install path on Windows: `C:\Program Files\glzr.io\GlazeWM Scrolling\cli\glazewm-scrolling.exe`
- **wm-common** (lib): Shared types, utilities, and constants used across other crates (incl. `InvokeCommand`/`AppCommand` clap definitions in `app_command.rs`, config types in `parsed_config.rs`, IPC DTOs).
- **wm-ipc-client** (lib): WebSocket client library for IPC with the main application.
- **wm-macros** (lib): Proc macros (`SubEnum`, `EnumFromInner`) used to define container enum subtypes.
- **wm-platform** (lib): Wrappers over platform-specific APIs; other crates do not call Windows/macOS APIs directly. Platform code lives in `src/platform_impl/{windows,macos}`.
- **wm-watcher** (Windows-only) (bin): Watchdog process ensuring proper cleanup when the main application exits. Install path on Windows: `C:\Program Files\glzr.io\GlazeWM Scrolling\glazewm-watcher.exe`

</project_overview>

<commands>

Toolchain is nightly (`rust-toolchain.toml`). CI (`.github/workflows/lint-check.yaml`) runs:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings   # clippy pedantic enabled workspace-wide
```

Other:

```sh
cargo build                          # default members: wm, wm-cli
cargo run                            # runs the wm
cargo test                           # all tests
cargo test -p wm                     # one crate
cargo test -p wm <test_name_substr>  # single test
```

- Config path: `~/.glzr/glazewm-scrolling/config.yaml`, override with `GLAZEWM_SCROLLING_CONFIG_PATH` env var or `--config` arg. Default config: `resources/assets/sample-config.yaml` (embedded via `include_str!`).
- `VERSION_NUMBER` env (set in `.cargo/config.toml`, default `0.0.0`) controls displayed version.
- Formatting: 2-space indent, `max_width = 75`, crate-granular imports grouped std/external/crate (`rustfmt.toml`).

</commands>

<architecture>

**Command-event loop.** All WM state lives in `WmState` (`packages/wm/src/wm_state.rs`). `start_wm` in `main.rs` runs a `tokio::select!` loop over platform events, keybindings, and IPC messages, dispatching to `WindowManager` (`wm.rs`):

- Platform events → handlers in `packages/wm/src/events/` (window shown/destroyed/focused/moved, display changes, mouse move).
- Commands (keybindings, IPC, CLI) → `WindowManager::run_command` matches on `InvokeCommand` (defined in `wm-common/src/app_command.rs`) and calls functions in `packages/wm/src/commands/{container,window,workspace,monitor,general}`. Commands also call each other.
- Adding a user-facing command = add variant to `InvokeCommand` + match arm in `wm.rs` + implementation under `commands/`.

**Deferred sync.** Commands/events mutate the container tree only; they never move native windows directly. They queue work on `state.pending_sync` (`pending_sync.rs`: `queue_container_to_redraw`, `queue_focus_change`, `queue_workspace_to_reorder`, etc.). After each event/command batch, `platform_sync` (`commands/general/platform_sync.rs`) computes rects and applies positions, focus, z-order, window effects, and show/hide to native windows.

**Container tree.** `Root → Monitor → Workspace → (SplitContainer | TilingWindow | NonTilingWindow)`. Split containers only hold tiling windows/splits and must have ≥1 child. Non-tiling windows (floating/minimized/maximized/fullscreen) are direct workspace children. Containers are `Rc<RefCell<..>>` wrappers in `packages/wm/src/models/`; the `Container` enum and subtypes (`TilingContainer`, `WindowContainer`, `DirectionContainer`, ...) are generated via `wm-macros` + `enum_as_inner` + `ambassador` delegation.

**Traits** (`packages/wm/src/traits/`) supply tree navigation (`CommonGetters`: parent, children, siblings, descendants, focus order), `TilingDirectionGetters`, `TilingSizeGetters`, `PositionGetters`, `WindowGetters`. Often implemented via `macro_rules!` (e.g. `impl_position_getters_as_resizable!`).

**Layout math (key for scrolling work).** No separate layout engine: a tiling container's rect is computed recursively in `PositionGetters::to_rect` (`traits/position_getters.rs`) from its parent's rect, the parent's `TilingDirection` (`Horizontal`/`Vertical`, `wm-common/src/tiling_direction.rs`), its fractional `tiling_size` (siblings sum to 1.0), gaps, and previous siblings' rects. In the tiling layout, all tiling children therefore fit within the workspace rect (see scrolling layout below for the exception). Resizing/insertion logic that maintains `tiling_size` lives in `commands/container/` (`attach_container`, `resize_tiling_container`, `flatten_*`, `wrap_in_split_container`, `move_container_within_tree`).

**Scrolling layout.** `Workspace` has a `layout` (`WorkspaceLayout::{Scrolling, Tiling}`, initial value from `WorkspaceConfig.layout`, changed at runtime via `set-layout`/`toggle-layout`), a `scroll_offset` (px), and `ScrollingConfig` (`general.scrolling`). Key rules:

- Direct tiling children of a scrolling workspace are *columns*. For columns only, `tiling_size` means "fraction of viewport width" and does NOT sum to 1; inside a column, normal proportional semantics apply. Check with `CommonGetters::scrolling_parent()` / `scrolling_workspace()`.
- Every size/tree helper branches on this: `attach_container` (default width), `detach_container` (no redistribution), `resize_tiling_container` (independent clamp), `flatten_split_container`/`wrap_in_split_container` (preserve column width), `flatten_child_split_containers` (never inverts a scrolling workspace's direction), `container_to_resize`. New code touching sizes must respect this.
- Pure geometry (spans, reveal, clamp, visibility, presets) lives in `packages/wm/src/scrolling.rs`; column rects come from `traits::scrolling_column_rect`.
- `platform_sync` calls `update_scroll_offsets` before redrawing: viewport minimally scrolls to the workspace's most recently focused column (view is derived from focus). Use `Workspace::tiling_insertion_point` to insert tiling windows (column-level in scrolling workspaces).
- Columns fully outside the viewport are cloaked in place (Windows; any `hide_method`), stay in the taskbar, and drive `DisplayState` like hidden workspaces. Never park them off-screen: DWM thumbnails of off-screen windows freeze (verified), and peek previews (planned) depend on live thumbnails.
- Scrolling commands: `commands/scrolling/` (`scroll`, `consume-into-column`, `expel-from-column`, `cycle-column-width`, `set-layout`, `toggle-layout`). `move left/right` swaps whole columns.

**Hiding windows.** Windows on non-displayed workspaces are hidden by `reposition_window` in `platform_sync.rs` — moved to a monitor "hide corner" (`HideCorner`, leaving a `VISIBLE_SLIVER`) or cloaked (Windows), per `HideMethod` config. `DisplayState` (Shown/Showing/Hiding/Hidden) tracks transitions so resulting platform events can be ignored.

**Tests.** Mock builders for models live in `packages/wm/src/test_utils.rs` (uses `bon` builders, mock monitor 1680x1050) and `packages/wm-platform/src/test_utils.rs`.

</architecture>

<output_guidelines>

- Be extremely concise. Sacrifice grammar for the sake of conciseness.
- Do not leave partial or simplified implementations.
- The required quality standard is high. Low quality code will be rejected.
- Do not proceed with solutions that are hacky. Solutions must be robust, maintainable, and extendable. Ask guiding questions if uncertain about a solution.

</output_guidelines>

<code_style_guidelines>

- Avoid `.unwrap()` wherever possible.
- For error handling:
  - Use `crate::Error` and `crate::Result` within the `wm-platform` crate.
  - Use `anyhow` in all other crates.
- For logging, use `tracing` macros (e.g. `tracing::info!("...")`).

</code_style_guidelines>

<code_comment_guidelines>

- Functions should always be documented.
- Use punctuation mark at the end of all comments.
- If using unsafe features, include a "SAFETY: ..." comment.
- Wrap type names in backticks (e.g. `NativeMonitor`).

Comment structure:

```rs
/// <Concise summary of the function or type>
///
/// (optional) <Notable caveats for usage (kept brief)>
///
/// (optional) <Describe return value if ambiguous (e.g. "Returns a vector of `NativeMonitor`, sorted by their position from left-to-right.")>
///
/// (optional) # Example usage
///
/// <Code block with example usage>
///
/// (optional) # Platform-specific
///
/// <Bullet-point list of behavioral differences on macOS vs Windows>
pub fn my_function() { ... }
```

</code_comment_guidelines>

<test_guidelines>

- Use `#[cfg(test)]` for test modules.
- Write unit tests for core functionality.

</test_guidelines>
