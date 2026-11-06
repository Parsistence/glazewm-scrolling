use std::sync::OnceLock;

use windows::{
  core::{w, PCWSTR},
  Win32::{
    Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Dwm::{
      DwmRegisterThumbnail, DwmUnregisterThumbnail,
      DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES,
      DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_RECTSOURCE,
      DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
      CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW,
      SetWindowPos, ShowWindow, HWND_NOTOPMOST, MA_NOACTIVATE,
      SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE,
      WM_MOUSEACTIVATE, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
      WS_POPUP,
    },
  },
};

use crate::{Dispatcher, NativeWindow, Rect};

/// Window class name of preview overlay windows.
const PREVIEW_CLASS_NAME: PCWSTR = w!("GlazeWMWindowPreview");

/// Platform-specific implementation of [`WindowPreview`].
pub(crate) struct WindowPreview {
  /// Handle of the overlay window that displays the thumbnail.
  overlay_handle: isize,

  /// DWM thumbnail ID linking the source window to the overlay.
  thumbnail_id: isize,

  dispatcher: Dispatcher,
}

impl WindowPreview {
  /// Implements [`WindowPreview::new`].
  pub(crate) fn new(
    source: &NativeWindow,
    dispatcher: &Dispatcher,
  ) -> crate::Result<Self> {
    let source_handle = source.id().0;

    // Windows must be created on the event loop thread, since that's the
    // thread that pumps their messages.
    let (overlay_handle, thumbnail_id) = dispatcher.dispatch_sync(
      move || -> crate::Result<(isize, isize)> {
        register_preview_class()?;
        let overlay_handle = create_overlay_window()?;

        // SAFETY: Both handles refer to top-level windows. The thumbnail
        // is unregistered when the preview is dropped.
        match unsafe {
          DwmRegisterThumbnail(HWND(overlay_handle), HWND(source_handle))
        } {
          Ok(thumbnail_id) => Ok((overlay_handle, thumbnail_id)),
          Err(err) => {
            // SAFETY: The overlay window was created above on this thread.
            let _ = unsafe { DestroyWindow(HWND(overlay_handle)) };
            Err(err.into())
          }
        }
      },
    )??;

    Ok(Self {
      overlay_handle,
      thumbnail_id,
      dispatcher: dispatcher.clone(),
    })
  }

  /// Implements [`WindowPreview::show`].
  pub(crate) fn show(
    &self,
    source_rect: &Rect,
    visible_rect: &Rect,
    raise: bool,
  ) -> crate::Result<()> {
    let overlay_handle = self.overlay_handle;
    let thumbnail_id = self.thumbnail_id;

    // Region of the source window to display, relative to its top-left
    // corner.
    let source_crop = RECT {
      left: visible_rect.left - source_rect.left,
      top: visible_rect.top - source_rect.top,
      right: visible_rect.right - source_rect.left,
      bottom: visible_rect.bottom - source_rect.top,
    };

    let destination = RECT {
      left: 0,
      top: 0,
      right: visible_rect.width(),
      bottom: visible_rect.height(),
    };

    let (x, y, width, height) = (
      visible_rect.x(),
      visible_rect.y(),
      visible_rect.width(),
      visible_rect.height(),
    );

    self
      .dispatcher
      .dispatch_sync(move || -> crate::Result<()> {
        let properties = DWM_THUMBNAIL_PROPERTIES {
          dwFlags: DWM_TNP_RECTDESTINATION
            | DWM_TNP_RECTSOURCE
            | DWM_TNP_VISIBLE
            | DWM_TNP_OPACITY
            | DWM_TNP_SOURCECLIENTAREAONLY,
          rcDestination: destination,
          rcSource: source_crop,
          opacity: u8::MAX,
          fVisible: BOOL(1),
          fSourceClientAreaOnly: BOOL(0),
        };

        // SAFETY: The thumbnail ID is valid for the lifetime of `self`.
        unsafe {
          DwmUpdateThumbnailProperties(thumbnail_id, &raw const properties)
        }?;

        let mut flags = SWP_NOACTIVATE | SWP_SHOWWINDOW;
        if !raise {
          flags |= SWP_NOZORDER;
        }

        // SAFETY: The overlay window is owned by this preview.
        unsafe {
          SetWindowPos(
            HWND(overlay_handle),
            HWND_NOTOPMOST,
            x,
            y,
            width,
            height,
            flags,
          )
        }?;

        Ok(())
      })?
  }

  /// Implements [`WindowPreview::hide`].
  pub(crate) fn hide(&self) -> crate::Result<()> {
    let overlay_handle = self.overlay_handle;

    self.dispatcher.dispatch_sync(move || {
      // SAFETY: The overlay window is owned by this preview.
      unsafe { ShowWindow(HWND(overlay_handle), SW_HIDE) };
    })
  }
}

impl Drop for WindowPreview {
  fn drop(&mut self) {
    let overlay_handle = self.overlay_handle;
    let thumbnail_id = self.thumbnail_id;

    // Dispatched asynchronously to avoid blocking (e.g. during shutdown).
    // If the event loop has already stopped, the window is destroyed with
    // the process.
    let _ = self.dispatcher.dispatch_async(move || {
      // SAFETY: The thumbnail and overlay are owned by this preview and
      // not used after this point.
      unsafe {
        let _ = DwmUnregisterThumbnail(thumbnail_id);
        let _ = DestroyWindow(HWND(overlay_handle));
      }
    });
  }
}

/// Registers the window class for overlay windows, once per process.
///
/// Must be called on the event loop thread.
fn register_preview_class() -> crate::Result<()> {
  static REGISTERED: OnceLock<bool> = OnceLock::new();

  let is_registered = *REGISTERED.get_or_init(|| {
    // SAFETY: Registers a class with a valid static window procedure.
    unsafe {
      let Ok(instance) = GetModuleHandleW(None) else {
        return false;
      };

      let class = WNDCLASSW {
        lpfnWndProc: Some(preview_window_proc),
        hInstance: instance.into(),
        lpszClassName: PREVIEW_CLASS_NAME,
        ..Default::default()
      };

      RegisterClassW(&raw const class) != 0
    }
  });

  if is_registered {
    Ok(())
  } else {
    Err(crate::Error::Platform(
      "Failed to register window preview class.".to_string(),
    ))
  }
}

/// Creates a hidden overlay window that never takes focus and is excluded
/// from the taskbar and Alt+Tab.
///
/// Must be called on the event loop thread.
fn create_overlay_window() -> crate::Result<isize> {
  // SAFETY: The class is registered by `register_preview_class`.
  let handle = unsafe {
    let instance = GetModuleHandleW(None)?;

    CreateWindowExW(
      WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
      PREVIEW_CLASS_NAME,
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
      "Failed to create window preview overlay.".to_string(),
    ));
  }

  Ok(handle.0)
}

/// Window procedure of overlay windows.
///
/// Clicks don't activate the overlay; the WM handles them through its
/// global mouse listener.
extern "system" fn preview_window_proc(
  hwnd: HWND,
  message: u32,
  wparam: WPARAM,
  lparam: LPARAM,
) -> LRESULT {
  match message {
    #[allow(clippy::cast_possible_wrap)]
    WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
    // SAFETY: Forwards unhandled messages to the default procedure.
    _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
  }
}
