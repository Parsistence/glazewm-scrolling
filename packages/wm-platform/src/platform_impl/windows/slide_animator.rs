use std::{
  collections::{HashMap, HashSet},
  ffi::c_void,
  sync::{mpsc as std_mpsc, OnceLock},
  thread::{self, JoinHandle},
  time::{Duration, Instant},
};

use tokio::sync::mpsc;
use windows::{
  core::{w, PCWSTR},
  Win32::{
    Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM},
    Graphics::Dwm::{
      DwmFlush, DwmGetCompositionTimingInfo, DwmGetWindowAttribute,
      DwmRegisterThumbnail, DwmUnregisterThumbnail,
      DwmUpdateThumbnailProperties, DWMWA_EXTENDED_FRAME_BOUNDS,
      DWM_THUMBNAIL_PROPERTIES, DWM_TIMING_INFO, DWM_TNP_OPACITY,
      DWM_TNP_RECTDESTINATION, DWM_TNP_RECTSOURCE,
      DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
      CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
      GetTopWindow, GetWindow, GetWindowLongPtrW, GetWindowRect,
      PeekMessageW, RegisterClassW, SetWindowPos, ShowWindow,
      TranslateMessage, GWL_EXSTYLE, GW_HWNDNEXT, GW_HWNDPREV,
      HTTRANSPARENT, HWND_TOP, MA_NOACTIVATE, MSG, PM_REMOVE,
      SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_MOUSEACTIVATE,
      WM_NCHITTEST, WNDCLASSW, WS_EX_NOACTIVATE,
      WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
      WS_POPUP,
    },
  },
};

use super::window_shadow::{ShadowMargins, ShadowSource};
use crate::{
  slide_animator::{Slide, SlideWindow},
  NativeWindow, NativeWindowWindowsExt, Rect, SlideEnded, SlideKey,
  SlideRequest, WindowId,
};

/// Window class name of slide overlay windows.
const OVERLAY_CLASS_NAME: PCWSTR = w!("GlazeWMSlideOverlay");

/// How often the animation thread pumps messages while idle.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How long to wait for the animation thread to respond.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

/// Refresh period used if it can't be queried from DWM (60 Hz).
const FALLBACK_REFRESH_PERIOD: Duration = Duration::from_micros(16_667);

/// Number of cached shadow sources above which unused ones are destroyed.
const MAX_CACHED_SHADOWS: usize = 16;

/// Identifies a shadow source by frame width, frame height and scale
/// factor (in thousandths).
type ShadowKey = (i32, i32, u32);

/// Commands sent to the animation thread.
enum Command {
  Start(SlideRequest, std_mpsc::Sender<crate::Result<u64>>),
  Finish(SlideKey, u64),
  Cancel(SlideKey),
}

/// Platform-specific implementation of [`SlideAnimator`].
pub(crate) struct SlideAnimator {
  command_tx: Option<std_mpsc::Sender<Command>>,
  thread: Option<JoinHandle<()>>,
}

impl SlideAnimator {
  /// Implements [`SlideAnimator::new`].
  pub(crate) fn new(
    ended_tx: mpsc::UnboundedSender<SlideEnded>,
  ) -> crate::Result<Self> {
    let (command_tx, command_rx) = std_mpsc::channel();
    let (ready_tx, ready_rx) = std_mpsc::channel();

    let thread = thread::Builder::new()
      .name("slide-animator".to_string())
      .spawn(move || {
        let registration = register_overlay_class();
        let is_registered = registration.is_ok();
        let _ = ready_tx.send(registration);

        if is_registered {
          AnimationThread::new(ended_tx).run(&command_rx);
        }
      })?;

    ready_rx.recv_timeout(RESPONSE_TIMEOUT)??;

    Ok(Self {
      command_tx: Some(command_tx),
      thread: Some(thread),
    })
  }

