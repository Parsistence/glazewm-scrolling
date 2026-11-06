use tracing::warn;
use wm_platform::Direction;

use crate::{
  commands::container::{
    flatten_child_split_containers, move_container_within_tree,
  },
  models::{Container, TilingWindow},
  traits::CommonGetters,
  wm_state::WmState,
};

/// Moves a tiling window out of its column into a new column, placed to
/// the left or right of the original column.
///
/// The new column gets the default column width. No-op if the window is
/// already the only window in its column, or if the window's workspace
/// doesn't use the scrolling layout.
pub fn expel_from_column(
  window: &TilingWindow,
  direction: &Direction,
  state: &mut WmState,
) -> anyhow::Result<()> {
  if matches!(direction, Direction::Up | Direction::Down) {
    warn!("Can only expel from columns to the left or right.");
    return Ok(());
  }

  let Some(workspace) = window.scrolling_workspace() else {
    return Ok(());
  };

  let window_container: Container = window.clone().into();

  let Some(column) = workspace.column_of(&window_container) else {
    return Ok(());
  };

  // The window is already its own column.
  if column.id() == window.id() {
    return Ok(());
  }

  let target_index = match direction {
    Direction::Left => column.index(),
    _ => column.index() + 1,
  };

  move_container_within_tree(
    &window_container,
    &workspace.clone().into(),
    target_index,
    state,
  )?;

  // Moving to the workspace (an ancestor) doesn't flatten the original
  // column, which might be left with a single child.
  flatten_child_split_containers(&workspace.clone().into())?;

  state
    .pending_sync
    .queue_containers_to_redraw(workspace.tiling_children());

  Ok(())
}
