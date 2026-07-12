// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use cef::ImplBrowserHost;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::num::NonZeroU32;
use std::os::raw::c_ulong;
use tauri_runtime::ProgressBarState;
use tauri_runtime::dpi::PhysicalSize;
use tauri_utils::config::Color;
use winit::cursor::CursorIcon;

use crate::window::AppWindow;
use crate::window_handle::SoftbufferWindowHandle;

use super::{taskbar, utils::set_wm_state};

fn cef_cursor_to_winit(cursor: &cef::CursorType) -> CursorIcon {
  let raw = cursor.get_raw();
  match raw {
    0 => CursorIcon::Default,       // CT_POINTER
    1 => CursorIcon::Pointer,       // CT_CROSS -> actually crosshair, fix below
    2 => CursorIcon::Pointer,       // CT_HAND
    3 => CursorIcon::Text,          // CT_IBEAM
    4 => CursorIcon::Wait,          // CT_WAIT
    5 => CursorIcon::Help,          // CT_HELP
    6 => CursorIcon::EResize,       // CT_EASTRESIZE
    7 => CursorIcon::NResize,       // CT_NORTHRESIZE
    8 => CursorIcon::NeResize,      // CT_NORTHEASTRESIZE
    9 => CursorIcon::NwResize,      // CT_NORTHWESTRESIZE
    10 => CursorIcon::SResize,      // CT_SOUTHRESIZE
    11 => CursorIcon::SeResize,     // CT_SOUTHEASTRESIZE
    12 => CursorIcon::SwResize,     // CT_SOUTHWESTRESIZE
    13 => CursorIcon::WResize,      // CT_WESTRESIZE
    14 => CursorIcon::NsResize,     // CT_NORTHSOUTHRESIZE
    15 => CursorIcon::EwResize,     // CT_EASTWESTRESIZE
    16 => CursorIcon::NeswResize,   // CT_NORTHEASTSOUTHWESTRESIZE
    17 => CursorIcon::NwseResize,   // CT_NORTHWESTSOUTHEASTRESIZE
    18 => CursorIcon::ColResize,    // CT_COLUMNRESIZE
    19 => CursorIcon::RowResize,    // CT_ROWRESIZE
    24 => CursorIcon::Move,         // CT_MOVE
    25 => CursorIcon::VerticalText, // CT_VERTICALTEXT
    26 => CursorIcon::Cell,         // CT_CELL
    27 => CursorIcon::ContextMenu,  // CT_CONTEXTMENU
    28 => CursorIcon::Alias,        // CT_ALIAS
    29 => CursorIcon::Progress,     // CT_PROGRESS
    30 => CursorIcon::NoDrop,       // CT_NODROP
    31 => CursorIcon::Copy,         // CT_COPY
    32 => CursorIcon::Default,      // CT_NONE
    33 => CursorIcon::NotAllowed,   // CT_NOTALLOWED
    34 => CursorIcon::ZoomIn,       // CT_ZOOMIN
    35 => CursorIcon::ZoomOut,      // CT_ZOOMOUT
    36 => CursorIcon::Grab,         // CT_GRAB
    37 => CursorIcon::Grabbing,     // CT_GRABBING
    _ => CursorIcon::Default,
  }
}

impl AppWindow {
  pub(crate) fn raw_cef_handle(&self) -> cef::sys::cef_window_handle_t {
    match self.window.window_handle() {
      Ok(handle) => match handle.as_raw() {
        RawWindowHandle::Xlib(handle) => handle.window as cef::sys::cef_window_handle_t,
        RawWindowHandle::Xcb(handle) => handle.window.get() as cef::sys::cef_window_handle_t,
        RawWindowHandle::Wayland(handle) => {
          handle.surface.as_ptr() as cef::sys::cef_window_handle_t
        }
        _ => 0,
      },
      Err(_) => 0,
    }
  }

  pub(crate) fn xid(&self) -> Option<c_ulong> {
    let handle = self.window.window_handle().ok()?;
    match handle.as_raw() {
      RawWindowHandle::Xlib(handle) => Some(handle.window as c_ulong),
      RawWindowHandle::Xcb(handle) => Some(handle.window.get() as c_ulong),
      _ => None,
    }
  }

