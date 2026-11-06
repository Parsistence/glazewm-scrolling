use std::{
  collections::{HashMap, HashSet},
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
      DwmFlush, DwmGetCompositionTimingInfo, DwmRegisterThumbnail,
      DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
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
    session.sync_thumbnails(&request.targets);
    session.render(now);
    session.show(&request.above)?;

    // Make sure the previews are on screen before the caller hides the
    // real windows.
    flush();
    self.last_present = Instant::now();
    self.refresh_period = refresh_period();

    Ok(session.generation)
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

      session.render(present_at);

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

/// An overlay window with previews of the windows of one animation.
struct Session {
  overlay: HWND,

  /// DWM thumbnail IDs by source window.
  thumbnails: HashMap<WindowId, isize>,

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
      thumbnails: HashMap::new(),
      slide: None,
      clip_rect: Rect::from_xy(0, 0, 0, 0),
      generation: 0,
      is_running: false,
    })
  }

  /// Registers thumbnails for new target windows, and unregisters those
  /// of windows that are no longer targets.
  fn sync_thumbnails(&mut self, targets: &[crate::SlideTarget]) {
    let target_ids = targets
      .iter()
      .map(|target| target.window.id())
      .collect::<HashSet<_>>();

    self.thumbnails.retain(|window_id, thumbnail_id| {
      let is_target = target_ids.contains(window_id);

      if !is_target {
        // SAFETY: The thumbnail was registered by this session.
        let _ = unsafe { DwmUnregisterThumbnail(*thumbnail_id) };
      }

      is_target
    });

    for target in targets {
      let window_id = target.window.id();

      if self.thumbnails.contains_key(&window_id) {
        continue;
      }

      // SAFETY: Both handles refer to top-level windows. The thumbnail
      // is unregistered when the session is hidden or destroyed.
      match unsafe {
        DwmRegisterThumbnail(self.overlay, target.window.hwnd())
      } {
        Ok(thumbnail_id) => {
          self.thumbnails.insert(window_id, thumbnail_id);
        }
        Err(err) => {
          tracing::warn!("Failed to register window thumbnail: {}", err);
        }
      }
    }
  }

  /// Positions all previews for the given time.
  fn render(&self, at: Instant) {
    let Some(slide) = &self.slide else {
      return;
    };

    for (window_id, rect) in slide.rects_at(at) {
      if let Some(thumbnail_id) = self.thumbnails.get(&window_id) {
        update_thumbnail(
          *thumbnail_id,
          HWND(window_id.0),
          &rect,
          &self.clip_rect,
        );
      }
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

  /// Hides the overlay and releases its thumbnails.
  fn hide(&mut self) {
    // SAFETY: The overlay window is owned by this session.
    unsafe { ShowWindow(self.overlay, SW_HIDE) };
    self.sync_thumbnails(&[]);
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

/// Draws the part of a window's thumbnail that lies within `clip_rect`,
/// as if the window's outer frame were positioned at `rect`.
fn update_thumbnail(
  thumbnail_id: isize,
  source: HWND,
  rect: &Rect,
  clip_rect: &Rect,
) {
  let visible = Rect::from_ltrb(
    rect.left.max(clip_rect.left),
    rect.top.max(clip_rect.top),
    rect.right.min(clip_rect.right),
    rect.bottom.min(clip_rect.bottom),
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
