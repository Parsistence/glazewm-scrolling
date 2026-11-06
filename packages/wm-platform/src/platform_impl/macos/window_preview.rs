use crate::{Dispatcher, NativeWindow, Rect};

/// Platform-specific implementation of [`WindowPreview`].
///
/// Window previews are not yet supported on macOS.
pub(crate) struct WindowPreview;

impl WindowPreview {
  /// Implements [`WindowPreview::new`].
  pub(crate) fn new(
    _source: &NativeWindow,
    _dispatcher: &Dispatcher,
  ) -> crate::Result<Self> {
    Err(crate::Error::Platform(
      "Window previews are not supported on macOS.".to_string(),
    ))
  }

  /// Implements [`WindowPreview::show`].
  // LINT: Signature matches the Windows implementation.
  #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
  pub(crate) fn show(
    &self,
    _source_rect: &Rect,
    _visible_rect: &Rect,
    _raise: bool,
  ) -> crate::Result<()> {
    Ok(())
  }

  /// Implements [`WindowPreview::hide`].
  // LINT: Signature matches the Windows implementation.
  #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
  pub(crate) fn hide(&self) -> crate::Result<()> {
    Ok(())
  }
}
