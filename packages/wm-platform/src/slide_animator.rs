use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::{platform_impl, Easing, NativeWindow, Rect, WindowId};

/// Identifies an animation, e.g. by the ID of the workspace whose windows
/// are animated. Each key has its own overlay.
pub type SlideKey = u128;

/// A window to slide from one position to another.
///
/// Rects are of the window's visible frame, excluding invisible resize
/// borders, since that's the area a preview shows.
#[derive(Clone, Debug)]
pub struct SlideTarget {
  pub window: NativeWindow,

  /// Visible frame the window's preview starts at. Ignored if the window
  /// is already being animated with the same key, in which case it starts
  /// from its current animated position.
  pub from: Rect,

  /// Visible frame the window's preview ends at.
  pub to: Rect,
}

/// Request to start (or retarget) an animation.
#[derive(Clone, Debug)]
pub struct SlideRequest {
  pub key: SlideKey,

  /// Area that previews are drawn in (e.g. the monitor's rect). Parts of
  /// previews outside of it are cut off.
  pub clip_rect: Rect,

  pub targets: Vec<SlideTarget>,

  /// Windows that the overlay is placed directly above in the z-order.
  /// The overlay is placed above the highest of these.
  pub above: Vec<NativeWindow>,

  pub duration: Duration,
  pub easing: Easing,
}

/// Notification that an animation has reached its end.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlideEnded {
  pub key: SlideKey,

  /// Generation of the animation that ended, as returned by
  /// [`SlideAnimator::start`].
  pub generation: u64,
}

/// Animates windows by sliding live previews of them across the screen,
/// while the real windows stay hidden.
///
/// Moving real windows every frame is too slow on Windows (applications
/// lag far behind their target positions), whereas previews are drawn by
/// the compositor and stay in sync with the display's refresh rate.
///
/// Usage:
/// 1. [`SlideAnimator::start`] shows the previews at their start
///    positions. The caller then hides (cloaks) the real windows and moves
///    them to their end positions.
/// 2. Once a [`SlideEnded`] notification is received, the caller shows the
///    real windows again and calls [`SlideAnimator::finish`] to hide the
///    previews.
///
/// # Platform-specific
///
/// - **Windows**: Uses DWM thumbnails in an overlay window, animated on a
///   dedicated thread. Previews of cloaked windows stay live.
/// - **macOS**: Not supported; [`SlideAnimator::new`] returns an error.
pub struct SlideAnimator {
  inner: platform_impl::SlideAnimator,
}

impl SlideAnimator {
  /// Creates an animator and the receiver of its [`SlideEnded`]
  /// notifications.
  pub fn new() -> crate::Result<(Self, mpsc::UnboundedReceiver<SlideEnded>)>
  {
    let (ended_tx, ended_rx) = mpsc::unbounded_channel();

    Ok((
      Self {
        inner: platform_impl::SlideAnimator::new(ended_tx)?,
      },
      ended_rx,
    ))
  }

  /// Starts an animation, or retargets the running animation with the
  /// same key. Blocks until the previews are shown at their start
  /// positions.
  ///
  /// Requests that don't change the targets of a running animation are
  /// ignored and return its current generation.
  ///
  /// Returns the generation of the animation, which identifies its
  /// [`SlideEnded`] notification.
  pub fn start(&self, request: SlideRequest) -> crate::Result<u64> {
    self.inner.start(request)
  }

  /// Hides the previews of an animation that has ended.
  ///
  /// Does nothing if the animation was retargeted since (i.e. if
  /// `generation` is outdated).
  pub fn finish(
    &self,
    key: SlideKey,
    generation: u64,
  ) -> crate::Result<()> {
    self.inner.finish(key, generation)
  }

  /// Hides the previews of an animation immediately, whether or not it has
  /// ended.
  pub fn cancel(&self, key: SlideKey) -> crate::Result<()> {
    self.inner.cancel(key)
  }
}

/// A window within a [`Slide`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SlideWindow {
  pub id: WindowId,
  pub from: Rect,
  pub to: Rect,
}

/// Timing and positions of a running animation (platform-independent).
#[derive(Clone, Debug)]
pub(crate) struct Slide {
  pub windows: Vec<SlideWindow>,
  start: Instant,
  duration: Duration,
  easing: Easing,
}

// LINT: Only used by the Windows implementation.
#[cfg_attr(target_os = "macos", allow(dead_code))]
impl Slide {
  /// Creates an animation starting at `now`.
  pub fn new(
    windows: Vec<SlideWindow>,
    now: Instant,
    duration: Duration,
    easing: Easing,
  ) -> Self {
    Self {
      windows,
      start: now,
      duration,
      easing,
    }
  }

