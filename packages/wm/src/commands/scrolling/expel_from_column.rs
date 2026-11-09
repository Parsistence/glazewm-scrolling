use tracing::warn;
use wm_common::ColumnTarget;
use wm_platform::Direction;

use super::{focus_target_in_column, window_at_column_edge};
use crate::{
  commands::container::{
    flatten_child_split_containers, move_container_within_tree,
    set_focused_descendant,
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

/// Moves the last (bottom) window of the subject's column out into a new
/// column, placed to the left or right of the original column.
///
/// Focus stays on the column's focused window, even if it's the one
/// moved. No-op if the column is a single window, or if the subject's
/// workspace doesn't use the scrolling layout.
pub fn expel_last_from_column(
  subject_container: &Container,
  direction: &Direction,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let Some(workspace) = subject_container.scrolling_workspace() else {
    return Ok(());
  };

  let Some(column) = workspace
    .column_of(subject_container)
    .or_else(|| workspace.focused_column())
  else {
    return Ok(());
  };

  let Some(window) = window_at_column_edge(&column, ColumnTarget::Last)
  else {
    return Ok(());
  };

  let focused_window = focus_target_in_column(&column);
  let is_focused_column = workspace
    .focused_column()
    .is_some_and(|focused| focused.id() == column.id());

  expel_from_column(&window, direction, state)?;

  // The expelled window takes over its old column's place in the
  // workspace's focus order. Give it back to the column's focused window.
  if is_focused_column && focused_window.id() != window.id() {
    if let Some(new_column) = workspace.column_of(&focused_window) {
      set_focused_descendant(&focused_window, Some(&new_column.into()));
    }
  }

  Ok(())
}
