use crate::{
  commands::container::resize_tiling_container,
  models::Container,
  scrolling,
  traits::{CommonGetters, TilingSizeGetters},
  wm_state::WmState,
};

/// Sets the width of the subject's column to the next configured width
/// preset, wrapping around to the smallest preset.
///
/// No-op if the subject's workspace doesn't use the scrolling layout, or
/// if no presets are configured.
pub fn cycle_column_width(
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

  let Some(next_size) = scrolling::next_preset_size(
    column.tiling_size(),
    &workspace.column_size_presets(),
  ) else {
    return;
  };

  resize_tiling_container(&column, next_size);

  state
    .pending_sync
    .queue_containers_to_redraw(workspace.tiling_children());
}
