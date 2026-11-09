use std::{collections::HashMap, time::Duration};

use anyhow::Context;
use uuid::Uuid;
#[cfg(target_os = "windows")]
use wm_common::WindowEffectConfig;
use wm_common::{
  CursorJumpTrigger, DisplayState, HideCorner, HideMethod, UniqueExt,
  WindowState, WmEvent,
};
#[cfg(target_os = "windows")]
use wm_platform::NativeWindowWindowsExt;
#[cfg(target_os = "windows")]
use wm_platform::{CornerStyle, OpacityValue};
use wm_platform::{Rect, SlideRequest, SlideTarget, WindowZOrder};

use crate::{
  models::{Container, WindowContainer, Workspace},
  scrolling::{self, ColumnVisibility, PeekDecision, RevealMode},
  traits::{CommonGetters, PositionGetters, WindowGetters},
  user_config::UserConfig,
  window_previews::DesiredPreview,
  wm_state::WmState,
};

pub fn platform_sync(
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let focused_container =
    state.focused_container().context("No focused container.")?;

  if state.pending_sync.needs_focus_update() {
    sync_focus(&focused_container, state)?;
  }

  update_scroll_offsets(state, config)?;

  if !state.pending_sync.containers_to_redraw().is_empty()
    || !state.pending_sync.workspaces_to_reorder().is_empty()
  {
    redraw_containers(&focused_container, state, config)?;
  }

  if state.pending_sync.needs_cursor_jump()
    && config.value.general.cursor_jump.enabled
  {
    jump_cursor(focused_container.clone(), state, config)?;
  }

  if state.pending_sync.needs_focused_effect_update()
    || state.pending_sync.needs_all_effects_update()
  {
    // Keep reference to the previous window that had focus effects
    // applied.
    let prev_effects_window = state.prev_effects_window.clone();

    if let Ok(window) = focused_container.as_window_container() {
      apply_window_effects(&window, true, config);
      state.prev_effects_window = Some(window.clone());
    } else {
      state.prev_effects_window = None;
    }

    // Get windows that should have the unfocused border applied to them.
    // For the sake of performance, we only update the border of the
    // previously focused window. If the `reset_window_effects` flag is
    // passed, the unfocused border is applied to all unfocused windows.
    let unfocused_windows =
      if state.pending_sync.needs_all_effects_update() {
        state.windows()
      } else {
        prev_effects_window.into_iter().collect()
      }
      .into_iter()
      .filter(|window| window.id() != focused_container.id());

    for window in unfocused_windows {
      apply_window_effects(&window, false, config);
    }
  }

  sync_window_previews(state)?;
  broadcast_scroll_state_changes(state)?;

  state.pending_sync.clear();

  Ok(())
}

/// Emits a `WorkspaceUpdated` event for each workspace whose layout or
/// scroll state (scroll offset, column sizes, focused column) changed
/// since it was last broadcast.
///
/// Bars (e.g. Zebar) use these events to keep scroll indicators in sync.
fn broadcast_scroll_state_changes(
  state: &mut WmState,
) -> anyhow::Result<()> {
  for workspace in state.workspaces() {
    let scroll_state =
      (workspace.layout(), workspace.scrolling_state_dto()?);

    if workspace.record_broadcast_scroll_state(scroll_state) {
      state.emit_event(WmEvent::WorkspaceUpdated {
        updated_workspace: workspace.to_dto()?,
      });
    }
  }

  Ok(())
}

/// Gets how a scrolling workspace should reveal its focused column in
/// this sync.
fn reveal_mode(workspace: &Workspace, state: &WmState) -> RevealMode {
  if state
    .pending_sync
    .workspaces_to_center()
    .contains(&workspace.id())
  {
    RevealMode::Centered
  } else {
    RevealMode::Minimal
  }
}

