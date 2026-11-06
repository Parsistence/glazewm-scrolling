use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Layout used to arrange the tiling windows of a workspace.
#[derive(
  Clone,
  Copy,
  Debug,
  Default,
  Deserialize,
  Eq,
  PartialEq,
  Serialize,
  ValueEnum,
)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceLayout {
  /// Columns with independent widths in a horizontally scrollable strip.
  #[default]
  Scrolling,

  /// Containers share the workspace's space proportionally.
  Tiling,
}

impl WorkspaceLayout {
  /// Gets the other layout.
  ///
  /// # Example usage
  ///
  /// ```
  /// # use wm_common::WorkspaceLayout;
  /// let layout = WorkspaceLayout::Scrolling.toggled();
  /// assert_eq!(layout, WorkspaceLayout::Tiling);
  /// ```
  #[must_use]
  pub fn toggled(self) -> Self {
    match self {
      Self::Scrolling => Self::Tiling,
      Self::Tiling => Self::Scrolling,
    }
  }
}
