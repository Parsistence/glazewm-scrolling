//! Live previews shown in place of peeking windows of scrolling
//! workspaces (see `scrolling::PeekDecision::Preview`).

use std::collections::HashMap;

use uuid::Uuid;
use wm_platform::{Dispatcher, NativeWindow, Point, Rect, WindowPreview};

/// A preview that should be shown for a window.
pub struct DesiredPreview {
  pub window_id: Uuid,
  pub native_window: NativeWindow,

  /// Visible frame of the window (excluding invisible resize borders)
  /// where it would be placed in the strip.
  pub source_rect: Rect,

  /// Area of the window that should be visible.
  pub visible_rect: Rect,
}

/// A preview that is currently shown.
struct ActivePreview {
  preview: WindowPreview,
  source_rect: Rect,
  visible_rect: Rect,
}

/// Previews currently shown in place of peeking windows, keyed by window
/// ID.
#[derive(Default)]
pub struct WindowPreviews {
  active: HashMap<Uuid, ActivePreview>,
}

impl WindowPreviews {
  /// Shows, moves and removes previews so that exactly the given previews
  /// are shown.
  ///
  /// Failures to create or update a preview are logged; the affected
  /// window's peeking part is then simply not shown.
  pub fn sync(
    &mut self,
    desired: Vec<DesiredPreview>,
    dispatcher: &Dispatcher,
  ) {
    // Drop previews that are no longer needed, which destroys them.
    self.active.retain(|window_id, _| {
      desired
        .iter()
        .any(|preview| preview.window_id == *window_id)
    });

    for desired in desired {
      if let Some(active) = self.active.get_mut(&desired.window_id) {
        if active.source_rect == desired.source_rect
          && active.visible_rect == desired.visible_rect
        {
          continue;
        }

        if let Err(err) = active.preview.show(
          &desired.source_rect,
          &desired.visible_rect,
          false,
        ) {
          tracing::warn!("Failed to update window preview: {}", err);
        }

        active.source_rect = desired.source_rect;
        active.visible_rect = desired.visible_rect;
        continue;
      }

      let preview =
        match WindowPreview::new(&desired.native_window, dispatcher) {
          Ok(preview) => preview,
          Err(err) => {
            tracing::warn!("Failed to create window preview: {}", err);
            continue;
          }
        };

      // Raise new previews so that they're above windows that were
      // previously shown in their area.
      if let Err(err) =
        preview.show(&desired.source_rect, &desired.visible_rect, true)
      {
        tracing::warn!("Failed to show window preview: {}", err);
        continue;
      }

      self.active.insert(
        desired.window_id,
        ActivePreview {
          preview,
          source_rect: desired.source_rect,
          visible_rect: desired.visible_rect,
        },
      );
    }
  }

  /// Gets the ID of the window whose preview contains the given point.
  pub fn window_at_point(&self, point: &Point) -> Option<Uuid> {
    self
      .active
      .iter()
      .find(|(_, active)| active.visible_rect.contains_point(point))
      .map(|(window_id, _)| *window_id)
  }
}