  /// Implements [`SlideAnimator::start`].
  pub(crate) fn start(&self, request: SlideRequest) -> crate::Result<u64> {
    let (response_tx, response_rx) = std_mpsc::channel();
    self.send(Command::Start(request, response_tx))?;
    response_rx.recv_timeout(RESPONSE_TIMEOUT)?
  }

  /// Implements [`SlideAnimator::finish`].
  pub(crate) fn finish(
    &self,
    key: SlideKey,
    generation: u64,
  ) -> crate::Result<()> {
    self.send(Command::Finish(key, generation))
  }

  /// Implements [`SlideAnimator::cancel`].
  pub(crate) fn cancel(&self, key: SlideKey) -> crate::Result<()> {
    self.send(Command::Cancel(key))
  }

  /// Sends a command to the animation thread.
  fn send(&self, command: Command) -> crate::Result<()> {
    self
      .command_tx
      .as_ref()
      .ok_or(crate::Error::ChannelSend)?
      .send(command)
      .map_err(|_| crate::Error::ChannelSend)
  }
}

impl Drop for SlideAnimator {
  fn drop(&mut self) {
    // Disconnecting the channel stops the animation thread, which then
    // destroys its overlays.
    self.command_tx = None;

    if let Some(thread) = self.thread.take() {
      let _ = thread.join();
    }
  }
}

/// State of the animation thread.
struct AnimationThread {
  sessions: HashMap<SlideKey, Session>,

  /// Shadow sources by frame size and scale, shared between sessions.
  shadows: HashMap<ShadowKey, ShadowSource>,

  ended_tx: mpsc::UnboundedSender<SlideEnded>,
  next_generation: u64,

  /// Time at which the last frame was presented (approximately).
  last_present: Instant,

  refresh_period: Duration,
}

impl AnimationThread {
  /// Creates the thread state.
  fn new(ended_tx: mpsc::UnboundedSender<SlideEnded>) -> Self {
    Self {
      sessions: HashMap::new(),
      shadows: HashMap::new(),
      ended_tx,
      next_generation: 1,
      last_present: Instant::now(),
      refresh_period: refresh_period(),
    }
  }

  /// Handles commands and renders frames until the command channel is
  /// disconnected.
  fn run(mut self, command_rx: &std_mpsc::Receiver<Command>) {
    loop {
      pump_messages();

      let is_running = self.sessions.values().any(|s| s.is_running);

      // Block while idle; otherwise only handle commands that are already
      // queued before rendering the next frame.
      let command = if is_running {
        match command_rx.try_recv() {
          Ok(command) => Some(command),
          Err(std_mpsc::TryRecvError::Empty) => None,
          Err(std_mpsc::TryRecvError::Disconnected) => break,
        }
      } else {
        match command_rx.recv_timeout(IDLE_POLL_INTERVAL) {
          Ok(command) => Some(command),
          Err(std_mpsc::RecvTimeoutError::Timeout) => None,
          Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
        }
      };

      match command {
        Some(command) => self.handle_command(command),
        None if is_running => self.render_frame(),
        None => {}
      }
    }

    for session in self.sessions.into_values() {
      session.destroy();
    }

    // Shadow sources are destroyed on drop, after the thumbnails of them
    // are unregistered.
    drop(self.shadows);
  }

  /// Handles a command from the WM.
  fn handle_command(&mut self, command: Command) {
    match command {
      Command::Start(request, response_tx) => {
        let result = self.start(&request);

        if let Err(err) = &result {
          tracing::warn!("Failed to start slide animation: {}", err);
        }

        let _ = response_tx.send(result);
      }
      Command::Finish(key, generation) => {
        if let Some(session) = self.sessions.get_mut(&key) {
          if session.generation == generation && !session.is_running {
            // Wait for the real windows to be composited before removing
            // the previews in front of them.
            flush();
            session.hide();
          }
        }
      }
      Command::Cancel(key) => {
        if let Some(session) = self.sessions.get_mut(&key) {
          session.hide();
        }
      }
    }
  }

