use tokio::sync::mpsc;

use crate::{SlideEnded, SlideKey, SlideRequest};

/// Platform-specific implementation of [`SlideAnimator`].
///
/// Slide animations are not yet supported on macOS.
pub(crate) struct SlideAnimator;

impl SlideAnimator {
  /// Implements [`SlideAnimator::new`].
  pub(crate) fn new(
    _ended_tx: mpsc::UnboundedSender<SlideEnded>,
  ) -> crate::Result<Self> {
    Err(crate::Error::Platform(
      "Slide animations are not supported on macOS.".to_string(),
    ))
  }

  /// Implements [`SlideAnimator::start`].
  // LINT: Signature matches the Windows implementation.
  #[allow(clippy::unused_self, clippy::needless_pass_by_value)]
  pub(crate) fn start(
    &self,
    _request: SlideRequest,
  ) -> crate::Result<u64> {
    Err(crate::Error::Platform(
      "Slide animations are not supported on macOS.".to_string(),
    ))
  }

  /// Implements [`SlideAnimator::finish`].
  // LINT: Signature matches the Windows implementation.
  #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
  pub(crate) fn finish(
    &self,
    _key: SlideKey,
    _generation: u64,
  ) -> crate::Result<()> {
    Ok(())
  }

  /// Implements [`SlideAnimator::cancel`].
  // LINT: Signature matches the Windows implementation.
  #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
  pub(crate) fn cancel(&self, _key: SlideKey) -> crate::Result<()> {
    Ok(())
  }
}
