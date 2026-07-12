use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::sync::Arc;
use wgpu::{
  Buffer, CommandEncoderDescriptor, CompositeAlphaMode, Device, Instance, LoadOp, Operations,
  PipelineLayoutDescriptor, PrimitiveState, Queue, RenderPassColorAttachment, RenderPassDescriptor,
  RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerDescriptor, ShaderModuleDescriptor,
  ShaderSource, Surface, SurfaceConfiguration, TextureFormat, TextureUsages, TextureViewDescriptor,
  VertexBufferLayout,
};

use crate::window_handle::SoftbufferWindowHandle;

const SHADER: &str = r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
  position: [f32; 2],
  uv: [f32; 2],
}

const VERTICES: [Vertex; 4] = [
  Vertex {
    position: [-1.0, 1.0],
    uv: [0.0, 0.0],
  },
  Vertex {
    position: [1.0, 1.0],
    uv: [1.0, 0.0],
  },
  Vertex {
    position: [-1.0, -1.0],
    uv: [0.0, 1.0],
  },
  Vertex {
    position: [1.0, -1.0],
    uv: [1.0, 1.0],
  },
];

pub(crate) struct GpuContext {
  pub instance: Instance,
  pub device: Device,
  pub queue: Queue,
  pub pipeline: RenderPipeline,
  pub sampler: Sampler,
  pub vertex_buffer: Buffer,
  pub bind_group_layout: wgpu::BindGroupLayout,
}

impl GpuContext {
  pub(crate) fn new() -> Option<Arc<Self>> {
    let instance = Instance::new(wgpu::InstanceDescriptor {
      backends: wgpu::Backends::VULKAN,
      ..wgpu::InstanceDescriptor::new_without_display_handle()
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
      power_preference: wgpu::PowerPreference::HighPerformance,
      ..Default::default()
    }))
    .ok()?;

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
      label: Some("elysiae osr gpu"),
      required_features: wgpu::Features::empty(),
      required_limits: wgpu::Limits::default(),
      ..Default::default()
    }))
    .ok()?;

    let shader = device.create_shader_module(ShaderModuleDescriptor {
      label: Some("osr shader"),
      source: ShaderSource::Wgsl(SHADER.into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
      label: Some("osr bind group layout"),
      entries: &[
        wgpu::BindGroupLayoutEntry {
          binding: 0,
          visibility: wgpu::ShaderStages::FRAGMENT,
          ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D2,
          },
          count: None,
        },
        wgpu::BindGroupLayoutEntry {
          binding: 1,
          visibility: wgpu::ShaderStages::FRAGMENT,
          ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
          count: None,
        },
      ],
    });

    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
      label: Some("osr pipeline layout"),
      bind_group_layouts: &[Some(&bind_group_layout)],
      immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
      label: Some("osr render pipeline"),
      layout: Some(&pipeline_layout),
      vertex: wgpu::VertexState {
        module: &shader,
        entry_point: Some("vs_main"),
        compilation_options: Default::default(),
        buffers: &[VertexBufferLayout {
          array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
          step_mode: wgpu::VertexStepMode::Vertex,
          attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
        }],
      },
      fragment: Some(wgpu::FragmentState {
        module: &shader,
        entry_point: Some("fs_main"),
        compilation_options: Default::default(),
        targets: &[Some(wgpu::ColorTargetState {
          format: TextureFormat::Bgra8Unorm,
          blend: Some(wgpu::BlendState::REPLACE),
          write_mask: wgpu::ColorWrites::ALL,
        })],
      }),
      primitive: PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleStrip,
        ..Default::default()
      },
      depth_stencil: None,
      multisample: wgpu::MultisampleState::default(),
      multiview_mask: None,
      cache: None,
    });

    let sampler = device.create_sampler(&SamplerDescriptor {
      label: Some("osr sampler"),
      mag_filter: wgpu::FilterMode::Linear,
      min_filter: wgpu::FilterMode::Linear,
      ..Default::default()
    });

    let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("osr vertex buffer"),
      size: std::mem::size_of::<[Vertex; 4]>() as wgpu::BufferAddress,
      usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&VERTICES));

    Some(Arc::new(Self {
      instance,
      device,
      queue,
      pipeline,
      sampler,
      vertex_buffer,
      bind_group_layout,
    }))
  }
}

pub(crate) struct GpuSurface {
  surface: Surface<'static>,
  width: u32,
  height: u32,
}

impl GpuSurface {
  pub(crate) fn new(handle: SoftbufferWindowHandle, ctx: &GpuContext) -> Option<Self> {
    let raw_display = handle.display_handle().ok()?.as_raw();
    let raw_window = handle.window_handle().ok()?.as_raw();

    let surface = unsafe {
      ctx
        .instance
        .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
          raw_display_handle: Some(raw_display),
          raw_window_handle: raw_window,
        })
        .ok()?
    };

    Some(Self {
      surface,
      width: 0,
      height: 0,
    })
  }

  pub(crate) fn configure(&mut self, ctx: &GpuContext, width: u32, height: u32) {
    if self.width == width && self.height == height && width > 0 && height > 0 {
      return;
    }
    self.width = width.max(1);
    self.height = height.max(1);
    self.surface.configure(
      &ctx.device,
      &SurfaceConfiguration {
        usage: TextureUsages::RENDER_ATTACHMENT,
        format: TextureFormat::Bgra8Unorm,
        width: self.width,
        height: self.height,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: CompositeAlphaMode::Opaque,
        view_formats: vec![],
        desired_maximum_frame_latency: 1,
      },
    );
  }

  pub(crate) fn render_with_bind_group(&mut self, ctx: &GpuContext, bind_group: &wgpu::BindGroup) {
    let surface_texture = match self.surface.get_current_texture() {
      wgpu::CurrentSurfaceTexture::Success(st) | wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
      _ => return,
    };

    let view = surface_texture
      .texture
      .create_view(&TextureViewDescriptor::default());

    let mut encoder = ctx
      .device
      .create_command_encoder(&CommandEncoderDescriptor {
        label: Some("osr encoder"),
      });

    {
      let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("osr render pass"),
        color_attachments: &[Some(RenderPassColorAttachment {
          view: &view,
          depth_slice: None,
          resolve_target: None,
          ops: Operations {
            load: LoadOp::Clear(wgpu::Color::BLACK),
            store: wgpu::StoreOp::Store,
          },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
      });

      pass.set_pipeline(&ctx.pipeline);
      pass.set_bind_group(0, bind_group, &[]);
      pass.set_vertex_buffer(0, ctx.vertex_buffer.slice(..));
      pass.draw(0..4, 0..1);
    }

    ctx.queue.submit(std::iter::once(encoder.finish()));
    surface_texture.present();
  }
}
