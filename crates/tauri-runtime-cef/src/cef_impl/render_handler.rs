use std::sync::{Arc, Mutex};

use cef::*;

#[cfg(target_os = "linux")]
use crate::platform::linux::gpu::GpuContext;

#[derive(Default)]
pub(crate) struct OsrBuffer {
  pub width: i32,
  pub height: i32,
  pub data: Vec<u8>,
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
pub(crate) struct GpuTextureEntry {
  pub texture: wgpu::Texture,
  pub view: wgpu::TextureView,
  pub bind_group: wgpu::BindGroup,
}

pub(crate) struct OsrState {
  pub buffer: Mutex<OsrBuffer>,
  pub view_width: Mutex<i32>,
  pub view_height: Mutex<i32>,
  pub scale_factor: Mutex<f64>,
  pub cursor: Mutex<Option<cef::CursorType>>,
  pub needs_redraw: std::sync::atomic::AtomicBool,
  #[cfg(target_os = "linux")]
  pub gpu_texture: Mutex<Option<Arc<GpuTextureEntry>>>,
  #[cfg(target_os = "linux")]
  pub gpu_ctx: Option<Arc<GpuContext>>,
}

impl OsrState {
  pub(crate) fn new(width: i32, height: i32, scale_factor: f64) -> Self {
    Self {
      buffer: Mutex::new(OsrBuffer::default()),
      view_width: Mutex::new(width),
      view_height: Mutex::new(height),
      scale_factor: Mutex::new(scale_factor),
      cursor: Mutex::new(None),
      needs_redraw: std::sync::atomic::AtomicBool::new(false),
      #[cfg(target_os = "linux")]
      gpu_texture: Mutex::new(None),
      #[cfg(target_os = "linux")]
      gpu_ctx: None,
    }
  }

  #[cfg(target_os = "linux")]
  pub(crate) fn new_with_gpu(
    width: i32,
    height: i32,
    scale_factor: f64,
    gpu_ctx: Arc<GpuContext>,
  ) -> Self {
    Self {
      buffer: Mutex::new(OsrBuffer::default()),
      view_width: Mutex::new(width),
      view_height: Mutex::new(height),
      scale_factor: Mutex::new(scale_factor),
      cursor: Mutex::new(None),
      needs_redraw: std::sync::atomic::AtomicBool::new(false),
      gpu_texture: Mutex::new(None),
      gpu_ctx: Some(gpu_ctx),
    }
  }

  pub(crate) fn set_view_size(&self, width: i32, height: i32) {
    *self.view_width.lock().unwrap() = width.max(1);
    *self.view_height.lock().unwrap() = height.max(1);
  }

  pub(crate) fn set_scale_factor(&self, scale: f64) {
    *self.scale_factor.lock().unwrap() = scale;
  }

  pub(crate) fn send_device_metrics(&self, host: &cef::BrowserHost) {
    let scale = *self.scale_factor.lock().unwrap();
    if scale <= 1.0 {
      return;
    }
    host.set_zoom_level(scale.ln() / 1.2f64.ln());
    host.was_resized();
  }

  pub(crate) fn take_needs_redraw(&self) -> bool {
    self
      .needs_redraw
      .swap(false, std::sync::atomic::Ordering::Relaxed)
  }
}

wrap_render_handler! {
  pub(crate) struct TauriCefRenderHandler {
    osr_state: Arc<OsrState>,
  }

  impl RenderHandler {
    fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
      let Some(rect) = rect else { return };
      let scale = *self.osr_state.scale_factor.lock().unwrap();
      let w = (*self.osr_state.view_width.lock().unwrap()).max(1) as f64;
      let h = (*self.osr_state.view_height.lock().unwrap()).max(1) as f64;
      let pw = (w * scale).round() as i32;
      let ph = (h * scale).round() as i32;
      rect.x = 0;
      rect.y = 0;
      rect.width = pw;
      rect.height = ph;
    }

    fn screen_info(
      &self,
      _browser: Option<&mut Browser>,
      info: Option<&mut ScreenInfo>,
    ) -> std::os::raw::c_int {
      let Some(info) = info else { return 0 };
      let scale = *self.osr_state.scale_factor.lock().unwrap();
      let w = (*self.osr_state.view_width.lock().unwrap()).max(1) as f64;
      let h = (*self.osr_state.view_height.lock().unwrap()).max(1) as f64;
      let pw = (w * scale).round() as i32;
      let ph = (h * scale).round() as i32;
      info.device_scale_factor = 1.0;
      info.depth = 24;
      info.depth_per_component = 8;
      info.is_monochrome = 0;
      info.rect = Rect { x: 0, y: 0, width: pw, height: ph };
      info.available_rect = info.rect.clone();
      1
    }

    fn root_screen_rect(
      &self,
      _browser: Option<&mut Browser>,
      rect: Option<&mut Rect>,
    ) -> std::os::raw::c_int {
      let Some(rect) = rect else { return 0 };
      let scale = *self.osr_state.scale_factor.lock().unwrap();
      let w = (*self.osr_state.view_width.lock().unwrap()).max(1) as f64;
      let h = (*self.osr_state.view_height.lock().unwrap()).max(1) as f64;
      rect.x = 0;
      rect.y = 0;
      rect.width = (w * scale).round() as i32;
      rect.height = (h * scale).round() as i32;
      1
    }

    fn on_paint(
      &self,
      _browser: Option<&mut Browser>,
      _type_: PaintElementType,
      _dirty_rects: Option<&[Rect]>,
      buffer: *const u8,
      width: std::os::raw::c_int,
      height: std::os::raw::c_int,
    ) {
      if width <= 0 || height <= 0 || buffer.is_null() {
        return;
      }
      let size = (width as usize) * (height as usize) * 4;
      let pixels = unsafe { std::slice::from_raw_parts(buffer, size) };
      let mut buf = self.osr_state.buffer.lock().unwrap();
      buf.width = width;
      buf.height = height;
      buf.data.clear();
      buf.data.extend_from_slice(pixels);
      drop(buf);
      self
        .osr_state
        .needs_redraw
        .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(target_os = "linux")]
    fn on_accelerated_paint(
      &self,
      _browser: Option<&mut Browser>,
      _type_: PaintElementType,
      _dirty_rects: Option<&[Rect]>,
      info: Option<&AcceleratedPaintInfo>,
    ) {
      let Some(info) = info else { return };
      let Some(ctx) = &self.osr_state.gpu_ctx else { return };

      let dmabuf = crate::platform::linux::dmabuf::DmaBufData::from_info(info);
      let texture = match dmabuf.import(&ctx.device) {
        Ok(t) => t,
        Err(_) => return,
      };

      let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
      let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("osr bind group"),
        layout: &ctx.bind_group_layout,
        entries: &[
          wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
          },
          wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::Sampler(&ctx.sampler),
          },
        ],
      });

      let entry = Arc::new(GpuTextureEntry {
        texture,
        view,
        bind_group,
      });

      *self.osr_state.gpu_texture.lock().unwrap() = Some(entry);

      self
        .osr_state
        .needs_redraw
        .store(true, std::sync::atomic::Ordering::Relaxed);
    }
  }
}
