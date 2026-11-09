//! Pure geometry for the scrolling layout.
//!
//! A scrolling workspace arranges its direct tiling children ("columns")
//! left-to-right in a strip that can be wider than the workspace. The
//! workspace acts as a viewport onto the strip, offset by a scroll
//! position. Column widths are fractions of the viewport (plus one inner
//! gap), so columns whose widths sum to `1.0` exactly fill the viewport.

use wm_common::PeekMode;
use wm_platform::Rect;

/// Minimum width of a column as a fraction of the viewport.
pub const MIN_COLUMN_SIZE: f32 = 0.05;

/// Maximum width of a column as a fraction of the viewport.
pub const MAX_COLUMN_SIZE: f32 = 1.0;

/// Tolerance used when comparing column sizes to presets.
const SIZE_EPSILON: f32 = 0.005;

/// Horizontal extent of a column within the strip.
///
/// `start` is relative to the left edge of the strip (i.e. unaffected by
/// the scroll offset).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColumnSpan {
  pub start: i32,
  pub width: i32,
}

impl ColumnSpan {
  /// Right edge of the column within the strip (exclusive).
  #[must_use]
  pub fn end(self) -> i32 {
    self.start + self.width
  }
}

/// How much of a column falls within the viewport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColumnVisibility {
  /// The column lies entirely within the viewport.
  Full,

  /// Part of the column lies within the viewport.
  Partial,

  /// No part of the column lies within the viewport.
  Hidden,
}

/// Converts a column size (fraction of the viewport) to a width in pixels.
///
/// A column of size `f` spans `f` of the viewport plus one inner gap,
/// minus its own trailing gap. Always returns at least 1px.
#[must_use]
pub fn column_width_px(size: f32, viewport_width: i32, gap: i32) -> i32 {
  #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
  let width = ((viewport_width + gap) as f32 * size).round() as i32 - gap;

  width.max(1)
}

/// Converts a width in pixels to a column size (fraction of the
/// viewport). Inverse of [`column_width_px`].
#[must_use]
pub fn column_size_from_px(
  width: i32,
  viewport_width: i32,
  gap: i32,
) -> f32 {
  let total = (viewport_width + gap).max(1);

  #[allow(clippy::cast_precision_loss)]
  let size = (width + gap) as f32 / total as f32;

  clamp_column_size(size)
}

/// Clamps a column size to the allowed range.
#[must_use]
pub fn clamp_column_size(size: f32) -> f32 {
  size.clamp(MIN_COLUMN_SIZE, MAX_COLUMN_SIZE)
}

/// Whether a column of the given size spans the full viewport width.
#[must_use]
pub fn is_full_width(size: f32) -> bool {
  size >= MAX_COLUMN_SIZE - SIZE_EPSILON
}

/// Lays out columns of the given sizes left-to-right, separated by `gap`.
#[must_use]
pub fn column_spans(
  sizes: &[f32],
  viewport_width: i32,
  gap: i32,
) -> Vec<ColumnSpan> {
  let mut start = 0;

  sizes
    .iter()
    .map(|size| {
      let span = ColumnSpan {
        start,
        width: column_width_px(*size, viewport_width, gap),
      };

      start = span.end() + gap;
      span
    })
    .collect()
}

/// Total width of the strip formed by the given spans.
#[must_use]
pub fn strip_width(spans: &[ColumnSpan]) -> i32 {
  spans.last().map_or(0, |span| span.end())
}

/// Clamps a scroll offset so that the viewport stays within the strip.
///
/// Strips narrower than the viewport are left-aligned (offset of 0).
#[must_use]
pub fn clamp_offset(
  offset: i32,
  strip_width: i32,
  viewport_width: i32,
) -> i32 {
  offset.clamp(0, (strip_width - viewport_width).max(0))
}

/// Gets the minimal scroll offset that brings the given span fully into
/// view, starting from `offset`.
///
/// Columns wider than the viewport are aligned to the viewport's left
/// edge.
#[must_use]
pub fn offset_to_reveal(
  offset: i32,
  span: ColumnSpan,
  viewport_width: i32,
) -> i32 {
  if span.start < offset || span.width >= viewport_width {
    span.start
  } else if span.end() > offset + viewport_width {
    span.end() - viewport_width
  } else {
    offset
  }
}

