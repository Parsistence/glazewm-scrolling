use std::ffi::c_void;

use windows::{
  core::PCWSTR,
  Win32::{
    Foundation::{BOOL, COLORREF, HWND, POINT, SIZE},
    Graphics::{
      Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK},
      Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
        GetDC, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER,
        BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, DIB_RGB_COLORS,
      },
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
      CreateWindowExW, DestroyWindow, ShowWindow, UpdateLayeredWindow,
      SW_SHOWNOACTIVATE, ULW_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE,
      WS_EX_TOOLWINDOW, WS_POPUP,
    },
  },
};

/// Drop shadow approximating the one Windows 11 draws around windows.
///
/// The shadow is a blurred rectangle relative to the window's visible
/// frame. Fitted to measurements of a window shadow at 150% scale, and
/// given here at 100% scale (96 DPI).
mod model {
  /// Peak opacity of the shadow.
  pub const OPACITY: f64 = 0.64;

  /// How far the shadow's rectangle extends past the left and right
  /// edges of the frame, before blurring.
  pub const SPREAD_X: f64 = 2.25;

  /// How far the shadow's rectangle extends past the top edge. Negative,
  /// since the shadow is cast downwards.
  pub const SPREAD_TOP: f64 = -10.6;

  /// How far the shadow's rectangle extends past the bottom edge.
  pub const SPREAD_BOTTOM: f64 = 23.7;

  /// Standard deviation of the horizontal blur.
  pub const BLUR_X: f64 = 16.1;

  /// Standard deviation of the vertical blur.
  pub const BLUR_Y: f64 = 22.4;
}

/// Space a shadow occupies around a window's visible frame, in pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ShadowMargins {
  pub left: i32,
  pub top: i32,
  pub right: i32,
  pub bottom: i32,
}

impl ShadowMargins {
  /// Gets the margins outside of which the shadow is negligible, for the
  /// given scale factor.
  #[allow(clippy::cast_possible_truncation)]
  pub fn for_scale(scale_factor: f64) -> Self {
    // Beyond 3 standard deviations, the shadow is invisible (<0.1% of its
    // peak opacity).
    let extent = |spread: f64, blur: f64| {
      ((spread + 3.0 * blur) * scale_factor).ceil().max(0.0) as i32
    };

    let horizontal = extent(model::SPREAD_X, model::BLUR_X);

    Self {
      left: horizontal,
      top: extent(model::SPREAD_TOP, model::BLUR_Y),
      right: horizontal,
      bottom: extent(model::SPREAD_BOTTOM, model::BLUR_Y),
    }
  }
}

/// Computes the shadow's opacity (`0.0..=1.0`) along one axis, for each
/// pixel of a span that starts `margin_before` pixels before the frame.
///
/// The shadow along one axis is a box from `-spread_before` to
/// `length + spread_after`, blurred with a Gaussian of standard deviation
/// `blur`.
fn axis_profile(
  length: i32,
  margin_before: i32,
  margin_after: i32,
  spread_before: f64,
  spread_after: f64,
  blur: f64,
) -> Vec<f64> {
  (-margin_before..length + margin_after)
    .map(|position| {
      // Sample at the pixel's center.
      let x = f64::from(position) + 0.5;
      normal_cdf((x + spread_before) / blur)
        - normal_cdf((x - f64::from(length) - spread_after) / blur)
    })
    .collect()
}

/// Renders a shadow for a frame of the given size as premultiplied BGRA
/// pixels (black with varying alpha), row by row.
///
/// The image covers the frame plus `ShadowMargins::for_scale`.
pub(crate) fn render_shadow(
  frame_width: i32,
  frame_height: i32,
  scale_factor: f64,
) -> (ShadowMargins, Vec<u32>) {
  let margins = ShadowMargins::for_scale(scale_factor);

  let columns = axis_profile(
    frame_width,
    margins.left,
    margins.right,
    model::SPREAD_X * scale_factor,
    model::SPREAD_X * scale_factor,
    model::BLUR_X * scale_factor,
  );

  let rows = axis_profile(
    frame_height,
    margins.top,
    margins.bottom,
    model::SPREAD_TOP * scale_factor,
    model::SPREAD_BOTTOM * scale_factor,
    model::BLUR_Y * scale_factor,
  );

  let pixels = rows
    .iter()
    .flat_map(|row| {
      columns.iter().map(move |column| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let alpha = (model::OPACITY * row * column * 255.0)
          .round()
          .clamp(0.0, 255.0) as u32;

        // Black, so the premultiplied color channels are all zero.
        alpha << 24
      })
    })
    .collect();

  (margins, pixels)
}

/// Standard normal cumulative distribution function.
fn normal_cdf(x: f64) -> f64 {
  f64::midpoint(1.0, erf(x / std::f64::consts::SQRT_2))
}

/// Error function (Abramowitz and Stegun 7.1.26; max error 1.5e-7).
fn erf(x: f64) -> f64 {
  let sign = x.signum();
  let x = x.abs();
  let t = 1.0 / (1.0 + 0.327_591_1 * x);
  let polynomial = t
    * (0.254_829_592
      + t
        * (-0.284_496_736
          + t
            * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));

  sign * (1.0 - polynomial * (-x * x).exp())
}

