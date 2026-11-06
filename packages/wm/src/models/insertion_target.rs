use crate::models::Container;

#[derive(Debug, Clone)]
pub struct InsertionTarget {
  pub target_parent: Container,
  pub target_index: usize,
  pub prev_tiling_size: f32,
  pub prev_sibling_count: usize,

  /// Whether the window was a column of a scrolling workspace, in which
  /// case `prev_tiling_size` is a column size rather than a share of its
  /// parent.
  pub was_scrolling_column: bool,
}