/// Gets the scroll offset that centers the given span in the viewport.
///
/// Columns wider than the viewport are aligned to the viewport's left
/// edge. The result may lie outside the strip, so it should be clamped
/// with [`clamp_offset`].
#[must_use]
pub fn offset_to_center(span: ColumnSpan, viewport_width: i32) -> i32 {
  if span.width >= viewport_width {
    span.start
  } else {
    span.start - (viewport_width - span.width) / 2
  }
}

/// How the viewport is scrolled to reveal the focused column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevealMode {
  /// Scroll just enough to show the column in full.
  Minimal,

  /// Scroll to center the column.
  Centered,
}

/// Gets the scroll offset that reveals the given span, starting from
/// `offset`. See [`offset_to_reveal`] and [`offset_to_center`].
#[must_use]
pub fn offset_for_reveal(
  offset: i32,
  span: ColumnSpan,
  viewport_width: i32,
  mode: RevealMode,
) -> i32 {
  match mode {
    RevealMode::Minimal => offset_to_reveal(offset, span, viewport_width),
    RevealMode::Centered => offset_to_center(span, viewport_width),
  }
}

/// Gets how much of a span is visible in the viewport at `offset`.
#[must_use]
pub fn column_visibility(
  span: ColumnSpan,
  offset: i32,
  viewport_width: i32,
) -> ColumnVisibility {
  let viewport_end = offset + viewport_width;

  if span.start >= offset && span.end() <= viewport_end {
    ColumnVisibility::Full
  } else if span.end() <= offset || span.start >= viewport_end {
    ColumnVisibility::Hidden
  } else {
    ColumnVisibility::Partial
  }
}

/// Gets the x-coordinate at which to park a column that is outside the
/// viewport, so that it stays within the viewport (and thus on its
/// monitor) while hidden.
///
/// Columns left of the viewport are aligned to its left edge, and columns
/// right of it to its right edge. Columns wider than the viewport are
/// aligned to the left edge.
///
/// Parking hidden columns on-screen keeps their DWM thumbnails live, and
/// keeps them reachable if the WM exits without restoring them.
#[must_use]
pub fn parked_x(
  column_x: i32,
  column_width: i32,
  viewport_x: i32,
  viewport_width: i32,
) -> i32 {
  if column_x < viewport_x || column_width >= viewport_width {
    viewport_x
  } else {
    viewport_x + viewport_width - column_width
  }
}

/// How a window in a column that's partially outside the viewport (a
/// "peeking" column) is shown.
#[derive(Clone, Debug, PartialEq)]
pub enum PeekDecision {
  /// The real window is shown at its position in the strip. Any part
  /// beyond the monitor's edge is simply off-screen.
  Shown,

  /// The real window is hidden, and a live preview covers the given
  /// visible area instead. Used where the window would otherwise extend
  /// onto a neighboring monitor.
  Preview(Rect),

  /// The window is hidden.
  Hidden,
}

/// Decides how to show a window of a peeking column.
///
/// `window_rect` is where the window would be placed in the strip, and
/// `monitor_rect` the bounds of the window's monitor.
#[must_use]
pub fn peek_decision(
  window_rect: &Rect,
  monitor_rect: &Rect,
  other_monitor_rects: &[Rect],
  mode: PeekMode,
) -> PeekDecision {
  if mode == PeekMode::Hide {
    return PeekDecision::Hidden;
  }

  let Some(visible_rect) = intersection(window_rect, monitor_rect) else {
    return PeekDecision::Hidden;
  };

  let crosses_left = window_rect.left < monitor_rect.left;
  let crosses_right = window_rect.right > monitor_rect.right;

  let needs_preview = match mode {
    PeekMode::Preview => crosses_left || crosses_right,
    PeekMode::Auto => {
      (crosses_left
        && has_adjacent_monitor(
          monitor_rect,
          other_monitor_rects,
          HorizontalEdge::Left,
        ))
        || (crosses_right
          && has_adjacent_monitor(
            monitor_rect,
            other_monitor_rects,
            HorizontalEdge::Right,
          ))
    }
    PeekMode::Hide => unreachable!("Handled above."),
  };

  if needs_preview {
    PeekDecision::Preview(visible_rect)
  } else {
    PeekDecision::Shown
  }
}

/// Left or right edge of a rect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HorizontalEdge {
  Left,
  Right,
}