/// Scrolls displayed workspaces with the scrolling layout to reveal their
/// focused column (centering it if queued via
/// `queue_workspace_to_center`), and queues their columns for redraw if
/// the viewport moved.
///
/// If enabled, the strip animates from its previous to its new position
/// (see `ScrollAnimations`). It also animates when a newly opened window
/// shifts other columns, even if the viewport didn't move.
fn update_scroll_offsets(
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  cancel_stale_animations(state);

  // The offset is kept while a tiling window is dragged. Otherwise, the
  // strip would scroll under the cursor when the dragged window's column
  // is temporarily removed, changing where the window gets dropped.
  let scrolling_workspaces =
    state.workspaces().into_iter().filter(|workspace| {
      workspace.is_scrolling()
        && workspace.is_displayed()
        && !workspace.has_active_tiling_drag()
    });

  let animation_config = &config.value.general.scrolling.animation;

  for workspace in scrolling_workspaces {
    let did_scroll =
      workspace.update_scroll_offset(reveal_mode(&workspace, state))?;

    if did_scroll {
      state
        .pending_sync
        .queue_containers_to_redraw(workspace.tiling_children());
    }

    if !animation_config.enabled {
      continue;
    }

    let window_rects = tiling_window_rects(&workspace)?;

    // Newly opened windows get an empty slot that the other columns slide
    // aside for, and appear once the animation ends.
    let opened_window_ids = window_rects
      .iter()
      .map(|(window, _)| window.id())
      .filter(|id| state.pending_sync.opened_window_ids().contains(id))
      .collect::<Vec<_>>();

    let above = window_rects
      .iter()
      .map(|(window, _)| window.native().clone())
      .collect::<Vec<_>>();

    let targets = slide_targets(window_rects, &opened_window_ids, state);

    let makes_room = !opened_window_ids.is_empty()
      && targets.iter().any(|target| target.from != target.to);

    // Animations already running are retargeted on every sync, since
    // windows may have been added, removed or resized.
    let is_animating =
      state.scroll_animations.is_animating(workspace.id());
    let should_animate = is_animating
      || ((did_scroll || makes_room)
        && state.scroll_animations.was_displayed(workspace.id()));

    tracing::debug!(
      "Scroll sync for workspace {}: offset={}, did_scroll={}, \
       makes_room={}, is_animating={}, should_animate={}.",
      workspace.id(),
      workspace.scroll_offset(),
      did_scroll,
      makes_room,
      is_animating,
      should_animate,
    );

    if !should_animate {
      continue;
    }

    let monitor = workspace.monitor().context("No monitor.")?;

    let request = SlideRequest {
      key: workspace.id().as_u128(),
      clip_rect: monitor.to_rect()?,
      scale_factor: monitor.native_properties().scale_factor,
      above,
      targets,
      duration: Duration::from_millis(u64::from(
        animation_config.duration_ms,
      )),
      easing: animation_config.easing,
    };

    let did_start = state.scroll_animations.start(workspace.id(), request);
    tracing::debug!(
      "Scroll animation started or retargeted: {did_start}."
    );

    state
      .scroll_animations
      .add_opening(workspace.id(), opened_window_ids);

    // Redraw the workspace's windows to hide them for the duration of the
    // animation.
    if did_start {
      state
        .pending_sync
        .queue_containers_to_redraw(workspace.tiling_children());
    }
  }

  let displayed_workspaces = state
    .workspaces()
    .into_iter()
    .filter(Workspace::is_displayed)
    .collect::<Vec<_>>();

  let mut strip_rects = HashMap::new();
  for workspace in displayed_workspaces.iter().filter(|w| w.is_scrolling())
  {
    for (window, rect) in tiling_window_rects(workspace)? {
      strip_rects.insert(window.id(), rect);
    }
  }

  state.scroll_animations.record_sync(
    displayed_workspaces.iter().map(CommonGetters::id).collect(),
    strip_rects,
  );

  Ok(())
}

