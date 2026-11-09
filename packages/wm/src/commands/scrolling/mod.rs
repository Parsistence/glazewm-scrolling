mod center_column;
mod column_edge;
mod consume_into_column;
mod cycle_column_width;
mod expel_from_column;
mod scroll_in_direction;
mod set_layout;
mod toggle_column_full_width;

pub use center_column::*;
pub use column_edge::*;
pub use consume_into_column::*;
pub use cycle_column_width::*;
pub use expel_from_column::*;
pub use scroll_in_direction::*;
pub use set_layout::*;
pub use toggle_column_full_width::*;

#[cfg(test)]
mod tests {
  use uuid::Uuid;
  use wm_common::{ColumnTarget, WorkspaceLayout};
  use wm_platform::Direction;

  use super::{
    center_column, consume_into_column, expel_from_column,
    expel_last_from_column, focus_column_at_edge, move_column_to_edge,
    pull_into_column, scroll_in_direction, toggle_column_full_width,
  };
  use crate::{
    commands::{
      container::{resize_tiling_container, set_focused_descendant},
      window::move_window_in_direction,
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

  #[test]
  fn pull_stacks_top_window_of_adjacent_column() {
    let (mut state, workspace, windows) = setup(3);
    let config = UserConfig::mock();
    let first_width = windows[0].tiling_size();
    consume_into_column(
      &windows[2],
      &Direction::Left,
      &mut state,
      &config,
    )
    .unwrap();
    set_focused_descendant(&windows[0].clone().into(), None);

    pull_into_column(
      &windows[0].clone().into(),
      &Direction::Right,
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

    // The pulling column keeps its width and focus.
    let stack = workspace.tiling_children().next().unwrap();
    assert!((stack.tiling_size() - first_width).abs() < f32::EPSILON);
    assert_eq!(
      state.focused_container().map(|focused| focused.id()),
      Some(windows[0].id())
    );
  }

  #[test]
  fn pull_at_strip_edge_is_noop() {
    let (mut state, workspace, windows) = setup(2);
    let config = UserConfig::mock();

    pull_into_column(
      &windows[1].clone().into(),
      &Direction::Right,
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
  fn expel_last_moves_bottom_window_and_keeps_focus() {
    let (mut state, workspace, windows) = setup(3);
    let config = UserConfig::mock();
    for window in &windows[1..] {
      consume_into_column(window, &Direction::Left, &mut state, &config)
        .unwrap();
    }
    set_focused_descendant(&windows[0].clone().into(), None);

    expel_last_from_column(
      &windows[0].clone().into(),
      &Direction::Right,
      &mut state,
    )
    .unwrap();

    assert_eq!(
      columns(&workspace),
      vec![
        vec![windows[0].id(), windows[1].id()],
        vec![windows[2].id()]
      ]
    );
    assert_eq!(
      state.focused_container().map(|focused| focused.id()),
      Some(windows[0].id())
    );
  }

  #[test]
  fn expel_last_of_single_window_column_is_noop() {
    let (mut state, workspace, windows) = setup(2);

    expel_last_from_column(
      &windows[0].clone().into(),
      &Direction::Right,
      &mut state,
    )
    .unwrap();

    assert_eq!(
      columns(&workspace),
      vec![vec![windows[0].id()], vec![windows[1].id()]]
    );
  }

  #[test]
  fn focus_column_at_edge_focuses_first_and_last_columns() {
    let (mut state, workspace, windows) = setup(3);
    set_focused_descendant(&windows[1].clone().into(), None);

    focus_column_at_edge(
      &windows[1].clone().into(),
      ColumnTarget::Last,
      &mut state,
    );
    assert_eq!(
      workspace.focused_column().map(|column| column.id()),
      Some(windows[2].id())
    );

    focus_column_at_edge(
      &windows[2].clone().into(),
      ColumnTarget::First,
      &mut state,
    );
    assert_eq!(
      workspace.focused_column().map(|column| column.id()),
      Some(windows[0].id())
    );
  }

  #[test]
  fn move_column_to_edge_moves_whole_column() {
    let (mut state, workspace, windows) = setup(3);
    let config = UserConfig::mock();
    consume_into_column(
      &windows[1],
      &Direction::Left,
      &mut state,
      &config,
    )
    .unwrap();

    move_column_to_edge(&windows[1], ColumnTarget::Last, &mut state)
      .unwrap();
    assert_eq!(
      columns(&workspace),
      vec![
        vec![windows[2].id()],
        vec![windows[0].id(), windows[1].id()]
      ]
    );

    move_column_to_edge(&windows[0], ColumnTarget::First, &mut state)
      .unwrap();
    assert_eq!(
      columns(&workspace),
      vec![
        vec![windows[0].id(), windows[1].id()],
        vec![windows[2].id()]
      ]
    );
  }

  #[test]
  fn full_width_toggles_back_to_previous_width() {
    let (mut state, workspace, windows) = setup(2);
    let column = windows[0].clone().into();
    let original_width = windows[0].tiling_size();

    toggle_column_full_width(&column, &mut state);
    assert!((windows[0].tiling_size() - 1.0).abs() < f32::EPSILON);

    toggle_column_full_width(&column, &mut state);
    assert!(
      (windows[0].tiling_size() - original_width).abs() < f32::EPSILON
    );

    // A column made full-width some other way restores to the default
    // width.
    resize_tiling_container(&windows[0].clone().into(), 1.0);
    toggle_column_full_width(&column, &mut state);
    assert!(
      (windows[0].tiling_size() - workspace.default_column_size()).abs()
        < f32::EPSILON
    );
  }

  #[test]
  fn center_column_is_only_queued_for_scrolling_workspaces() {
    let (mut state, workspace, windows) = setup(2);

    center_column(&windows[0].clone().into(), &mut state);
    assert!(state
      .pending_sync
      .workspaces_to_center()
      .contains(&workspace.id()));

    state.pending_sync.clear();
    workspace.set_layout(WorkspaceLayout::Tiling);
    center_column(&windows[0].clone().into(), &mut state);
    assert!(state.pending_sync.workspaces_to_center().is_empty());
  }
}
