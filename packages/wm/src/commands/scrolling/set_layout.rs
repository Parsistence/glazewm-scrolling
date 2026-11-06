use tracing::info;
use wm_common::{TilingDirection, WorkspaceLayout};

use crate::{
  commands::container::flatten_child_split_containers,
  models::Workspace,
  traits::{CommonGetters, TilingDirectionGetters, TilingSizeGetters},
  wm_state::WmState,
};

/// Changes the layout of a workspace and queues it for redraw.
///
/// See [`apply_layout`] for how container sizes are converted.
pub fn set_layout(
  workspace: &Workspace,
  layout: WorkspaceLayout,
  state: &mut WmState,
) -> anyhow::Result<()> {
  if workspace.layout() == layout {
    return Ok(());
  }

  info!("Setting layout of {workspace} to {layout:?}.");
  apply_layout(workspace, layout)?;

  // A `WorkspaceUpdated` event is emitted on the next platform sync, since
  // the layout is part of the broadcast scroll state.
  state
    .pending_sync
    .queue_containers_to_redraw(workspace.tiling_children());

  Ok(())
}

/// Changes the layout of a workspace, converting the sizes of its
/// top-level tiling containers to suit the new layout.
///
/// - Tiling -> scrolling: each top-level container becomes a column with
///   the default column width, and the workspace tiles horizontally.
/// - Scrolling -> tiling: column widths are rescaled to share the
///   workspace proportionally.
pub fn apply_layout(
  workspace: &Workspace,
  layout: WorkspaceLayout,
) -> anyhow::Result<()> {
  let tiling_children = workspace.tiling_children().collect::<Vec<_>>();
  workspace.set_layout(layout);
  workspace.set_scroll_offset(0);

  match layout {
    WorkspaceLayout::Scrolling => {
      workspace.set_tiling_direction(TilingDirection::Horizontal);

      let column_size = workspace.default_column_size();
      for child in &tiling_children {
        child.set_tiling_size(column_size);
      }
    }
    WorkspaceLayout::Tiling => {
      let total_size = tiling_children
        .iter()
        .map(TilingSizeGetters::tiling_size)
        .sum::<f32>();

      if total_size > 0. {
        for child in &tiling_children {
          child.set_tiling_size(child.tiling_size() / total_size);
        }
      }
    }
  }

  // Changing the tiling direction can leave redundant split containers
  // (e.g. horizontal splits within a now horizontal workspace).
  flatten_child_split_containers(&workspace.clone().into())
}

#[cfg(test)]
mod tests {
  use wm_common::{TilingDirection, WorkspaceLayout};

  use super::apply_layout;
  use crate::{
    models::{SplitContainer, TilingWindow, Workspace},
    traits::{CommonGetters, TilingDirectionGetters, TilingSizeGetters},
  };

  #[test]
  fn round_trip_conversion_restores_equal_shares() {
    let workspace = Workspace::mock()
      .tiling_direction(TilingDirection::Vertical)
      .tiling_containers(vec![
        TilingWindow::mock().call().into(),
        TilingWindow::mock().call().into(),
        TilingWindow::mock().call().into(),
      ])
      .call();

    apply_layout(&workspace, WorkspaceLayout::Scrolling).unwrap();
    assert_eq!(workspace.tiling_direction(), TilingDirection::Horizontal);
    for child in workspace.tiling_children() {
      assert!((child.tiling_size() - 0.5).abs() < 1e-6);
    }

    apply_layout(&workspace, WorkspaceLayout::Tiling).unwrap();
    for child in workspace.tiling_children() {
      assert!((child.tiling_size() - 1. / 3.).abs() < 1e-6);
    }
  }

  #[test]
  fn horizontal_splits_become_columns() {
    // V[1 H[2 3]] -> H[1 2 3] when switching to scrolling.
    let workspace = Workspace::mock()
      .tiling_direction(TilingDirection::Vertical)
      .tiling_containers(vec![
        TilingWindow::mock().call().into(),
        SplitContainer::mock()
          .tiling_direction(TilingDirection::Horizontal)
          .tiling_containers(vec![
            TilingWindow::mock().call().into(),
            TilingWindow::mock().call().into(),
          ])
          .call()
          .into(),
      ])
      .call();

    apply_layout(&workspace, WorkspaceLayout::Scrolling).unwrap();

    let columns = workspace.tiling_children().collect::<Vec<_>>();
    assert_eq!(columns.len(), 3);
    assert!(columns
      .iter()
      .all(|column| column.as_tiling_window().is_some()));
    for column in &columns {
      assert!((column.tiling_size() - 0.5).abs() < 1e-6);
    }
  }
}