  /// Starts or retargets an animation and shows its previews at their
  /// start positions.
  fn start(&mut self, request: &SlideRequest) -> crate::Result<u64> {
    let session = match self.sessions.entry(request.key) {
      std::collections::hash_map::Entry::Occupied(entry) => {
        entry.into_mut()
      }
      std::collections::hash_map::Entry::Vacant(entry) => {
        entry.insert(Session::new()?)
      }
    };

    let targets = request
      .targets
      .iter()
      .map(|target| (target.window.id(), target.to.clone()))
      .collect::<Vec<_>>();

    if let Some(slide) = &session.slide {
      if slide.has_targets(&targets)
        && session.clip_rect == request.clip_rect
      {
        return Ok(session.generation);
      }
    }

    let now = Instant::now();
    let windows = request
      .targets
      .iter()
      .map(|target| SlideWindow {
        id: target.window.id(),
        from: target.from.clone(),
        to: target.to.clone(),
      })
      .collect::<Vec<_>>();

    match &mut session.slide {
      Some(slide) => {
        slide.retarget(windows, now, request.duration, request.easing);
      }
      None => {
        session.slide =
          Some(Slide::new(windows, now, request.duration, request.easing));
      }
    }

    session.generation = self.next_generation;
    self.next_generation += 1;
    session.is_running = true;
    session.clip_rect = request.clip_rect.clone();
    session.sync_thumbnails(
      &request.targets,
      f64::from(request.scale_factor),
      &mut self.shadows,
    );
    session.render(now, &self.shadows);
    session.show(&request.above)?;
    let generation = session.generation;
    self.evict_unused_shadows();

    // Make sure the previews are on screen before the caller hides the
    // real windows.
    flush();
    self.last_present = Instant::now();
    self.refresh_period = refresh_period();

    Ok(generation)
  }

  /// Destroys cached shadow sources that no session uses, once there are
  /// more than `MAX_CACHED_SHADOWS`.
  fn evict_unused_shadows(&mut self) {
    if self.shadows.len() <= MAX_CACHED_SHADOWS {
      return;
    }

    let used = self
      .sessions
      .values()
      .flat_map(|session| &session.entries)
      .filter_map(|entry| entry.shadow.as_ref().map(|shadow| shadow.key))
      .collect::<HashSet<_>>();

    self.shadows.retain(|key, _| used.contains(key));
  }

  /// Renders the next frame of all running animations, and notifies the
  /// WM of animations that have ended.
  fn render_frame(&mut self) {
    // Position previews for when the frame is expected to be presented,
    // which keeps the motion even if a frame is occasionally late.
    let present_at =
      (self.last_present + self.refresh_period).max(Instant::now());

    for (key, session) in &mut self.sessions {
      if !session.is_running {
        continue;
      }

      session.render(present_at, &self.shadows);

      let is_done = session
        .slide
        .as_ref()
        .is_none_or(|slide| slide.is_done(present_at));

      if is_done {
        session.is_running = false;

        let _ = self.ended_tx.send(SlideEnded {
          key: *key,
          generation: session.generation,
        });
      }
    }

    flush();
    self.last_present = Instant::now();
  }
}

/// Distances between a window's outer frame and its visible frame, i.e.
/// the size of its invisible resize borders.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FrameInsets {
  left: i32,
  top: i32,
  right: i32,
  bottom: i32,
}

impl FrameInsets {
  /// Queries a window's frame insets. Falls back to no insets if they
  /// can't be queried.
  fn of(handle: HWND) -> Self {
    let mut outer = RECT::default();
    let mut visible = RECT::default();

    // SAFETY: Both rects are valid out pointers of the correct size.
    let result = unsafe {
      GetWindowRect(handle, &raw mut outer).and_then(|()| {
        DwmGetWindowAttribute(
          handle,
          DWMWA_EXTENDED_FRAME_BOUNDS,
          (&raw mut visible).cast::<c_void>(),
          // LINT: `RECT` is 16 bytes.
          #[allow(clippy::cast_possible_truncation)]
          {
            std::mem::size_of::<RECT>() as u32
          },
        )
      })
    };

    if result.is_err() {
      return Self::default();
    }

    Self {
      left: (visible.left - outer.left).max(0),
      top: (visible.top - outer.top).max(0),
      right: (outer.right - visible.right).max(0),
      bottom: (outer.bottom - visible.bottom).max(0),
    }
  }

