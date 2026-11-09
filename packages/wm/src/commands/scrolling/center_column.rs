use crate::{models::Container, traits::CommonGetters, wm_state::WmState};

/// Scrolls the viewport of the subject's workspace to center its focused
/// column.
///
/// The offset is applied on the next sync, so commands that change focus
/// beforehand in the same batch center the newly focused column. The
/// viewport stays put until focus moves to a column outside of it.
/// Columns near either end of the strip can't be fully centered, since
/// the viewport stays within the strip. No-op if the subject's workspace
/// doesn't use the scrolling layout.
pub fn center_column(subject_container: &Container, state: &mut WmState) {
  let Some(workspace) = subject_container.scrolling_workspace() else {
    return;
  };

  state.pending_sync.queue_workspace_to_center(&workspace);
}