/// Gets the slide targets for a scrolling workspace's tiling windows,
/// sliding from where they were placed at the last sync.
///
/// Opening windows are excluded, since they appear once the animation
/// ends. Windows that weren't in the strip before (e.g. moved from
/// another workspace) appear in place.
fn slide_targets(
  window_rects: Vec<(WindowContainer, Rect)>,
  opened_window_ids: &[Uuid],
  state: &WmState,
) -> Vec<SlideTarget> {
  window_rects
    .into_iter()
    .filter(|(window, _)| {
      !opened_window_ids.contains(&window.id())
        && !state.scroll_animations.is_opening(window.id())
    })
    .map(|(window, to)| {
      let from = state
        .scroll_animations
        .last_strip_rect(window.id())
        .cloned()
        .unwrap_or_else(|| to.clone());

      SlideTarget {
        window: window.native().clone(),
        from,
        to,
      }
    })
    .collect()
}

/// Cancels animations of workspaces that are no longer displayed or no
/// longer use the scrolling layout, and queues their windows for redraw.
fn cancel_stale_animations(state: &mut WmState) {
  for workspace_id in state.scroll_animations.animating_workspace_ids() {
    let workspace = state
      .workspaces()
      .into_iter()
      .find(|workspace| workspace.id() == workspace_id);

    let is_stale = workspace.as_ref().is_none_or(|workspace| {
      !workspace.is_scrolling() || !workspace.is_displayed()
    });

    if is_stale {
      state.scroll_animations.cancel(workspace_id);

      if let Some(workspace) = workspace {
        state
          .pending_sync
          .queue_containers_to_redraw(workspace.tiling_children());
      }
    }
  }
}

/// Gets the tiling windows of a workspace with the rects their outer
/// frames are positioned at.
fn tiling_window_rects(
  workspace: &Workspace,
) -> anyhow::Result<Vec<(WindowContainer, Rect)>> {
  workspace
    .descendants()
    .filter_map(|descendant| descendant.as_window_container().ok())
    .filter(WindowContainer::is_tiling_window)
    .map(|window| {
      let rect = window_frame_rect(&window)?;
      Ok((window, rect))
    })
    .collect()
}

/// Decides how a window is shown based on its column's position relative
/// to its scrolling workspace's viewport.
///
/// Windows that aren't tiling windows of a scrolling workspace are always
/// `PeekDecision::Shown`.
///
/// # Platform-specific
///
/// - **macOS**: Always `PeekDecision::Shown`; columns outside the viewport
///   are left positioned outside of it, since windows can't be cloaked.
fn window_peek(
  window: &WindowContainer,
  workspace: &Workspace,
  state: &WmState,
) -> anyhow::Result<PeekDecision> {
  if cfg!(target_os = "macos")
    || !workspace.is_scrolling()
    || !window.is_tiling_window()
  {
    return Ok(PeekDecision::Shown);
  }

  let Some(column) = workspace.column_of(&window.clone().into()) else {
    return Ok(PeekDecision::Shown);
  };

  match workspace.column_visibility(&column)? {
    Some(ColumnVisibility::Full) | None => Ok(PeekDecision::Shown),
    Some(ColumnVisibility::Hidden) => Ok(PeekDecision::Hidden),
    Some(ColumnVisibility::Partial) => {
      let monitor = workspace.monitor().context("No monitor.")?;
      let other_monitor_rects = state
        .monitors()
        .into_iter()
        .filter(|other| other.id() != monitor.id())
        .map(|other| other.to_rect())
        .try_collect::<Vec<_>>()?;

      Ok(scrolling::peek_decision(
        &window_frame_rect(window)?,
        &monitor.to_rect()?,
        &other_monitor_rects,
        workspace.peek_mode(),
      ))
    }
  }
}

/// Gets the rect to position a window's outer frame at, including its
/// border delta.
fn window_frame_rect(window: &WindowContainer) -> anyhow::Result<Rect> {
  Ok(
    window
      .to_rect()?
      .apply_delta(&window.total_border_delta()?, None),
  )
}