  pub(crate) fn draw_osr_surface(&mut self) {
    let Some(osr_state) = self
      .children
      .iter()
      .filter_map(|c| c.osr_state.as_ref())
      .next()
    else {
      return;
    };

    #[cfg(target_os = "linux")]
    {
      let entry = osr_state.gpu_texture.lock().unwrap().clone();

      if let Some(entry) = entry
        && let Some(ctx) = &self.gpu_ctx
      {
        if self.gpu_surface.is_none()
          && let Some(handle) = SoftbufferWindowHandle::new(self.window.as_ref())
        {
          self.gpu_surface = crate::platform::linux::gpu::GpuSurface::new(handle, ctx);
        }
        if let Some(gs) = &mut self.gpu_surface {
          let scale = *osr_state.scale_factor.lock().unwrap();
          let vw = osr_state.view_width.lock().unwrap().max(1);
          let vh = osr_state.view_height.lock().unwrap().max(1);
          let w = (vw as f64 * scale).round() as u32;
          let h = (vh as f64 * scale).round() as u32;
          gs.configure(ctx, w, h);
          gs.render_with_bind_group(ctx, &entry.bind_group);
        }
        if let Some(cursor) = osr_state.cursor.lock().unwrap().as_ref() {
          let icon = cef_cursor_to_winit(cursor);
          self.window.set_cursor(icon.into());
        }
        let _ = ctx.device.poll(wgpu::PollType::Poll);
        return;
      }
    }

    let buf = osr_state.buffer.lock().unwrap();
    if buf.width <= 0 || buf.height <= 0 || buf.data.is_empty() {
      return;
    }

    let (Some(width), Some(height)) = (
      NonZeroU32::new(buf.width as u32),
      NonZeroU32::new(buf.height as u32),
    ) else {
      return;
    };

    if self.osr_surface.is_none() {
      let Some(handle) = SoftbufferWindowHandle::new(self.window.as_ref()) else {
        return;
      };
      let Ok(context) = softbuffer::Context::new(handle) else {
        return;
      };
      let Ok(surface) = softbuffer::Surface::new(&context, handle) else {
        return;
      };
      self.osr_surface = Some(surface);
    }

    let Some(surface) = &mut self.osr_surface else {
      return;
    };

    if surface.resize(width, height).is_err() {
      return;
    }

    let Ok(mut buffer) = surface.buffer_mut() else {
      return;
    };

    let src = &buf.data;
    let dst = &mut buffer[..];
    let pixel_count = src.len() / 4;
    for i in 0..pixel_count.min(dst.len()) {
      let b = src[i * 4] as u32;
      let g = src[i * 4 + 1] as u32;
      let r = src[i * 4 + 2] as u32;
      dst[i] = (r << 16) | (g << 8) | b;
    }

    let _ = buffer.present();

    if let Some(cursor) = osr_state.cursor.lock().unwrap().as_ref() {
      let icon = cef_cursor_to_winit(cursor);
      self.window.set_cursor(icon.into());
    }
  }

  pub(crate) fn osr_resize(&mut self, size: PhysicalSize<u32>) {
    for child in &self.children {
      if let Some(state) = &child.osr_state {
        let scale = *state.scale_factor.lock().unwrap();
        let logical_w = ((size.width as f64) / scale).round() as i32;
        let logical_h = ((size.height as f64) / scale).round() as i32;
        state.set_view_size(logical_w.max(1), logical_h.max(1));
        let host = &child.host;
        host.notify_move_or_resize_started();
        host.was_resized();
        state.send_device_metrics(host);
      }
    }
  }

  pub(crate) fn set_enabled(&self, enabled: bool) {
    let _ = (self, enabled);
    // TODO: implement native window enabled state on Linux/BSD.
  }

  pub(crate) fn is_enabled(&self) -> bool {
    let _ = self;
    // TODO: query native window enabled state on Linux/BSD.
    true
  }

  pub(crate) fn set_background_color(&self, color: Option<Color>) {
    let Some(xid) = self.xid() else {
      return;
    };
    let Some(color) = color else {
      return;
    };

    super::utils::with_x11((), |xlib, display| unsafe {
      let screen = (xlib.XDefaultScreen)(display);
      let colormap = (xlib.XDefaultColormap)(display, screen);
      let mut xcolor = x11_dl::xlib::XColor {
        pixel: 0,
        red: u16::from(color.0) * 257,
        green: u16::from(color.1) * 257,
        blue: u16::from(color.2) * 257,
        flags: x11_dl::xlib::DoRed | x11_dl::xlib::DoGreen | x11_dl::xlib::DoBlue,
        pad: 0,
      };

      if (xlib.XAllocColor)(display, colormap, &mut xcolor) != 0 {
        (xlib.XSetWindowBackground)(display, xid, xcolor.pixel);
        (xlib.XClearWindow)(display, xid);
      }
    });
  }

  pub(crate) fn set_skip_taskbar(&self, skip: bool) {
    let Some(xid) = self.xid() else {
      return;
    };
    set_wm_state(xid, skip, "_NET_WM_STATE_SKIP_TASKBAR", None);
  }

  pub(crate) fn set_visible_on_all_workspaces(&self, visible: bool) {
    let Some(xid) = self.xid() else {
      return;
    };
    set_wm_state(xid, visible, "_NET_WM_STATE_STICKY", None);
  }

  pub(crate) fn set_progress_bar(&self, state: ProgressBarState) {
    taskbar::set_progress_bar(state);
  }
}
