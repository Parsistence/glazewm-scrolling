//! Pure geometry for the scrolling layout.
//!
//! A scrolling workspace arranges its direct tiling children ("columns")
//! left-to-right in a strip that can be wider than the workspace. The
//! workspace acts as a viewport onto the strip, offset by a scroll
//! position. Column widths are fractions of the viewport (plus one inner
//! gap), so columns whose widths sum to `1.0` exactly fill the viewport.

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
  fn presets_cycle_and_wrap() {
    let presets = [0.5, 0.33, 1.0, 0.67];

    assert_eq!(next_preset_size(0.33, &presets), Some(0.5));
    assert_eq!(next_preset_size(0.4, &presets), Some(0.5));
    assert_eq!(next_preset_size(0.502, &presets), Some(0.67));
    assert_eq!(next_preset_size(1.0, &presets), Some(0.33));
    assert_eq!(next_preset_size(0.5, &[]), None);
  }
}