  /// Gets the visible frame of a window whose outer frame is `outer`.
  fn visible_frame(self, outer: &Rect) -> Rect {
    Rect::from_ltrb(
      outer.left + self.left,
      outer.top + self.top,
      outer.right - self.right,
      outer.bottom - self.bottom,
    )
  }
}

/// A window's shadow thumbnail within a session.
struct ShadowEntry {
  thumbnail: isize,
  key: ShadowKey,
  margins: ShadowMargins,
}

/// A window's thumbnails within a session.
struct Entry {
  window_id: WindowId,
  thumbnail: isize,
  shadow: Option<ShadowEntry>,
  insets: FrameInsets,
}

/// An overlay window with previews of the windows of one animation.
struct Session {
  overlay: HWND,

  /// Thumbnails of the target windows, in target order.
  entries: Vec<Entry>,

  slide: Option<Slide>,
  clip_rect: Rect,
  generation: u64,
  is_running: bool,
}

impl Session {
  /// Creates a session with a hidden overlay window.
  fn new() -> crate::Result<Self> {
    Ok(Self {
      overlay: create_overlay_window()?,
      entries: Vec::new(),
      slide: None,
      clip_rect: Rect::from_xy(0, 0, 0, 0),
      generation: 0,
      is_running: false,
    })
  }

  /// Registers thumbnails (and shadow thumbnails) for the target windows,
  /// unless they're already registered for the same windows and shadow
  /// sizes.
  ///
  /// All shadow thumbnails are registered before all window thumbnails,
  /// since later thumbnails are drawn on top. This way, shadows never
  /// cover neighboring windows.
  fn sync_thumbnails(
    &mut self,
    targets: &[crate::SlideTarget],
    scale_factor: f64,
    shadows: &mut HashMap<ShadowKey, ShadowSource>,
  ) {
    let desired = targets
      .iter()
      .map(|target| {
        let insets = FrameInsets::of(target.window.hwnd());
        let frame = insets.visible_frame(&target.to);

        // LINT: Scale factors are small positive numbers.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let key = (
          frame.width(),
          frame.height(),
          (scale_factor * 1000.0).round() as u32,
        );

        (target.window.id(), key, insets)
      })
      .collect::<Vec<_>>();

    let is_unchanged =
      self.entries.len() == desired.len()
        && self.entries.iter().zip(&desired).all(
          |(entry, (id, key, _))| {
            entry.window_id == *id
              && entry
                .shadow
                .as_ref()
                .is_some_and(|shadow| shadow.key == *key)
          },
        );

    if is_unchanged {
      return;
    }

    self.clear_thumbnails();

    let shadow_entries = desired
      .iter()
      .map(|(_, key, _)| self.register_shadow(*key, shadows))
      .collect::<Vec<_>>();

