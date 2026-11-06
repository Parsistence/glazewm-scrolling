use anyhow::Context;
use wm_common::{TilingDirection, WmEvent};

use super::{flatten_split_container, wrap_in_split_container};
use crate::{
  models::{Container, DirectionContainer, SplitContainer, TilingWindow},
  traits::{CommonGetters, TilingDirectionGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

pub fn toggle_tiling_direction(
  container: Container,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  // Scrolling workspaces always arrange their columns horizontally. Use
  // the column commands (e.g. `consume-into-column`) to stack windows.
  let is_scrolling_level = match &container {
    Container::Workspace(workspace) => workspace.is_scrolling(),
    Container::TilingWindow(window) => window.scrolling_parent().is_some(),
    _ => false,
  };

  if is_scrolling_level {
    tracing::warn!(
      "Cannot change tiling direction of a scrolling workspace's columns."
    );
    return Ok(());
  }

  let direction_container = match container {
    Container::TilingWindow(tiling_window) => {
      toggle_window_direction(tiling_window, config)
    }
    Container::Workspace(workspace) => {
      workspace
        .set_tiling_direction(workspace.tiling_direction().inverse());

      Ok(workspace.into())
    }
    // Can only toggle tiling direction from a tiling window or workspace.
    _ => return Ok(()),
  }?;

  state.emit_event(WmEvent::TilingDirectionChanged {
    direction_container: direction_container.to_dto()?,
    new_tiling_direction: direction_container.tiling_direction(),
  });

  Ok(())
}

fn toggle_window_direction(
  tiling_window: TilingWindow,
  config: &UserConfig,
) -> anyhow::Result<DirectionContainer> {
  let parent = tiling_window
    .direction_container()
    .context("No direction container.")?;

  // If the window is an only child, then either change the tiling
  // direction of its parent workspace or flatten its parent split
  // container.
  if tiling_window.tiling_siblings().count() == 0 {
    return match parent {
      DirectionContainer::Workspace(workspace) => {
        workspace
          .set_tiling_direction(workspace.tiling_direction().inverse());

        Ok(workspace.into())
      }
      DirectionContainer::Split(split_container) => {
        flatten_split_container(split_container.clone())?;

        tiling_window
          .direction_container()
          .context("No direction container.")
      }
    };
  }

  // Create a new split container to wrap the window.
  let split_container = SplitContainer::new(
    parent.tiling_direction().inverse(),
    config.value.gaps.clone(),
  );

  wrap_in_split_container(
    &split_container,
    &parent.into(),
    &[tiling_window.into()],
  )?;

  Ok(split_container.into())
}

pub fn set_tiling_direction(
  container: Container,
  state: &mut WmState,
  config: &UserConfig,
  tiling_direction: &TilingDirection,
) -> anyhow::Result<()> {
  let direction_container = container
    .direction_container()
    .context("No direction container.")?;

  if direction_container.tiling_direction() == *tiling_direction {
    Ok(())
  } else {
    toggle_tiling_direction(container, state, config)
  }
}
