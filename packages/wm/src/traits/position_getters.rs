use ambassador::delegatable_trait;
use anyhow::Context;
use uuid::Uuid;
use wm_platform::Rect;

use crate::{models::Workspace, traits::CommonGetters};

#[delegatable_trait]
pub trait PositionGetters {
  fn to_rect(&self) -> anyhow::Result<Rect>;
}

/// Gets the rect of a column within a workspace that uses the scrolling
/// layout.
///
/// The rect is positioned relative to the workspace's scroll offset, and
/// can therefore lie partially or entirely outside the workspace.
pub fn scrolling_column_rect(
  column_id: Uuid,
  workspace: &Workspace,
) -> anyhow::Result<Rect> {
  let viewport = workspace.to_rect()?;

  let (_, span) = workspace
    .column_spans()?
    .into_iter()
    .find(|(column, _)| column.id() == column_id)
    .context("Column not found in scrolling workspace.")?;

  Ok(Rect::from_xy(
    viewport.x() - workspace.scroll_offset() + span.start,
    viewport.y(),
    span.width,
    viewport.height(),
  ))
}

/// Implements the `PositionGetters` trait for tiling containers that can
/// be resized. This is used by `SplitContainer` and `TilingWindow`.
///
/// Expects that the struct has a wrapping `RefCell` containing a struct
/// with an `id` and a `parent` field.
#[macro_export]
macro_rules! impl_position_getters_as_resizable {
  ($struct_name:ident) => {
    impl PositionGetters for $struct_name {
      fn to_rect(&self) -> anyhow::Result<Rect> {
        // Columns of scrolling workspaces have independent widths.
        if let Some(workspace) = self.scrolling_parent() {
          return $crate::traits::scrolling_column_rect(
            self.id(),
            &workspace,
          );
        }

        let parent = self
          .parent()
          .and_then(|parent| parent.as_direction_container().ok())
          .context("Parent does not have a tiling direction.")?;

        let parent_rect = parent.to_rect()?;

        let (horizontal_gap, vertical_gap) = self.inner_gaps()?;
        let inner_gap = match parent.tiling_direction() {
          TilingDirection::Vertical => vertical_gap,
          TilingDirection::Horizontal => horizontal_gap,
        };

        #[allow(
          clippy::cast_precision_loss,
          clippy::cast_possible_truncation,
          clippy::cast_possible_wrap
        )]
        let (width, height) = match parent.tiling_direction() {
          TilingDirection::Vertical => {
            let available_height = parent_rect.height()
              - inner_gap * self.tiling_siblings().count() as i32;

            let height =
              (self.tiling_size() * available_height as f32) as i32;

            (parent_rect.width(), height)
          }
          TilingDirection::Horizontal => {
            let available_width = parent_rect.width()
              - inner_gap * self.tiling_siblings().count() as i32;

            let width =
              (available_width as f32 * self.tiling_size()).round() as i32;

            (width, parent_rect.height())
          }
        };

        let (x, y) = {
          let mut prev_siblings = self
            .prev_siblings()
            .filter_map(|sibling| sibling.as_tiling_container().ok());

          match prev_siblings.next() {
            None => (parent_rect.x(), parent_rect.y()),
            Some(sibling) => {
              let sibling_rect = sibling.to_rect()?;

              match parent.tiling_direction() {
                TilingDirection::Vertical => (
                  parent_rect.x(),
                  sibling_rect.y() + sibling_rect.height() + inner_gap,
                ),
                TilingDirection::Horizontal => (
                  sibling_rect.x() + sibling_rect.width() + inner_gap,
                  parent_rect.y(),
                ),
              }
            }
          }
        };

        Ok(Rect::from_xy(x, y, width, height))
      }
    }
  };
}