    for ((window_id, _, insets), shadow) in
      desired.into_iter().zip(shadow_entries)
    {
      // SAFETY: Both handles refer to top-level windows. The thumbnail
      // is unregistered when the session is hidden or destroyed.
      match unsafe {
        DwmRegisterThumbnail(self.overlay, HWND(window_id.0))
      } {
        Ok(thumbnail) => self.entries.push(Entry {
          window_id,
          thumbnail,
          shadow,
          insets,
        }),
        Err(err) => {
          tracing::warn!("Failed to register window thumbnail: {}", err);

          if let Some(shadow) = shadow {
            // SAFETY: The thumbnail was registered by this session.
            let _ = unsafe { DwmUnregisterThumbnail(shadow.thumbnail) };
          }
        }
      }
    }
  }

  /// Registers a thumbnail of the shadow source for the given key,
  /// creating the source if needed. Returns `None` on failure, in which
  /// case the window is shown without a shadow.
  fn register_shadow(
    &self,
    key: ShadowKey,
    shadows: &mut HashMap<ShadowKey, ShadowSource>,
  ) -> Option<ShadowEntry> {
    let (width, height, scale) = key;

    let source = match shadows.entry(key) {
      std::collections::hash_map::Entry::Occupied(entry) => {
        entry.into_mut()
      }
      std::collections::hash_map::Entry::Vacant(entry) => {
        match ShadowSource::new(
          OVERLAY_CLASS_NAME,
          width,
          height,
          f64::from(scale) / 1000.0,
        ) {
          Ok(source) => entry.insert(source),
          Err(err) => {
            tracing::warn!("Failed to create window shadow: {}", err);
            return None;
          }
        }
      }
    };

    // SAFETY: Both handles refer to top-level windows. The thumbnail is
    // unregistered before the shadow source is destroyed.
    match unsafe { DwmRegisterThumbnail(self.overlay, source.handle) } {
      Ok(thumbnail) => Some(ShadowEntry {
        thumbnail,
        key,
        margins: source.margins,
      }),
      Err(err) => {
        tracing::warn!("Failed to register shadow thumbnail: {}", err);
        None
      }
    }
  }

  /// Positions all previews and their shadows for the given time.
  fn render(
    &self,
    at: Instant,
    shadows: &HashMap<ShadowKey, ShadowSource>,
  ) {
    let Some(slide) = &self.slide else {
      return;
    };

    for (window_id, rect) in slide.rects_at(at) {
      let Some(entry) = self
        .entries
        .iter()
        .find(|entry| entry.window_id == window_id)
      else {
        continue;
      };

      if let Some(shadow) = &entry.shadow {
        if let Some(source) = shadows.get(&shadow.key) {
          let frame = entry.insets.visible_frame(&rect);
          let shadow_rect = Rect::from_ltrb(
            frame.left - shadow.margins.left,
            frame.top - shadow.margins.top,
            frame.right + shadow.margins.right,
            frame.bottom + shadow.margins.bottom,
          );

          update_thumbnail(
            shadow.thumbnail,
            source.handle,
            &shadow_rect,
            &shadow_rect,
            &self.clip_rect,
          );
        }
      }

      update_thumbnail(
        entry.thumbnail,
        HWND(window_id.0),
        &rect,
        &entry.insets.visible_frame(&rect),
        &self.clip_rect,
      );
    }
  }

  /// Shows the overlay over the clip rect, directly above the highest of
  /// the given windows.
  fn show(&self, above: &[NativeWindow]) -> crate::Result<()> {
    let insert_after = insert_after_for(above, self.overlay);

    // SAFETY: The overlay window is owned by this session.
    unsafe {
      SetWindowPos(
        self.overlay,
        insert_after,
        self.clip_rect.x(),
        self.clip_rect.y(),
        self.clip_rect.width(),
        self.clip_rect.height(),
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
      )
    }?;

    Ok(())
  }

  /// Unregisters all thumbnails.
  fn clear_thumbnails(&mut self) {
    for entry in self.entries.drain(..) {
      // SAFETY: The thumbnails were registered by this session.
      unsafe {
        let _ = DwmUnregisterThumbnail(entry.thumbnail);

        if let Some(shadow) = entry.shadow {
          let _ = DwmUnregisterThumbnail(shadow.thumbnail);
        }
      }
    }
  }

  /// Hides the overlay and releases its thumbnails.
  fn hide(&mut self) {
    // SAFETY: The overlay window is owned by this session.
    unsafe { ShowWindow(self.overlay, SW_HIDE) };
    self.clear_thumbnails();
    self.slide = None;
    self.is_running = false;
  }

  /// Releases the thumbnails and destroys the overlay window.
  fn destroy(mut self) {
    self.hide();

    // SAFETY: The overlay window is owned by this session and not used
    // after this point.
    let _ = unsafe { DestroyWindow(self.overlay) };
  }
}

