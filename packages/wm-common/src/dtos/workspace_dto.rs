use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ContainerDto;
use crate::{TilingDirection, WorkspaceLayout};

/// User-friendly representation of a workspace.
///
/// Used for IPC and debug logging.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDto {
  pub id: Uuid,
  pub name: String,
  pub display_name: Option<String>,
  pub parent_id: Option<Uuid>,
  pub children: Vec<ContainerDto>,
  pub child_focus_order: Vec<Uuid>,
  pub has_focus: bool,
  pub is_displayed: bool,
  pub width: i32,
  pub height: i32,
  pub x: i32,
  pub y: i32,
  pub tiling_direction: TilingDirection,
  pub layout: WorkspaceLayout,

  /// Scroll state of the workspace. Only present for workspaces with the
  /// scrolling layout.
  pub scrolling: Option<ScrollingStateDto>,
}

/// Scroll state of a workspace with the scrolling layout.
///
/// The workspace's `x` and `width` describe the viewport; the columns form
/// a strip that the viewport is scrolled across.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollingStateDto {
  /// Horizontal position of the viewport within the strip (in pixels).
  pub scroll_offset: i32,

  /// Total width of the strip (in pixels).
  pub strip_width: i32,

  /// Columns in left-to-right order.
  pub columns: Vec<ColumnDto>,

  /// ID of the most recently focused column.
  pub focused_column_id: Option<Uuid>,
}

/// Horizontal extent of a column within a scrolling workspace's strip.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDto {
  /// ID of the column's container (a tiling window or split container).
  pub id: Uuid,

  /// Left edge of the column, relative to the start of the strip (in
  /// pixels).
  pub start: i32,

  /// Width of the column (in pixels).
  pub width: i32,
}