/// A hidden window whose content is a shadow image, used as the source of
/// shadow thumbnails.
///
/// The window is cloaked, so it's never visible itself, while thumbnails
/// of it still show its content.
pub(crate) struct ShadowSource {
  pub handle: HWND,
  pub margins: ShadowMargins,
}

impl ShadowSource {
  /// Creates a shadow source for a frame of the given size.
  ///
  /// Must be called on a thread that pumps messages, using a registered
  /// window class.
  pub fn new(
    class_name: PCWSTR,
    frame_width: i32,
    frame_height: i32,
    scale_factor: f64,
  ) -> crate::Result<Self> {
    let (margins, pixels) =
      render_shadow(frame_width, frame_height, scale_factor);

    let width = frame_width + margins.left + margins.right;
    let height = frame_height + margins.top + margins.bottom;

    // SAFETY: The class is registered by the caller.
    let handle = unsafe {
      CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class_name,
        PCWSTR::null(),
        WS_POPUP,
        0,
        0,
        width,
        height,
        None,
        None,
        GetModuleHandleW(None)?,
        None,
      )
    };

    if handle.0 == 0 {
      return Err(crate::Error::Platform(
        "Failed to create shadow window.".to_string(),
      ));
    }

    let source = Self { handle, margins };
    source.paint(width, height, &pixels)?;

    let cloak = BOOL(1);

    // SAFETY: The window is owned by this thread, and `cloak` is a valid
    // `BOOL` for `DWMWA_CLOAK`.
    unsafe {
      DwmSetWindowAttribute(
        handle,
        DWMWA_CLOAK,
        (&raw const cloak).cast(),
        std::mem::size_of::<BOOL>().try_into()?,
      )?;

      ShowWindow(handle, SW_SHOWNOACTIVATE);
    }

    Ok(source)
  }

  /// Sets the layered window's content to the given premultiplied BGRA
  /// pixels.
  fn paint(
    &self,
    width: i32,
    height: i32,
    pixels: &[u32],
  ) -> crate::Result<()> {
    let info = BITMAPINFO {
      bmiHeader: BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>().try_into()?,
        biWidth: width,
        // Negative for a top-down bitmap.
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        ..Default::default()
      },
      ..Default::default()
    };

    // SAFETY: The DIB section is sized for `width * height` 32-bit pixels,
    // which `pixels` has. All GDI objects are released before returning.
    unsafe {
      let screen = GetDC(HWND(0));
      let memory = CreateCompatibleDC(screen);
      let mut bits: *mut c_void = std::ptr::null_mut();

      let result = CreateDIBSection(
        memory,
        &raw const info,
        DIB_RGB_COLORS,
        &raw mut bits,
        None,
        0,
      )
      .map_err(crate::Error::from)
      .and_then(|bitmap| {
        std::ptr::copy_nonoverlapping(
          pixels.as_ptr(),
          bits.cast::<u32>(),
          pixels.len(),
        );

        let previous = SelectObject(memory, bitmap);
        let blend = BLENDFUNCTION {
          BlendOp: u8::try_from(AC_SRC_OVER).unwrap_or_default(),
          BlendFlags: 0,
          SourceConstantAlpha: u8::MAX,
          AlphaFormat: u8::try_from(AC_SRC_ALPHA).unwrap_or_default(),
        };

        let size = SIZE {
          cx: width,
          cy: height,
        };
        let origin = POINT::default();

        let result = UpdateLayeredWindow(
          self.handle,
          screen,
          None,
          Some(&raw const size),
          memory,
          Some(&raw const origin),
          COLORREF(0),
          Some(&raw const blend),
          ULW_ALPHA,
        );

        SelectObject(memory, previous);
        let _ = DeleteObject(bitmap);
        result.map_err(crate::Error::from)
      });

      let _ = DeleteDC(memory);
      ReleaseDC(HWND(0), screen);
      result
    }
  }
}

impl Drop for ShadowSource {
  fn drop(&mut self) {
    // SAFETY: The window is owned by this source and not used after this
    // point.
    let _ = unsafe { DestroyWindow(self.handle) };
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn shadow_is_strongest_at_the_frame_and_fades_out() {
    let (margins, pixels) = render_shadow(200, 100, 1.0);
    let width = 200 + margins.left + margins.right;
    let alpha = |x: i32, y: i32| pixels[(y * width + x) as usize] >> 24;

    let middle_row = margins.top + 50;
    let at_edge = alpha(margins.left - 1, middle_row);
    let far_out = alpha(0, middle_row);
    let inside = alpha(margins.left + 100, middle_row);

    assert!(at_edge > 40 && at_edge < inside);
    assert_eq!(far_out, 0);
  }

  #[test]
  fn shadow_extends_further_below_than_above() {
    let margins = ShadowMargins::for_scale(1.0);
    assert!(margins.bottom > margins.top);
    assert_eq!(margins.left, margins.right);

    let doubled = ShadowMargins::for_scale(2.0);
    assert!(doubled.bottom > margins.bottom);
  }

  #[test]
  fn erf_matches_known_values() {
    assert!(erf(0.0).abs() < 1e-6);
    assert!((erf(1.0) - 0.842_700_79).abs() < 1e-6);
    assert!((erf(-1.0) + 0.842_700_79).abs() < 1e-6);
  }
}
