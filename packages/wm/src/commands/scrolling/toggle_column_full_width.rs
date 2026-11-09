use crate::{
  commands::container::resize_tiling_container,
  models::Container,
  scrolling,
  traits::{CommonGetters, TilingSizeGetters},
  wm_state::WmState,
};

/// Toggles the subject's column between the full width of the viewport
/// and the width it had before.
///
/// Columns that are already full-width without having been toggled (e.g.
/// via a width preset) are restored to the default column width. No-op if
/// the subject's workspace doesn't use the scrolling layout.
pub fn toggle_column_full_width(
  subject_container: &Container,
  state: &mut WmState,
) {
  let Some(workspace) = subject_container.scrolling_workspace() else {
    return;
  };

  let Some(column) = workspace
    .column_of(subject_container)
    .or_else(|| workspace.focused_column())
  else {
    return;
  };

  let current_size = column.tiling_size();

  let new_size = if scrolling::is_full_width(current_size) {
    workspace
      .take_pre_full_width_size(column.id())
      .unwrap_or_else(|| workspace.default_column_size())
  } else {
    workspace.set_pre_full_width_size(column.id(), current_size);
    scrolling::MAX_COLUMN_SIZE
  };

  resize_tiling_container(&column, new_size);

  state
    .pending_sync
    .queue_containers_to_redraw(workspace.tiling_children());
}