/// Draws the part `drawn_rect` of a window's thumbnail, as if the
/// window's outer frame were positioned at `rect`. Parts outside of
/// `clip_rect` are cut off.
///
/// `drawn_rect` excludes the invisible resize borders of windows, which
/// thumbnails don't draw as transparent.
fn update_thumbnail(
  thumbnail_id: isize,
  source: HWND,
  rect: &Rect,
  drawn_rect: &Rect,
  clip_rect: &Rect,
) {
  let visible = Rect::from_ltrb(
    drawn_rect.left.max(clip_rect.left),
    drawn_rect.top.max(clip_rect.top),
    drawn_rect.right.min(clip_rect.right),
    drawn_rect.bottom.min(clip_rect.bottom),
  );

  let is_visible = visible.width() > 0
    && visible.height() > 0
    && rect.width() > 0
    && rect.height() > 0;

  // Thumbnail source coordinates are relative to the source window's
  // outer frame (including invisible resize borders), even though
  // `DwmQueryThumbnailSourceSize` reports the size of the visible frame.
  // Scale from `rect` to the outer frame's actual size, since they can
  // differ (e.g. while a column is resized).
  let mut outer_frame = RECT::default();
  // SAFETY: `outer_frame` is a valid out pointer. On failure, `rect`'s
  // size is used.
  let source_size =
    match unsafe { GetWindowRect(source, &raw mut outer_frame) } {
      Ok(()) => SIZE {
        cx: outer_frame.right - outer_frame.left,
        cy: outer_frame.bottom - outer_frame.top,
      },
      Err(_) => SIZE {
        cx: rect.width(),
        cy: rect.height(),
      },
    };

  let scale_x = f64::from(source_size.cx) / f64::from(rect.width().max(1));
  let scale_y =
    f64::from(source_size.cy) / f64::from(rect.height().max(1));

  #[allow(clippy::cast_possible_truncation)]
  let scale = |value: i32, factor: f64| (f64::from(value) * factor) as i32;

  let properties = DWM_THUMBNAIL_PROPERTIES {
    dwFlags: DWM_TNP_RECTDESTINATION
      | DWM_TNP_RECTSOURCE
      | DWM_TNP_VISIBLE
      | DWM_TNP_OPACITY
      | DWM_TNP_SOURCECLIENTAREAONLY,
    rcDestination: RECT {
      left: visible.left - clip_rect.left,
      top: visible.top - clip_rect.top,
      right: visible.right - clip_rect.left,
      bottom: visible.bottom - clip_rect.top,
    },
    rcSource: RECT {
      left: scale(visible.left - rect.left, scale_x),
      top: scale(visible.top - rect.top, scale_y),
      right: scale(visible.right - rect.left, scale_x),
      bottom: scale(visible.bottom - rect.top, scale_y),
    },
    opacity: u8::MAX,
    fVisible: BOOL::from(is_visible),
    fSourceClientAreaOnly: BOOL(0),
  };

  // SAFETY: The thumbnail ID is valid while registered. Failures (e.g.
  // the source window was destroyed) leave the preview unchanged.
  let _ = unsafe {
    DwmUpdateThumbnailProperties(thumbnail_id, &raw const properties)
  };
}