/// Whether another monitor shares the given edge of `monitor_rect`, i.e.
/// is directly beside it and overlaps it vertically.
#[must_use]
pub fn has_adjacent_monitor(
  monitor_rect: &Rect,
  other_monitor_rects: &[Rect],
  edge: HorizontalEdge,
) -> bool {
  other_monitor_rects.iter().any(|other| {
    let is_beside = match edge {
      HorizontalEdge::Left => other.right == monitor_rect.left,
      HorizontalEdge::Right => other.left == monitor_rect.right,
    };

    is_beside
      && other.top < monitor_rect.bottom
      && other.bottom > monitor_rect.top
  })
}

/// Gets the overlapping area of two rects, or `None` if they don't
/// overlap.
#[must_use]
pub fn intersection(a: &Rect, b: &Rect) -> Option<Rect> {
  let left = a.left.max(b.left);
  let top = a.top.max(b.top);
  let right = a.right.min(b.right);
  let bottom = a.bottom.min(b.bottom);

  (left < right && top < bottom)
    .then(|| Rect::from_ltrb(left, top, right, bottom))
}

/// Gets the next size in `presets` after `current`, wrapping around to
/// the smallest preset.
///
/// Presets are compared in ascending order regardless of the order they
/// are given in. Returns `None` if there are no presets.
#[must_use]
pub fn next_preset_size(current: f32, presets: &[f32]) -> Option<f32> {
  let mut sorted = presets.to_vec();
  sorted.sort_by(f32::total_cmp);

  sorted
    .iter()
    .find(|preset| **preset > current + SIZE_EPSILON)
    .or_else(|| sorted.first())
    .copied()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn half_width_columns_fill_viewport_with_gap() {
    let spans = column_spans(&[0.5, 0.5], 1000, 10);

    assert_eq!(
      spans[0],
      ColumnSpan {
        start: 0,
        width: 495
      }
    );
    assert_eq!(
      spans[1],
      ColumnSpan {
        start: 505,
        width: 495
      }
    );
    assert_eq!(strip_width(&spans), 1000);
  }

  #[test]
  fn width_conversion_round_trips() {
    let width = column_width_px(0.33, 1680, 20);
    let size = column_size_from_px(width, 1680, 20);

    assert!((size - 0.33).abs() < 0.001);
  }

  #[test]
  fn size_from_px_is_clamped() {
    assert!(
      (column_size_from_px(1, 1000, 0) - MIN_COLUMN_SIZE).abs() < 1e-6
    );
    assert!(
      (column_size_from_px(5000, 1000, 0) - MAX_COLUMN_SIZE).abs() < 1e-6
    );
  }

  #[test]
  fn width_is_never_zero() {
    assert_eq!(column_width_px(0.0, 1000, 10), 1);
  }

  #[test]
  fn offset_clamps_to_strip() {
    assert_eq!(clamp_offset(-50, 3000, 1000), 0);
    assert_eq!(clamp_offset(5000, 3000, 1000), 2000);
    assert_eq!(clamp_offset(700, 3000, 1000), 700);
    // Strip narrower than viewport.
    assert_eq!(clamp_offset(300, 600, 1000), 0);
  }

  #[test]
  fn reveal_scrolls_minimally() {
    let span = ColumnSpan {
      start: 1200,
      width: 500,
    };

    // Already visible: no change.
    assert_eq!(offset_to_reveal(1000, span, 1000), 1000);
    // Off to the right: align right edge.
    assert_eq!(offset_to_reveal(0, span, 1000), 700);
    // Off to the left: align left edge.
    assert_eq!(offset_to_reveal(1500, span, 1000), 1200);
  }

  #[test]
  fn reveal_aligns_wide_columns_left() {
    let span = ColumnSpan {
      start: 400,
      width: 1200,
    };
    assert_eq!(offset_to_reveal(0, span, 1000), 400);
  }

  #[test]
  fn center_ignores_current_offset_and_aligns_wide_columns_left() {
    let span = ColumnSpan {
      start: 1200,
      width: 500,
    };

    assert_eq!(
      offset_for_reveal(0, span, 1000, RevealMode::Centered),
      950
    );
    assert_eq!(
      offset_for_reveal(5000, span, 1000, RevealMode::Centered),
      950
    );

    let wide_span = ColumnSpan {
      start: 400,
      width: 1200,
    };
    assert_eq!(offset_to_center(wide_span, 1000), 400);
  }

  #[test]
  fn center_of_first_column_is_clamped_to_strip_start() {
    let span = ColumnSpan {
      start: 0,
      width: 500,
    };

    assert_eq!(clamp_offset(offset_to_center(span, 1000), 3000, 1000), 0);
  }

  #[test]
  fn visibility_classification() {
    let span = ColumnSpan {
      start: 500,
      width: 500,
    };

    assert_eq!(column_visibility(span, 0, 1000), ColumnVisibility::Full);
    assert_eq!(column_visibility(span, 200, 1000), ColumnVisibility::Full);
    assert_eq!(
      column_visibility(span, 600, 1000),
      ColumnVisibility::Partial
    );
    assert_eq!(
      column_visibility(span, -200, 1000),
      ColumnVisibility::Partial
    );
    assert_eq!(
      column_visibility(span, 900, 1000),
      ColumnVisibility::Partial
    );
    assert_eq!(
      column_visibility(span, 1000, 1000),
      ColumnVisibility::Hidden
    );
    assert_eq!(
      column_visibility(span, -600, 1000),
      ColumnVisibility::Hidden
    );
  }

  #[test]
  fn parked_columns_stay_within_viewport() {
    // Viewport spans 100..1100.
    assert_eq!(parked_x(-900, 500, 100, 1000), 100);
    assert_eq!(parked_x(1500, 500, 100, 1000), 600);
    assert_eq!(parked_x(1500, 1200, 100, 1000), 100);
  }

  /// Two side-by-side 1000x800 monitors.
  fn monitors() -> (Rect, Rect) {
    (
      Rect::from_xy(0, 0, 1000, 800),
      Rect::from_xy(1000, 0, 1000, 800),
    )
  }

  #[test]
  fn adjacent_monitors_are_detected_per_edge() {
    let (left, right) = monitors();

    assert!(has_adjacent_monitor(
      &left,
      std::slice::from_ref(&right),
      HorizontalEdge::Right
    ));
    assert!(!has_adjacent_monitor(
      &left,
      std::slice::from_ref(&right),
      HorizontalEdge::Left
    ));

    // Vertically offset without overlap isn't adjacent.
    let below = Rect::from_xy(1000, 800, 1000, 800);
    assert!(!has_adjacent_monitor(
      &left,
      &[below],
      HorizontalEdge::Right
    ));
  }

  #[test]
  fn auto_mode_previews_only_at_shared_edges() {
    let (left, right) = monitors();
    let others = [right];

    // Crosses the shared right edge: preview the visible part.
    let crossing_right = Rect::from_xy(700, 0, 500, 800);
    assert_eq!(
      peek_decision(&crossing_right, &left, &others, PeekMode::Auto),
      PeekDecision::Preview(Rect::from_ltrb(700, 0, 1000, 800))
    );

    // Crosses the outer left edge: the real window can overflow.
    let crossing_left = Rect::from_xy(-200, 0, 500, 800);
    assert_eq!(
      peek_decision(&crossing_left, &left, &others, PeekMode::Auto),
      PeekDecision::Shown
    );
  }

  #[test]
  fn preview_mode_previews_at_every_edge() {
    let (left, _) = monitors();
    let crossing_left = Rect::from_xy(-200, 0, 500, 800);

    assert_eq!(
      peek_decision(&crossing_left, &left, &[], PeekMode::Preview),
      PeekDecision::Preview(Rect::from_ltrb(0, 0, 300, 800))
    );
  }

  #[test]
  fn windows_within_the_monitor_are_shown_and_hide_mode_hides() {
    let (left, right) = monitors();
    let others = [right];
    let in_outer_gap = Rect::from_xy(10, 0, 500, 800);

    assert_eq!(
      peek_decision(&in_outer_gap, &left, &others, PeekMode::Preview),
      PeekDecision::Shown
    );
    assert_eq!(
      peek_decision(&in_outer_gap, &left, &others, PeekMode::Hide),
      PeekDecision::Hidden
    );

    let off_monitor = Rect::from_xy(1200, 0, 500, 800);
    assert_eq!(
      peek_decision(&off_monitor, &left, &others, PeekMode::Auto),
      PeekDecision::Hidden
    );
  }

  #[test]
  fn presets_cycle_and_wrap() {
    let presets = [0.5, 0.33, 1.0, 0.67];

    assert_eq!(next_preset_size(0.33, &presets), Some(0.5));
    assert_eq!(next_preset_size(0.4, &presets), Some(0.5));
    assert_eq!(next_preset_size(0.502, &presets), Some(0.67));
    assert_eq!(next_preset_size(1.0, &presets), Some(0.33));
    assert_eq!(next_preset_size(0.5, &[]), None);
  }
}