  /// Eased progress (`0.0..=1.0`) at the given time.
  pub fn progress(&self, at: Instant) -> f64 {
    if self.duration.is_zero() {
      return 1.0;
    }

    let elapsed = at.saturating_duration_since(self.start);
    self
      .easing
      .apply(elapsed.as_secs_f64() / self.duration.as_secs_f64())
  }

  /// Whether the animation has reached its end at the given time.
  pub fn is_done(&self, at: Instant) -> bool {
    at.saturating_duration_since(self.start) >= self.duration
  }

  /// Positions of all windows at the given time.
  pub fn rects_at(&self, at: Instant) -> Vec<(WindowId, Rect)> {
    let progress = self.progress(at);

    self
      .windows
      .iter()
      .map(|window| {
        (
          window.id,
          interpolate_rect(&window.from, &window.to, progress),
        )
      })
      .collect()
  }

  /// Whether the given windows and end positions match this animation's.
  pub fn has_targets(&self, targets: &[(WindowId, Rect)]) -> bool {
    self.windows.len() == targets.len()
      && self
        .windows
        .iter()
        .zip(targets)
        .all(|(window, (id, to))| window.id == *id && window.to == *to)
  }

  /// Restarts the animation at `now` towards new end positions. Windows
  /// that were already animated start from their current position.
  pub fn retarget(
    &mut self,
    windows: Vec<SlideWindow>,
    now: Instant,
    duration: Duration,
    easing: Easing,
  ) {
    let current = self.rects_at(now);

    let windows = windows
      .into_iter()
      .map(|window| {
        let from = current
          .iter()
          .find(|(id, _)| *id == window.id)
          .map_or(window.from, |(_, rect)| rect.clone());

        SlideWindow { from, ..window }
      })
      .collect();

    *self = Self::new(windows, now, duration, easing);
  }
}

/// Linearly interpolates between two rects. `progress` of `0.0` gives
/// `from`, and `1.0` gives `to`.
#[allow(clippy::cast_possible_truncation)]
fn interpolate_rect(from: &Rect, to: &Rect, progress: f64) -> Rect {
  let lerp = |a: i32, b: i32| {
    (f64::from(a) + f64::from(b - a) * progress).round() as i32
  };

  Rect::from_ltrb(
    lerp(from.left, to.left),
    lerp(from.top, to.top),
    lerp(from.right, to.right),
    lerp(from.bottom, to.bottom),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn window(id: isize, from_x: i32, to_x: i32) -> SlideWindow {
    SlideWindow {
      id: WindowId(id),
      from: Rect::from_xy(from_x, 0, 100, 100),
      to: Rect::from_xy(to_x, 0, 100, 100),
    }
  }

  #[test]
  fn slide_interpolates_between_positions() {
    let now = Instant::now();
    let slide = Slide::new(
      vec![window(1, 0, 200)],
      now,
      Duration::from_millis(100),
      Easing::Linear,
    );

    let halfway = slide.rects_at(now + Duration::from_millis(50));
    assert_eq!(halfway[0].1, Rect::from_xy(100, 0, 100, 100));

    let end = slide.rects_at(now + Duration::from_millis(500));
    assert_eq!(end[0].1, Rect::from_xy(200, 0, 100, 100));
    assert!(slide.is_done(now + Duration::from_millis(100)));
    assert!(!slide.is_done(now + Duration::from_millis(99)));
  }

  #[test]
  fn retarget_continues_from_current_position() {
    let now = Instant::now();
    let duration = Duration::from_millis(100);
    let mut slide =
      Slide::new(vec![window(1, 0, 200)], now, duration, Easing::Linear);

    let later = now + Duration::from_millis(50);
    slide.retarget(
      vec![window(1, 0, 400), window(2, 900, 300)],
      later,
      duration,
      Easing::Linear,
    );

    let start = slide.rects_at(later);
    assert_eq!(start[0].1.left, 100);
    assert_eq!(start[1].1.left, 900);
    assert!(!slide.is_done(later));
  }

  #[test]
  fn has_targets_compares_windows_and_end_positions() {
    let slide = Slide::new(
      vec![window(1, 0, 200)],
      Instant::now(),
      Duration::from_millis(100),
      Easing::Linear,
    );

    let same = [(WindowId(1), Rect::from_xy(200, 0, 100, 100))];
    let moved = [(WindowId(1), Rect::from_xy(300, 0, 100, 100))];
    assert!(slide.has_targets(&same));
    assert!(!slide.has_targets(&moved));
    assert!(!slide.has_targets(&[]));
  }

  #[test]
  fn zero_duration_ends_immediately() {
    let now = Instant::now();
    let slide = Slide::new(
      vec![window(1, 0, 200)],
      now,
      Duration::ZERO,
      Easing::Linear,
    );

    assert!(slide.is_done(now));
    assert_eq!(slide.rects_at(now)[0].1.left, 200);
  }
}
