use crate::{platform_impl, Dispatcher, NativeWindow, Rect};

/// A live, cropped preview of another window, drawn in an overlay window
/// that never takes focus and isn't shown in the taskbar or task switcher.
///
/// Used to show part of a window whose real window has to stay hidden,
/// such as a column of a scrolling workspace that would otherwise extend
/// onto a neighboring monitor.
///
/// The overlay is destroyed when the preview is dropped.
///
/// # Platform-specific
///
/// - **Windows**: Uses a DWM thumbnail. The preview stays live while the
///   source window is cloaked, but freezes if the source window is
///   entirely off-screen.
/// - **macOS**: Not supported; [`WindowPreview::new`] returns an error.
pub struct WindowPreview {
  /// Inner platform-specific window preview.
  inner: platform_impl::WindowPreview,
}

impl WindowPreview {
  /// Creates a hidden preview of the given source window. Use
  /// [`WindowPreview::show`] to display it.
  pub fn new(
    source: &NativeWindow,
    dispatcher: &Dispatcher,
  ) -> crate::Result<Self> {
    Ok(Self {
      inner: platform_impl::WindowPreview::new(source, dispatcher)?,
    })
  }

  /// Shows the part of the source window that lies within `visible_rect`,
  /// drawn as if the source window were positioned at `source_rect`. The
  /// overlay covers exactly `visible_rect`.
  ///
  /// Both rects are in screen coordinates. `source_rect` is the source
  /// window's visible frame (excluding invisible resize borders), and
  /// doesn't need to match where the source window actually is.
  ///
  /// If `raise` is `true`, the overlay is brought to the top of the
  /// (non-topmost) z-order; otherwise its z-order is left unchanged.
  pub fn show(
    &self,
    source_rect: &Rect,
    visible_rect: &Rect,
    raise: bool,
  ) -> crate::Result<()> {
    self.inner.show(source_rect, visible_rect, raise)
  }

  /// Hides the preview without destroying it.
  pub fn hide(&self) -> crate::Result<()> {
    self.inner.hide()
  }
}
