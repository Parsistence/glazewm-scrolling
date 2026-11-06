use tracing::warn;
use wm_platform::Direction;

use crate::{
  commands::container::set_focused_descendant,
  models::{Container, TilingContainer, Workspace},
  traits::CommonGetters,
  wm_state::WmState,
};

/// Focuses the adjacent column in the given direction, causing the
/// viewport to scroll to it.
///
/// Unlike focusing in a direction, this never leaves the workspace and
/// doesn't move the cursor (e.g. when triggered by scrolling over a bar).
/// No-op at either end of the strip, or if the subject's workspace doesn't
/// use the scrolling layout.
pub fn scroll_in_direction(
  subject_container: &Container,
  direction: &Direction,
  state: &mut WmState,
) {
  if matches!(direction, Direction::Up | Direction::Down) {
    warn!("Scrolling is only supported horizontally.");
    return;
  }

  let Some(workspace) = subject_container.scrolling_workspace() else {
    return;
  };

  let Some(target_column) =
    adjacent_column(&workspace, subject_container, direction)
  else {
    return;
  };

  set_focused_descendant(&focus_target_in_column(&target_column), None);
  state.pending_sync.queue_focus_change();
}

/// Gets the column next to the subject's column in the given direction.
///
/// Falls back to the workspace's most recently focused column if the
/// subject isn't in a column (e.g. a floating window).
pub fn adjacent_column(
  workspace: &Workspace,
  subject_container: &Container,
  direction: &Direction,
) -> Option<TilingContainer> {
  let current_column = workspace
    .column_of(subject_container)
    .or_else(|| workspace.focused_column())?;

  match direction {
    Direction::Left => current_column
      .prev_siblings()
      .find_map(|sibling| sibling.as_tiling_container().ok()),
    Direction::Right => current_column
      .next_siblings()
      .find_map(|sibling| sibling.as_tiling_container().ok()),
    Direction::Up | Direction::Down => None,
  }
}

/// Gets the container to focus within a column: its most recently focused
/// window, or the column itself if it's a window.
pub fn focus_target_in_column(column: &TilingContainer) -> Container {
  column
    .descendant_focus_order()
    .find(Container::is_tiling_window)
    .unwrap_or_else(|| column.clone().into())
}