/// Shows live previews in place of the real windows of peeking columns
/// that would otherwise extend onto a neighboring monitor, and removes
/// previews that are no longer needed.
fn sync_window_previews(state: &mut WmState) -> anyhow::Result<()> {
  let mut desired = Vec::new();

  // Animating workspaces show previews of all their windows instead.
  let displayed_scrolling_workspaces =
    state.workspaces().into_iter().filter(|workspace| {
      workspace.is_scrolling()
        && workspace.is_displayed()
        && !state.scroll_animations.is_animating(workspace.id())
    });

  for workspace in displayed_scrolling_workspaces {
    let tiling_windows = workspace
      .descendants()
      .filter_map(|descendant| descendant.as_window_container().ok())
      .filter(WindowContainer::is_tiling_window);

    for window in tiling_windows {
      if let PeekDecision::Preview(visible_rect) =
        window_peek(&window, &workspace, state)?
      {
        desired.push(DesiredPreview {
          window_id: window.id(),
          native_window: window.native().clone(),
          source_rect: window_frame_rect(&window)?,
          visible_rect,
        });
      }
    }
  }

  let dispatcher = state.dispatcher.clone();
  state.window_previews.sync(desired, &dispatcher);

  Ok(())
}

fn sync_focus(
  focused_container: &Container,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let native_window = focused_container.as_window_container().ok();

  // Sets focus to the appropriate target:
  // - If the container is a window, focuses that window.
  // - If the container is a workspace, "resets" focus by focusing the
  //   desktop window.
  //
  // In either case, a `PlatformEvent::WindowFocused` event is subsequently
  // triggered.
  let result = if let Some(window) = native_window {
    tracing::info!("Setting focus to window: {window}");
    window.native().focus()
  } else {
    tracing::info!("Setting focus to the desktop window.");
    state.dispatcher.reset_focus()
  };

  if let Err(err) = result {
    tracing::warn!("Failed to set focus: {}", err);
  }

  state.emit_event(WmEvent::FocusChanged {
    focused_container: focused_container.to_dto()?,
  });

  Ok(())
}

/// Finds windows that should be brought to the top of their workspace's
/// z-order.
///
/// Windows are brought to front if they match the focused window's state
/// (floating/tiling) and any of these conditions are met:
///  * Focus has changed to a different window.
///  * Focused window's state has changed (e.g. tiling -> floating).
///  * Focused window has moved to a different workspace.
fn windows_to_bring_to_front(
  focused_container: &Container,
  state: &WmState,
) -> anyhow::Result<Vec<WindowContainer>> {
  let focused_workspace =
    focused_container.workspace().context("No workspace.")?;

  // Add focused workspace if there's been a focus change.
  let workspaces_to_reorder = state
    .pending_sync
    .workspaces_to_reorder()
    .iter()
    .chain(
      state
        .pending_sync
        .needs_focus_update()
        .then_some(&focused_workspace),
    )
    .unique_by(|workspace| workspace.id());

  // Bring forward windows that match the focused state. Only do this for
  // tiling/floating windows.
  let windows_to_bring_to_front = workspaces_to_reorder
    .flat_map(|workspace| {
      let focused_descendant = workspace
        .descendant_focus_order()
        .next()
        .and_then(|container| container.as_window_container().ok());

      match focused_descendant {
        Some(focused_descendant) => workspace
          .descendants()
          .filter_map(|descendant| descendant.as_window_container().ok())
          .filter(|window| {
            let is_floating_or_tiling = matches!(
              window.state(),
              WindowState::Floating(_) | WindowState::Tiling
            );

            is_floating_or_tiling
              && window.state().is_same_state(&focused_descendant.state())
          })
          .collect(),
        None => vec![],
      }
    })
    .collect::<Vec<_>>();

  Ok(windows_to_bring_to_front)
}

