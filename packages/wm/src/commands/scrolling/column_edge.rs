use wm_common::ColumnTarget;

use super::focus_target_in_column;
use crate::{
  commands::container::{
    move_container_within_tree, set_focused_descendant,
  },
  models::{Container, TilingContainer, TilingWindow, Workspace},
  traits::CommonGetters,
  wm_state::WmState,
};

/// Focuses the first or last column of the subject's workspace, causing
/// the viewport to scroll to it.
///
/// No-op if the workspace has no columns, or if it doesn't use the
/// scrolling layout.
pub fn focus_column_at_edge(
  subject_container: &Container,
  target: ColumnTarget,
  state: &mut WmState,
) {
  let Some(workspace) = subject_container.scrolling_workspace() else {
    return;
  };

  let Some(column) = column_at_edge(&workspace, target) else {
    return;
  };

  set_focused_descendant(&focus_target_in_column(&column), None);
  state.pending_sync.queue_focus_change().queue_cursor_jump();
}

/// Moves the column of a tiling window to the start or end of its
/// workspace's strip.
///
/// No-op if the window's workspace doesn't use the scrolling layout.
pub fn move_column_to_edge(
  window: &TilingWindow,
  target: ColumnTarget,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let Some(workspace) = window.scrolling_workspace() else {
    return Ok(());
  };

  let Some(column) = workspace.column_of(&window.clone().into()) else {
    return Ok(());
  };

  let target_index = match target {
    ColumnTarget::First => 0,
    ColumnTarget::Last => workspace.child_count(),
  };

  move_container_within_tree(
    &column.into(),
    &workspace.clone().into(),
    target_index,
    state,
  )?;

  state
    .pending_sync
    .queue_containers_to_redraw(workspace.tiling_children());

  Ok(())
}

/// Gets the first (top-left) or last (bottom-right) window of a column,
/// descending through nested split containers.
pub fn window_at_column_edge(
  column: &TilingContainer,
  target: ColumnTarget,
) -> Option<TilingWindow> {
  match column {
    TilingContainer::TilingWindow(window) => Some(window.clone()),
    TilingContainer::Split(split) => {
      let mut children = split.tiling_children();

      let child = match target {
        ColumnTarget::First => children.next(),
        ColumnTarget::Last => children.last(),
      }?;

      window_at_column_edge(&child, target)
    }
  }
}

/// Gets the first or last column of a workspace.
fn column_at_edge(
  workspace: &Workspace,
  target: ColumnTarget,
) -> Option<TilingContainer> {
  match target {
    ColumnTarget::First => workspace.tiling_children().next(),
    ColumnTarget::Last => workspace.tiling_children().last(),
  }
}
