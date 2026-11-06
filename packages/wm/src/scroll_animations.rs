//! Animated scrolling of scrolling workspaces.
//!
//! While a workspace's strip animates, live previews of its tiling windows
//! slide across the screen (see `wm_platform::SlideAnimator`), and the
//! real windows stay cloaked at their final positions. Once the animation
//! ends, the real windows are shown again and the previews are removed.

use std::collections::{HashMap, HashSet};

use tokio::sync::mpsc;
use uuid::Uuid;
use wm_platform::{SlideAnimator, SlideEnded, SlideRequest};

/// Scroll animations of all workspaces.
#[derive(Default)]
pub struct ScrollAnimations {
  /// Animator, if animations are supported and enabled.
  animator: Option<SlideAnimator>,

  /// Generation of the latest animation of each animating workspace.
  /// Entries are removed once the animation has ended.
  active: HashMap<Uuid, u64>,

  /// Workspaces that were displayed at the last sync. Only workspaces
  /// that stay displayed are animated (not e.g. when switching
  /// workspaces).
  displayed: HashSet<Uuid>,
}

impl ScrollAnimations {
  /// Creates the animator.
  ///
  /// Returns the receiver of notifications for ended animations, or `None`
  /// if animations aren't supported on this platform.
  pub fn enable(&mut self) -> Option<mpsc::UnboundedReceiver<SlideEnded>> {
    match SlideAnimator::new() {
      Ok((animator, ended_rx)) => {
        self.animator = Some(animator);
        Some(ended_rx)
      }
      Err(err) => {
        tracing::info!("Scroll animations are unavailable: {}", err);
        None
      }
    }
  }

  /// Whether the given workspace's strip is currently animating.
  pub fn is_animating(&self, workspace_id: Uuid) -> bool {
    self.active.contains_key(&workspace_id)
  }

  /// IDs of workspaces whose strip is currently animating.
  pub fn animating_workspace_ids(&self) -> Vec<Uuid> {
    self.active.keys().copied().collect()
  }

  /// Whether the given workspace was displayed at the last sync.
  pub fn was_displayed(&self, workspace_id: Uuid) -> bool {
    self.displayed.contains(&workspace_id)
  }

  /// Records which workspaces are displayed as of this sync.
  pub fn set_displayed(&mut self, workspace_ids: HashSet<Uuid>) {
    self.displayed = workspace_ids;
  }

  /// Starts or retargets the animation of a workspace's strip.
  ///
  /// Returns whether a new animation was started (or an existing one
  /// retargeted), in which case the workspace's windows need to be
  /// redrawn. On failure, the strip is simply not animated.
  pub fn start(
    &mut self,
    workspace_id: Uuid,
    request: SlideRequest,
  ) -> bool {
    let Some(animator) = &self.animator else {
      return false;
    };

    match animator.start(request) {
      Ok(generation) => {
        self.active.insert(workspace_id, generation) != Some(generation)
      }
      Err(err) => {
        tracing::warn!("Failed to animate scrolling: {}", err);
        self.cancel(workspace_id);
        false
      }
    }
  }

  /// Handles the end of an animation.
  ///
  /// Returns the ID of the workspace whose animation ended, or `None` if
  /// the notification is outdated (the animation has since been
  /// retargeted or cancelled).
  pub fn end(&mut self, ended: &SlideEnded) -> Option<Uuid> {
    let workspace_id = Uuid::from_u128(ended.key);

    if self.active.get(&workspace_id) != Some(&ended.generation) {
      return None;
    }

    self.active.remove(&workspace_id);
    Some(workspace_id)
  }

  /// Removes the previews of an ended animation. Must be called after the
  /// real windows are shown again.
  pub fn finish(&self, ended: &SlideEnded) {
    if let Some(animator) = &self.animator {
      if let Err(err) = animator.finish(ended.key, ended.generation) {
        tracing::warn!("Failed to finish scroll animation: {}", err);
      }
    }
  }

  /// Stops a workspace's animation and removes its previews immediately.
  pub fn cancel(&mut self, workspace_id: Uuid) {
    self.active.remove(&workspace_id);

    if let Some(animator) = &self.animator {
      if let Err(err) = animator.cancel(workspace_id.as_u128()) {
        tracing::warn!("Failed to cancel scroll animation: {}", err);
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn outdated_end_notifications_are_ignored() {
    let workspace_id = Uuid::new_v4();
    let mut animations = ScrollAnimations::default();
    animations.active.insert(workspace_id, 2);

    let outdated = SlideEnded {
      key: workspace_id.as_u128(),
      generation: 1,
    };
    assert_eq!(animations.end(&outdated), None);
    assert!(animations.is_animating(workspace_id));

    let current = SlideEnded {
      key: workspace_id.as_u128(),
      generation: 2,
    };
    assert_eq!(animations.end(&current), Some(workspace_id));
    assert!(!animations.is_animating(workspace_id));
  }

  #[test]
  fn start_without_animator_does_nothing() {
    let workspace_id = Uuid::new_v4();
    let mut animations = ScrollAnimations::default();

    let request = SlideRequest {
      key: workspace_id.as_u128(),
      clip_rect: wm_platform::Rect::from_xy(0, 0, 100, 100),
      targets: vec![],
      above: vec![],
      duration: std::time::Duration::from_millis(100),
      easing: wm_platform::Easing::default(),
    };

    assert!(!animations.start(workspace_id, request));
    assert!(!animations.is_animating(workspace_id));
  }
}