#[allow(clippy::too_many_lines)]
fn redraw_containers(
  focused_container: &Container,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let windows_to_redraw = state.windows_to_redraw();
  let windows_to_bring_to_front =
    windows_to_bring_to_front(focused_container, state)?;

  let windows_to_update = {
    let mut windows = windows_to_redraw
      .iter()
      .chain(&windows_to_bring_to_front)
      .unique_by(|window| window.id())
      .collect::<Vec<_>>();

    let descendant_focus_order = state
      .root_container
      .descendant_focus_order()
      .collect::<Vec<_>>();

    // Sort the windows to update by their focus order. Windows are
    // updated in reverse, so the most recently focused window is updated
    // last. Exceptions are opening windows (see `ScrollAnimations`),
    // which are updated first: they're still at the position the OS
    // opened them at, and there's no preview covering them. Hiding them
    // last would leave them visible there while all other windows are
    // being hidden (~10 ms each).
    // TODO: To reduce flicker, redraw windows that will be shown first,
    // then redraw the ones to be hidden last.
    windows.sort_by_key(|window| {
      (
        state.scroll_animations.is_opening(window.id()),
        descendant_focus_order
          .iter()
          .position(|order| order.id() == window.id()),
      )
    });

    windows
  };

  // Get monitors by their optimal hide corner.
  let monitors_by_hide_corner = state.monitors_by_hide_corner();

  for window in windows_to_update.iter().rev() {
    let should_bring_to_front = windows_to_bring_to_front.contains(window);

    let workspace =
      window.workspace().context("Window has no workspace.")?;

    let monitor = window.monitor().context("No monitor.")?;
    let hide_corner = monitors_by_hide_corner
      .iter()
      .find(|(m, _)| m.id() == monitor.id())
      .map(|(_, hide_corner)| hide_corner)
      .context("Monitor not found in hide corner map.")?;

    // Whether the window should be shown above all other windows.
    let z_order = match window.state() {
      WindowState::Floating(config) if config.shown_on_top => {
        WindowZOrder::TopMost
      }
      WindowState::Fullscreen(config) if config.shown_on_top => {
        WindowZOrder::TopMost
      }
      _ if should_bring_to_front => {
        let focused_descendant = workspace
          .descendant_focus_order()
          .next()
          .and_then(|container| container.as_window_container().ok());

        if let Some(focused_descendant) = focused_descendant {
          if window.id() == focused_descendant.id() {
            WindowZOrder::Normal
          } else {
            WindowZOrder::AfterWindow(focused_descendant.native().id())
          }
        } else {
          WindowZOrder::Normal
        }
      }
      _ => WindowZOrder::Normal,
    };

    // Set the z-order of the window.
    //
    // Windows that are hidden by the WM are skipped (e.g. scrolled-out
    // columns, or windows of an animating strip). Setting their z-order
    // would show them again (`SWP_SHOWWINDOW` uncloaks them) and raise
    // them above the scroll animation's previews. Their z-order is set
    // when they're redrawn to be shown.
    //
    // NOTE: macOS doesn't have a robust public API for setting the z-order
    // of a window. See `NativeWindow::raise` for more details.
    #[cfg(target_os = "windows")]
    if should_bring_to_front
      && !windows_to_redraw.contains(window)
      && matches!(
        window.display_state(),
        DisplayState::Shown | DisplayState::Showing
      )
    {
      tracing::info!("Updating window z-order: {window}");

      if let Err(err) = window.native().set_z_order(&z_order) {
        tracing::warn!("Failed to set window z-order: {}", err);
      }
    }

    // Skip updating the window's position if it only required a z-order
    // change.
    if !windows_to_redraw.contains(window) {
      continue;
    }

    // Windows in columns outside the viewport are hidden (and parked)
    // even though their workspace is displayed. Peeking windows that are
    // replaced by a preview are hidden too (see `sync_window_previews`).
    let is_parked = workspace.is_displayed()
      && window_peek(window, &workspace, state)? != PeekDecision::Shown;

    // While the strip animates, its windows are hidden at their final
    // positions and shown as previews instead.
    let is_animating = window.is_tiling_window()
      && state.scroll_animations.is_animating(workspace.id());

    let is_scrolled_out =
      is_parked || (workspace.is_displayed() && is_animating);

    // Transition display state depending on whether window will be
    // shown or hidden.
    window.set_display_state(
      match (
        window.display_state(),
        workspace.is_displayed() && !is_scrolled_out,
      ) {
        (DisplayState::Hidden | DisplayState::Hiding, true) => {
          DisplayState::Showing
        }
        (DisplayState::Shown | DisplayState::Showing, false) => {
          DisplayState::Hiding
        }
        _ => window.display_state(),
      },
    );

    let is_visible = matches!(
      window.display_state(),
      DisplayState::Showing | DisplayState::Shown
    );

    tracing::debug!(
      "Redrawing window {}: visible={}, scrolled_out={}, parked={}, \
       animating={}.",
      window.native().id().0,
      is_visible,
      is_scrolled_out,
      is_parked,
      is_animating,
    );

    if let Err(err) = reposition_window(
      window,
      *hide_corner,
      &z_order,
      is_visible,
      is_scrolled_out,
      is_parked,
      config,
    ) {
      tracing::warn!("Failed to set window position: {}", err);
    }

    // Whether the window is either transitioning to or from fullscreen.
    // TODO: This check can be improved since `prev_state` can be
    // fullscreen without it needing to be marked as not fullscreen.
    #[cfg(target_os = "windows")]
    {
      let is_transitioning_fullscreen =
        match (window.prev_state(), window.state()) {
          (Some(_), WindowState::Fullscreen(s)) if !s.maximized => true,
          (Some(WindowState::Fullscreen(_)), _) => true,
          _ => false,
        };

      if is_transitioning_fullscreen {
        if let Err(err) = window.native().mark_fullscreen(matches!(
          window.state(),
          WindowState::Fullscreen(_)
        )) {
          tracing::warn!("Failed to mark window as fullscreen: {}", err);
        }
      }
    }

    // Skip setting taskbar visibility if the window is hidden (has no
    // effect). Since cloaked windows are normally always visible in the
    // taskbar, we only need to set visibility if `show_all_in_taskbar` is
    // `false`. Scrolled-out windows stay in the taskbar so that they can
    // be scrolled to by clicking them.
    #[cfg(target_os = "windows")]
    if config.value.general.hide_method == HideMethod::Cloak
      && !config.value.general.show_all_in_taskbar
      && matches!(
        window.display_state(),
        DisplayState::Showing | DisplayState::Hiding
      )
    {
      if let Err(err) = window
        .native()
        .set_taskbar_visibility(is_visible || is_scrolled_out)
      {
        tracing::warn!("Failed to set taskbar visibility: {}", err);
      }
    }
  }

  Ok(())
}