/// Gets the `hWndInsertAfter` argument that places the overlay directly
/// above the highest of the given windows in the z-order.
///
/// Falls back to the top of the non-topmost z-order if none of the windows
/// are found, or if the window above them is topmost.
fn insert_after_for(above: &[NativeWindow], overlay: HWND) -> HWND {
  let handles = above
    .iter()
    .map(|window| window.hwnd().0)
    .collect::<HashSet<_>>();

  // SAFETY: Only walks the z-order of top-level windows.
  unsafe {
    // Windows are enumerated from the top of the z-order downwards.
    let mut current = GetTopWindow(HWND(0));

    while current.0 != 0 && !handles.contains(&current.0) {
      current = GetWindow(current, GW_HWNDNEXT);
    }

    if current.0 == 0 {
      return HWND_TOP;
    }

    let mut previous = GetWindow(current, GW_HWNDPREV);

    if previous == overlay {
      previous = GetWindow(previous, GW_HWNDPREV);
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let is_topmost = previous.0 != 0
      && (GetWindowLongPtrW(previous, GWL_EXSTYLE) as u32
        & WS_EX_TOPMOST.0)
        != 0;

    if previous.0 == 0 || is_topmost {
      HWND_TOP
    } else {
      // Inserting after a window places the overlay directly below it.
      previous
    }
  }
}

/// Waits until DWM has composed the next frame.
fn flush() {
  // SAFETY: No preconditions.
  let _ = unsafe { DwmFlush() };
}

/// Gets the display's refresh period from DWM.
fn refresh_period() -> Duration {
  #[allow(clippy::cast_possible_truncation)]
  let mut info = DWM_TIMING_INFO {
    cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
    ..Default::default()
  };

  // SAFETY: `info` is a valid, correctly sized struct.
  let result =
    unsafe { DwmGetCompositionTimingInfo(HWND(0), &raw mut info) };

  let rate = info.rateRefresh;
  if result.is_err() || rate.uiNumerator == 0 {
    return FALLBACK_REFRESH_PERIOD;
  }

  Duration::from_secs_f64(
    f64::from(rate.uiDenominator) / f64::from(rate.uiNumerator),
  )
}

/// Dispatches pending messages of the overlay windows.
fn pump_messages() {
  let mut message = MSG::default();

  // SAFETY: Standard message loop over this thread's messages.
  unsafe {
    while PeekMessageW(&raw mut message, HWND(0), 0, 0, PM_REMOVE)
      .as_bool()
    {
      let _ = TranslateMessage(&raw const message);
      DispatchMessageW(&raw const message);
    }
  }
}

/// Registers the window class for overlay windows, once per process.
fn register_overlay_class() -> crate::Result<()> {
  static REGISTERED: OnceLock<bool> = OnceLock::new();

  let is_registered = *REGISTERED.get_or_init(|| {
    // SAFETY: Registers a class with a valid static window procedure.
    unsafe {
      let Ok(instance) = GetModuleHandleW(None) else {
        return false;
      };

      let class = WNDCLASSW {
        lpfnWndProc: Some(overlay_window_proc),
        hInstance: instance.into(),
        lpszClassName: OVERLAY_CLASS_NAME,
        ..Default::default()
      };

      RegisterClassW(&raw const class) != 0
    }
  });

  if is_registered {
    Ok(())
  } else {
    Err(crate::Error::Platform(
      "Failed to register slide overlay class.".to_string(),
    ))
  }
}

/// Creates a hidden, transparent overlay window that never takes focus
/// and is excluded from the taskbar and Alt+Tab.
///
/// With `WS_EX_NOREDIRECTIONBITMAP`, the window has no surface of its own,
/// so only its thumbnails are visible.
fn create_overlay_window() -> crate::Result<HWND> {
  // SAFETY: The class is registered by `register_overlay_class`.
  let handle = unsafe {
    let instance = GetModuleHandleW(None)?;

    CreateWindowExW(
      WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
      OVERLAY_CLASS_NAME,
      PCWSTR::null(),
      WS_POPUP,
      0,
      0,
      0,
      0,
      None,
      None,
      instance,
      None,
    )
  };

  if handle.0 == 0 {
    return Err(crate::Error::Platform(
      "Failed to create slide overlay window.".to_string(),
    ));
  }

  Ok(handle)
}

/// Window procedure of overlay windows.
///
/// Overlays are only shown during animations, and never take focus.
extern "system" fn overlay_window_proc(
  hwnd: HWND,
  message: u32,
  wparam: WPARAM,
  lparam: LPARAM,
) -> LRESULT {
  match message {
    #[allow(clippy::cast_possible_wrap)]
    WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
    #[allow(clippy::cast_possible_wrap)]
    WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
    // SAFETY: Forwards unhandled messages to the default procedure.
    _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
  }
}
