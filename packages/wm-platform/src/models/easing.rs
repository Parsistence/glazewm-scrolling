use serde::{Deserialize, Serialize};

/// Timing curve of an animation.
#[derive(
  Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
  /// Constant speed.
  Linear,

  /// Starts fast and decelerates (cubic).
  #[default]
  EaseOutCubic,

  /// Accelerates, then decelerates (cubic).
  EaseInOutCubic,
}

impl Easing {
  /// Maps linear progress (`0.0..=1.0`) to eased progress (`0.0..=1.0`).
  ///
  /// Progress outside of `0.0..=1.0` is clamped.
  #[must_use]
  pub fn apply(self, progress: f64) -> f64 {
    let t = progress.clamp(0.0, 1.0);

    match self {
      Easing::Linear => t,
      Easing::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
      Easing::EaseInOutCubic => {
        if t < 0.5 {
          4.0 * t.powi(3)
        } else {
          1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn curves_start_at_zero_and_end_at_one() {
    for easing in
      [Easing::Linear, Easing::EaseOutCubic, Easing::EaseInOutCubic]
    {
      assert!(easing.apply(0.0).abs() < f64::EPSILON);
      assert!((easing.apply(1.0) - 1.0).abs() < f64::EPSILON);
      assert!((easing.apply(2.0) - 1.0).abs() < f64::EPSILON);
      assert!(easing.apply(-1.0).abs() < f64::EPSILON);
    }
  }

  #[test]
  fn ease_out_is_ahead_of_linear() {
    assert!(Easing::EaseOutCubic.apply(0.3) > Easing::Linear.apply(0.3));
    assert!((Easing::EaseInOutCubic.apply(0.5) - 0.5).abs() < 1e-9);
  }
}