/// Moves and resizes a window to its target rect, and shows or hides it.
///
/// Scrolled-out windows (`is_scrolled_out`) are cloaked regardless of the
/// hide method. This keeps their DWM thumbnails live and keeps them
/// visible in the taskbar. Those in columns outside the viewport
/// (`is_parked`) are also moved within the viewport, so that they stay
/// on-screen if the WM exits without restoring them.
fn reposition_window(
  window: &WindowContainer,
  hide_corner: HideCorner,
  // LINT: `z_order` is only used on Windows.
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  z_order: &WindowZOrder,
  is_visible: bool,
  // LINT: `is_scrolled_out` is only used on Windows.
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  is_scrolled_out: bool,
  is_parked: bool,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let rect = window
    .to_rect()?
    .apply_delta(&window.total_border_delta()?, None);

  // Windows outside the viewport are parked within it while cloaked, so
  // that they never end up off-screen (e.g. if the WM crashes).
  let rect = if is_parked {
    window
      .workspace()
      .context("No workspace.")?
      .parked_column_rect(&rect)?
  } else {
    rect
  };

  // For `HideMethod::PlaceInCorner`, we need to reposition windows on
  // hidden workspaces to the corner of the monitor.
  if config.value.general.hide_method == HideMethod::PlaceInCorner
    && !is_visible
    && !is_scrolled_out
  {
    const VISIBLE_SLIVER: i32 = 1;

    let monitor_rect = window
      .monitor()
      .context("No monitor.")?
      .native_properties()
      .working_area;

    let frame = window.native_properties().frame;

    let position_y = monitor_rect.bottom - VISIBLE_SLIVER;
    let position_x = match hide_corner {
      HideCorner::BottomLeft => {
        monitor_rect.left + VISIBLE_SLIVER - frame.width()
      }
      HideCorner::BottomRight => monitor_rect.right - VISIBLE_SLIVER,
    };

    // Even though the window size is unchanged, `NativeWindow::set_frame`
    // is used instead of `NativeWindow::reposition` because the latter
    // resulted in occasional incorrect positionings on macOS.
    window.native().set_frame(&Rect::from_xy(
      position_x,
      position_y,
      frame.width(),
      frame.height(),
    ))?;

    return Ok(());
  }

  if window.active_drag().is_some() {
    window.native().resize(rect.width(), rect.height())?;
  } else {
    #[cfg(target_os = "macos")]
    window.native().set_frame(&rect)?;

    #[cfg(target_os = "windows")]
    {
      use wm_platform::{
        SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOCOPYBITS, SWP_NOSENDCHANGING, WS_MAXIMIZEBOX,
      };

      // Restore window if it's minimized/maximized and shouldn't be. This
      // is needed to be able to move and resize it.
      let should_restore = match &window.state() {
        // Need to restore window if transitioning from maximized
        // fullscreen to non-maximized fullscreen.
        WindowState::Fullscreen(fullscreen) => {
          !fullscreen.maximized && window.native().is_maximized()?
        }
        // No need to restore window if it'll be minimized. Transitioning
        // from maximized to minimized works without having to
        // restore.
        WindowState::Minimized => false,
        _ => {
          window.native().is_minimized()?
            || window.native().is_maximized()?
        }
      };

      if should_restore {
        // Restoring to position has the same effect as `ShowWindow` with
        // `SW_RESTORE`, but doesn't cause a flicker.
        window.native().restore(Some(&rect))?;
      }

      let mut swp_flags = SWP_NOACTIVATE
        | SWP_NOCOPYBITS
        | SWP_NOSENDCHANGING
        | SWP_ASYNCWINDOWPOS;

      match &window.state() {
        WindowState::Minimized => {
          if !window.native().is_minimized()? {
            window.native().minimize()?;
          }
        }
        WindowState::Fullscreen(fullscreen)
          if fullscreen.maximized
            && window.native().has_window_style(WS_MAXIMIZEBOX) =>
        {
          if !window.native().is_maximized()? {
            window.native().maximize()?;
          }

          window.native().set_window_pos(z_order, &rect, swp_flags)?;
        }
        _ => {
          swp_flags |= SWP_FRAMECHANGED;

          window.native().set_window_pos(z_order, &rect, swp_flags)?;

          // When there's a mismatch between the DPI of the monitor and the
          // window, the window might be sized incorrectly after the first
          // move. If we set the position twice, inconsistencies after the
          // first move are resolved.
          if window.has_pending_dpi_adjustment() {
            window.native().set_window_pos(z_order, &rect, swp_flags)?;
          }
        }
      }

      set_visibility(window, is_visible, is_scrolled_out, config)?;
    }
  }

  Ok(())
}

