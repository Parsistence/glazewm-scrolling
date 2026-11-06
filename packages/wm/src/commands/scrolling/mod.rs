mod consume_into_column;
mod cycle_column_width;
mod expel_from_column;
mod scroll_in_direction;
mod set_layout;

pub use consume_into_column::*;
pub use cycle_column_width::*;
pub use expel_from_column::*;
pub use scroll_in_direction::*;
pub use set_layout::*;

#[cfg(test)]
mod tests {
  use uuid::Uuid;
  use wm_common::WorkspaceLayout;
  use wm_platform::Direction;

  use super::{
    consume_into_column, expel_from_column, scroll_in_direction,
  };
  use crate::{
    commands::{
      container::set_focused_descendant, window::move_window_in_direction,
    },
    models::{Monitor, TilingContainer, TilingWindow, Workspace},
    traits::{CommonGetters, TilingSizeGetters},
    user_config::UserConfig,
    wm_state::WmState,
  };

  /// Creates a state with a scrolling workspace of single-window columns,
  /// returning the workspace and its windows from left to right.
  fn setup(
    column_count: usize,
  ) -> (WmState, Workspace, Vec<TilingWindow>) {
    let windows = (0..column_count)
      .map(|_| TilingWindow::mock().call())
      .collect::<Vec<_>>();

    let workspace = Workspace::mock()
      .layout(WorkspaceLayout::Scrolling)
      .tiling_containers(
        windows.iter().cloned().map(TilingContainer::from).collect(),
      )
      .call();

    let monitor =
      Monitor::mock().workspaces(vec![workspace.clone()]).call();
    (WmState::mock(vec![monitor]), workspace, windows)
  }

  /// Gets the IDs of the windows in each column, from left to right.
  fn columns(workspace: &Workspace) -> Vec<Vec<Uuid>> {
    workspace
      .tiling_children()
      .map(|column| match column {
        TilingContainer::TilingWindow(window) => vec![window.id()],
        TilingContainer::Split(split) => {
          split.tiling_children().map(|child| child.id()).collect()
        }
      })
      .collect()
  }

  #[test]
  fn consume_stacks_window_at_bottom_of_adjacent_column() {
    let (mut state, workspace, windows) = setup(3);
    let config = UserConfig::mock();
    let first_width = windows[0].tiling_size();

    consume_into_column(
      &windows[1],
      &Direction::Left,
      &mut state,
      &config,
    )
    .unwrap();

    assert_eq!(
      columns(&workspace),
      vec![
        vec![windows[0].id(), windows[1].id()],
        vec![windows[2].id()]
      ]
    );

    // The stack keeps the width of the column it was consumed into.
    let stack = workspace.tiling_children().next().unwrap();
    assert!((stack.tiling_size() - first_width).abs() < f32::EPSILON);
  }

  #[test]
  fn consume_at_strip_edge_is_noop() {
    let (mut state, workspace, windows) = setup(2);
    let config = UserConfig::mock();

    consume_into_column(
      &windows[0],
      &Direction::Left,
      &mut state,
      &config,
    )
    .unwrap();

    assert_eq!(
      columns(&workspace),
      vec![vec![windows[0].id()], vec![windows[1].id()]]
    );
  }

  #[test]
  fn expel_moves_window_into_new_column() {
    let (mut state, workspace, windows) = setup(3);
    let config = UserConfig::mock();
    consume_into_column(
      &windows[2],
      &Direction::Left,
      &mut state,
      &config,
    )
    .unwrap();

    expel_from_column(&windows[2], &Direction::Left, &mut state).unwrap();

    assert_eq!(
      columns(&workspace),
      vec![
        vec![windows[0].id()],
        vec![windows[2].id()],
        vec![windows[1].id()],
      ]
    );

    // The original column is flattened back into a single window.
    let original_column = workspace.tiling_children().nth(2).unwrap();
    assert_eq!(original_column.id(), windows[1].id());
  }

  #[test]
  fn expel_of_single_window_column_is_noop() {
    let (mut state, workspace, windows) = setup(2);

    expel_from_column(&windows[0], &Direction::Right, &mut state).unwrap();

    assert_eq!(
      columns(&workspace),
      vec![vec![windows[0].id()], vec![windows[1].id()]]
    );
  }

  #[test]
  fn scroll_focuses_adjacent_column_and_stops_at_edges() {
    let (mut state, workspace, windows) = setup(3);
    set_focused_descendant(&windows[1].clone().into(), None);

    scroll_in_direction(
      &windows[1].clone().into(),
      &Direction::Right,
      &mut state,
    );
    assert_eq!(
      workspace.focused_column().map(|column| column.id()),
      Some(windows[2].id())
    );

    scroll_in_direction(
      &windows[2].clone().into(),
      &Direction::Right,
      &mut state,
    );
    assert_eq!(
      workspace.focused_column().map(|column| column.id()),
      Some(windows[2].id())
    );
  }

  #[test]
  fn move_swaps_whole_columns() {
    let (mut state, workspace, windows) = setup(3);
    let config = UserConfig::mock();
    consume_into_column(
      &windows[1],
      &Direction::Left,
      &mut state,
      &config,
    )
    .unwrap();

    // Moving a stacked window right swaps its entire column.
    move_window_in_direction(
      windows[1].clone().into(),
      &Direction::Right,
      &mut state,
      &config,
    )
    .unwrap();

    assert_eq!(
      columns(&workspace),
      vec![
        vec![windows[2].id()],
        vec![windows[0].id(), windows[1].id()]
      ]
    );
  }
}
