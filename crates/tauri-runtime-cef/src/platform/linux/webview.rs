// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use cef::ImplBrowserHost;
use std::os::raw::c_ulong;
use tauri_runtime::dpi::{PhysicalPosition, PhysicalSize, Rect};
use tauri_utils::config::Color;
use x11_dl::xlib;

use crate::{webview::AppWebview, window::AppWindow};

use super::utils::{atom, with_cef_display};

impl AppWebview {
  fn xid(&self) -> Option<c_ulong> {
    let xid = self.host.window_handle();
    if xid == 0 {
      return None;
    }
    // Check if we're on X11 by querying CEF's X display
    if cef::get_xdisplay().is_null() {
      return None;
    }
    Some(xid as c_ulong)
  }

  pub(crate) fn set_background_color(&self, color: Option<Color>) {
    let _ = (self, color);
    // Native child-window background is not equivalent to Chromium's rendered
    // background. Creation still applies BrowserSettings.
  }

  pub(crate) fn bounds(&self) -> Option<Rect> {
    if let Some(state) = self.osr_state.as_ref() {
      let w = *state.view_width.lock().unwrap();
      let h = *state.view_height.lock().unwrap();
      return Some(Rect {
        position: PhysicalPosition::new(0, 0).into(),
        size: PhysicalSize::new(w as u32, h as u32).into(),
      });
    }

    let Some(xid) = self.xid() else {
      return None;
    };

    with_cef_display(None, |xlib, display| unsafe {
      let mut root: xlib::Window = 0;
      let mut x: i32 = 0;
      let mut y: i32 = 0;
      let mut width: u32 = 0;
      let mut height: u32 = 0;
      let mut border_width: u32 = 0;
      let mut depth: u32 = 0;

      if (xlib.XGetGeometry)(
        display,
        xid as xlib::Window,
        &mut root,
        &mut x,
        &mut y,
        &mut width,
        &mut height,
        &mut border_width,
        &mut depth,
      ) == 0
      {
        return None;
      }

      Some(Rect {
        position: PhysicalPosition::new(x, y).into(),
        size: PhysicalSize::new(width, height).into(),
      })
    })
  }

  pub(crate) fn reparent(&self, parent: &AppWindow) {
    if self.osr_state.is_some() {
      return;
    }

    let Some(xid) = self.xid() else {
      return;
    };
    let Some(parent_xid) = parent.xid() else {
      return;
    };

    with_cef_display((), |xlib, display| unsafe {
      (xlib.XReparentWindow)(
        display,
        xid as xlib::Window,
        parent_xid as xlib::Window,
        0,
        0,
      );
      (xlib.XMapRaised)(display, xid as xlib::Window);
    });
  }

  pub(crate) fn apply_visible(&self, visible: bool) {
    if self.osr_state.is_some() {
      return;
    }

    let Some(xid) = self.xid() else {
      return;
    };

    with_cef_display((), |xlib, display| unsafe {
      let net_wm_state = atom(xlib, display, "_NET_WM_STATE");
      const PROP_MODE_REPLACE: i32 = 0;

      if visible {
        (xlib.XChangeProperty)(
          display,
          xid as xlib::Window,
          net_wm_state,
          xlib::XA_ATOM,
          32,
          PROP_MODE_REPLACE,
          std::ptr::null(),
          0,
        );
        (xlib.XMapWindow)(display, xid as xlib::Window);
      } else {
        let hidden: [c_ulong; 1] = [atom(xlib, display, "_NET_WM_STATE_HIDDEN")];
        (xlib.XChangeProperty)(
          display,
          xid as xlib::Window,
          net_wm_state,
          xlib::XA_ATOM,
          32,
          PROP_MODE_REPLACE,
          hidden.as_ptr() as *const u8,
          1,
        );
        (xlib.XUnmapWindow)(display, xid as xlib::Window);
      }
    });
  }

  pub(crate) fn apply_physical_bounds(&self, scale: f64, x: i32, y: i32, width: i32, height: i32) {
    if let Some(state) = &self.osr_state {
      let logical_w = ((width as f64) / scale).round() as i32;
      let logical_h = ((height as f64) / scale).round() as i32;
      state.set_view_size(logical_w.max(1), logical_h.max(1));
      state.send_device_metrics(&self.host);
      return;
    }

    #[cfg(target_os = "linux")]
    if std::env::var("WAYLAND_DISPLAY").is_ok() && std::env::var("ELYSIAE_FORCE_X11").is_err() {
      self.host.notify_move_or_resize_started();
      self.host.was_resized();
      return;
    }

    let Some(xid) = self.xid() else {
      return;
    };

    with_cef_display((), |xlib, display| unsafe {
      (xlib.XMoveResizeWindow)(
        display,
        xid as xlib::Window,
        x,
        y,
        width.max(1) as u32,
        height.max(1) as u32,
      );
    });
  }
}