/// Shows, hides, or cloaks a window based on the hide method.
///
/// Scrolled-out windows are always cloaked (and otherwise shown). The
/// order of calls avoids the window briefly flashing into view.
#[cfg(target_os = "windows")]
fn set_visibility(
  window: &WindowContainer,
  is_visible: bool,
  is_scrolled_out: bool,
  config: &UserConfig,
) -> anyhow::Result<()> {
  if config.value.general.hide_method == HideMethod::Cloak {
    window.native().set_cloaked(!is_visible)?;
  } else if is_scrolled_out {
    window.native().set_cloaked(true)?;
    window.native().show()?;
  } else {
    if is_visible {
      window.native().show()?;
    } else {
      window.native().hide()?;
    }

    // Uncloak windows that were previously scrolled out.
    if window.native().is_cloaked().unwrap_or(false) {
      window.native().set_cloaked(false)?;
    }
  }

  Ok(())
}

fn jump_cursor(
  focused_container: Container,
  state: &WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let cursor_jump = &config.value.general.cursor_jump;

  let jump_target = match cursor_jump.trigger {
    CursorJumpTrigger::WindowFocus => Some(focused_container),
    CursorJumpTrigger::MonitorFocus => {
      let target_monitor =
        focused_container.monitor().context("No monitor.")?;

      let cursor_monitor = state
        .dispatcher
        .cursor_position()
        .ok()
        .and_then(|pos| state.monitor_at_point(&pos));

      // Jump to the target monitor if the cursor is not already on it.
      cursor_monitor
        .filter(|monitor| monitor.id() != target_monitor.id())
        .map(|_| target_monitor.into())
    }
  };

  if let Some(jump_target) = jump_target {
    let center = jump_target.to_rect()?.center_point();

    if let Err(err) = state.dispatcher.set_cursor_position(&center) {
      tracing::warn!("Failed to set cursor position: {}", err);
    }
  }

  Ok(())
}

