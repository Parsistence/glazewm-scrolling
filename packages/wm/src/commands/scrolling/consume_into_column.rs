use tracing::warn;
use wm_common::{ColumnTarget, TilingDirection};
use wm_platform::Direction;

use super::{adjacent_column, window_at_column_edge};
use crate::{
  commands::container::{
    flatten_child_split_containers, move_container_within_tree,
    wrap_in_split_container,
  },
  models::{Container, SplitContainer, TilingContainer, TilingWindow},
  traits::{CommonGetters, TilingDirectionGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

/// Moves a tiling window into the adjacent column in the given direction,
/// stacking it at the bottom of that column.
///
/// If the adjacent column is a single window, it's turned into a vertical
/// stack. No-op if there's no adjacent column, or if the window's
/// workspace doesn't use the scrolling layout.
pub fn consume_into_column(
  window: &TilingWindow,
  direction: &Direction,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  if matches!(direction, Direction::Up | Direction::Down) {
    warn!("Can only consume into columns to the left or right.");
    return Ok(());
  }

  let Some(workspace) = window.scrolling_workspace() else {
    return Ok(());
  };

  let window_container: Container = window.clone().into();

  let Some(target_column) =
    adjacent_column(&workspace, &window_container, direction)
  else {
    return Ok(());
  };

  let target_stack =
    stack_for_column(&target_column, &workspace.clone().into(), config)?;

  move_container_within_tree(
    &window_container,
    &target_stack.clone().into(),
    target_stack.child_count(),
    state,
  )?;

  // Ensure the window's original column isn't left as a redundant split
  // container with a single child.
  flatten_child_split_containers(&workspace.clone().into())?;

  state
    .pending_sync
    .queue_containers_to_redraw(workspace.tiling_children());

  Ok(())
}

/// Pulls the top window of the adjacent column in the given direction
/// into the subject's column, stacking it at the bottom.
///
/// Focus stays on the focused window. No-op if there's no adjacent
/// column, or if the subject's workspace doesn't use the scrolling layout.
pub fn pull_into_column(
  subject_container: &Container,
  direction: &Direction,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  if matches!(direction, Direction::Up | Direction::Down) {
    warn!("Can only pull from columns to the left or right.");
    return Ok(());
  }

  let Some(workspace) = subject_container.scrolling_workspace() else {
    return Ok(());
  };

  let Some(source_window) =
    adjacent_column(&workspace, subject_container, direction).and_then(
      |column| window_at_column_edge(&column, ColumnTarget::First),
    )
  else {
    return Ok(());
  };

  consume_into_column(&source_window, &direction.inverse(), state, config)
}

/// Gets the vertical split container that windows should be stacked into
/// for the given column, wrapping the column in one if needed.
fn stack_for_column(
  column: &TilingContainer,
  workspace: &Container,
  config: &UserConfig,
) -> anyhow::Result<SplitContainer> {
  if let TilingContainer::Split(split) = column {
    if split.tiling_direction() == TilingDirection::Vertical {
      return Ok(split.clone());
    }
  }

  let stack = SplitContainer::new(
    TilingDirection::Vertical,
    config.value.gaps.clone(),
  );

  wrap_in_split_container(
    &stack,
    workspace,
    std::slice::from_ref(column),
  )?;

  Ok(stack)
}