fn apply_window_effects(
  // LINT: `window` is only used on Windows.
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  window: &WindowContainer,
  is_focused: bool,
  config: &UserConfig,
) {
  let window_effects = &config.value.window_effects;

  // LINT: `effect_config` is only used on Windows.
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  let effect_config = if is_focused {
    &window_effects.focused_window
  } else {
    &window_effects.other_windows
  };

  // Skip if both focused + non-focused window effects are disabled.
  #[cfg(target_os = "windows")]
  if window_effects.focused_window.border.enabled
    || window_effects.other_windows.border.enabled
  {
    apply_border_effect(window, effect_config);
  }

  #[cfg(target_os = "windows")]
  if window_effects.focused_window.hide_title_bar.enabled
    || window_effects.other_windows.hide_title_bar.enabled
  {
    apply_hide_title_bar_effect(window, effect_config);
  }

  #[cfg(target_os = "windows")]
  if window_effects.focused_window.corner_style.enabled
    || window_effects.other_windows.corner_style.enabled
  {
    apply_corner_effect(window, effect_config);
  }

  #[cfg(target_os = "windows")]
  if window_effects.focused_window.transparency.enabled
    || window_effects.other_windows.transparency.enabled
  {
    apply_transparency_effect(window, effect_config);
  }
}

#[cfg(target_os = "windows")]
fn apply_border_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  let border_color = if effect_config.border.enabled {
    Some(&effect_config.border.color)
  } else {
    None
  };

  _ = window.native().set_border_color(border_color);

  let native = window.native().clone();
  let border_color = border_color.cloned();

  // Re-apply border color after a short delay to better handle
  // windows that change it themselves.
  tokio::task::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    _ = native.set_border_color(border_color.as_ref());
  });
}

#[cfg(target_os = "windows")]
fn apply_hide_title_bar_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  _ = window
    .native()
    .set_title_bar_visibility(!effect_config.hide_title_bar.enabled);
}

#[cfg(target_os = "windows")]
fn apply_corner_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  let corner_style = if effect_config.corner_style.enabled {
    &effect_config.corner_style.style
  } else {
    &CornerStyle::Default
  };

  _ = window.native().set_corner_style(corner_style);
}

#[cfg(target_os = "windows")]
fn apply_transparency_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  let transparency = if effect_config.transparency.enabled {
    &effect_config.transparency.opacity
  } else {
    // Reset the transparency to default.
    &OpacityValue::from_alpha(u8::MAX)
  };

  _ = window.native().set_transparency(transparency);
}
