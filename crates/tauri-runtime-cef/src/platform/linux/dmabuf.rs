use ash::vk;
use cef::{AcceleratedPaintInfo, sys::cef_color_type_t};
use wgpu::TextureUses;
use wgpu::hal::{self, api};

pub struct DmaBufData {
  fds: Vec<std::os::fd::RawFd>,
  format: cef_color_type_t,
  modifier: u64,
  width: u32,
  height: u32,
  strides: Vec<u32>,
  offsets: Vec<u32>,
}

impl DmaBufData {
  pub fn from_info(info: &AcceleratedPaintInfo) -> Self {
    let plane_count = info.plane_count as usize;
    let mut fds = Vec::with_capacity(plane_count);
    let mut strides = Vec::with_capacity(plane_count);
    let mut offsets = Vec::with_capacity(plane_count);
    for i in 0..plane_count {
      let plane = &info.planes[i];
      fds.push(plane.fd);
      strides.push(plane.stride);
      offsets.push(plane.offset as u32);
    }
    Self {
      fds,
      format: *info.format.as_ref(),
      modifier: info.modifier,
      width: info.extra.coded_size.width as u32,
      height: info.extra.coded_size.height as u32,
      strides,
      offsets,
    }
  }

  pub fn import(&self, device: &wgpu::Device) -> Result<wgpu::Texture, String> {
    if self.fds.is_empty() || self.width == 0 || self.height == 0 {
      return Err("invalid DMA-BUF parameters".to_string());
    }

    for &fd in &self.fds {
      if fd < 0 {
        return Err("invalid file descriptor".to_string());
      }
      let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
      if flags == -1 {
        return Err("file descriptor not valid".to_string());
      }
    }

    let wgpu_format = cef_to_wgpu(self.format)?;
    let vk_format = cef_to_vulkan(self.format)?;

    let hal_texture = unsafe {
      let hal_device_guard = device.as_hal::<api::Vulkan>();
      let Some(hal_device) = hal_device_guard else {
        return Err("not using Vulkan backend".to_string());
      };

      let (vk_image, device_memory) = self.create_vulkan_image(&hal_device, vk_format)?;

      <api::Vulkan as hal::Api>::Device::texture_from_raw(
        &hal_device,
        vk_image,
        &hal::TextureDescriptor {
          label: Some("CEF DMA-BUF"),
          size: wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
          },
          mip_level_count: 1,
          sample_count: 1,
          dimension: wgpu::TextureDimension::D2,
          format: wgpu_format,
          usage: TextureUses::COPY_DST | TextureUses::RESOURCE,
          memory_flags: hal::MemoryFlags::empty(),
          view_formats: vec![],
        },
        None,
        hal::vulkan::TextureMemory::Dedicated(device_memory),
      )
    };

    let texture = unsafe {
      device.create_texture_from_hal::<api::Vulkan>(
        hal_texture,
        &wgpu::TextureDescriptor {
          label: Some("CEF DMA-BUF"),
          size: wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
          },
          mip_level_count: 1,
          sample_count: 1,
          dimension: wgpu::TextureDimension::D2,
          format: wgpu_format,
          usage: wgpu::TextureUsages::TEXTURE_BINDING,
          view_formats: &[],
        },
      )
    };

    Ok(texture)
  }

  fn create_vulkan_image(
    &self,
    hal_device: &hal::vulkan::Device,
    vk_format: vk::Format,
  ) -> Result<(vk::Image, vk::DeviceMemory), String> {
    let device = hal_device.raw_device();
    let instance = hal_device.shared_instance().raw_instance();
    let physical_device = hal_device.raw_physical_device();

    let plane_layouts: Vec<vk::SubresourceLayout> = self
      .fds
      .iter()
      .enumerate()
      .map(|(i, _)| vk::SubresourceLayout {
        offset: self.offsets.get(i).copied().unwrap_or(0) as u64,
        size: 0,
        row_pitch: self.strides.get(i).copied().unwrap_or(0) as u64,
        array_pitch: 0,
        depth_pitch: 0,
      })
      .collect();

    let mut drm_format_modifier = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
      .drm_format_modifier(self.modifier)
      .plane_layouts(&plane_layouts);

    let image_create_info = vk::ImageCreateInfo::default()
      .image_type(vk::ImageType::TYPE_2D)
      .format(vk_format)
      .extent(vk::Extent3D {
        width: self.width,
        height: self.height,
        depth: 1,
      })
      .mip_levels(1)
      .array_layers(1)
      .samples(vk::SampleCountFlags::TYPE_1)
      .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
      .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::COLOR_ATTACHMENT)
      .sharing_mode(vk::SharingMode::EXCLUSIVE)
      .push_next(&mut drm_format_modifier);

    let image = unsafe {
      device
        .create_image(&image_create_info, None)
        .map_err(|e| format!("create_image failed: {e:?}"))?
    };

    let memory_requirements = unsafe { device.get_image_memory_requirements(image) };

    let dup_fd = unsafe { libc::dup(self.fds[0]) };
    if dup_fd == -1 {
      unsafe { device.destroy_image(image, None) };
      return Err("failed to duplicate DMA-BUF fd".to_string());
    }

    let mut import_memory_fd = vk::ImportMemoryFdInfoKHR::default()
      .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
      .fd(dup_fd);

    let memory_properties =
      unsafe { instance.get_physical_device_memory_properties(physical_device) };

    let memory_type_index = find_memory_type_index(
      memory_requirements.memory_type_bits,
      vk::MemoryPropertyFlags::empty(),
      &memory_properties,
    )
    .ok_or_else(|| "no suitable memory type".to_string())?;

    let allocate_info = vk::MemoryAllocateInfo::default()
      .allocation_size(memory_requirements.size)
      .memory_type_index(memory_type_index)
      .push_next(&mut import_memory_fd);

    let device_memory = unsafe {
      device.allocate_memory(&allocate_info, None).map_err(|e| {
        device.destroy_image(image, None);
        format!("allocate_memory failed: {e:?}")
      })?
    };

    unsafe {
      device
        .bind_image_memory(image, device_memory, 0)
        .map_err(|e| {
          device.free_memory(device_memory, None);
          device.destroy_image(image, None);
          format!("bind_image_memory failed: {e:?}")
        })?;
    }

    Ok((image, device_memory))
  }
}

fn cef_to_wgpu(format: cef_color_type_t) -> Result<wgpu::TextureFormat, String> {
  match format {
    cef_color_type_t::CEF_COLOR_TYPE_BGRA_8888 => Ok(wgpu::TextureFormat::Bgra8Unorm),
    cef_color_type_t::CEF_COLOR_TYPE_RGBA_8888 => Ok(wgpu::TextureFormat::Rgba8Unorm),
    _ => Err(format!("unsupported color type: {format:?}")),
  }
}

fn cef_to_vulkan(format: cef_color_type_t) -> Result<vk::Format, String> {
  match format {
    cef_color_type_t::CEF_COLOR_TYPE_BGRA_8888 => Ok(vk::Format::B8G8R8A8_UNORM),
    cef_color_type_t::CEF_COLOR_TYPE_RGBA_8888 => Ok(vk::Format::R8G8B8A8_UNORM),
    _ => Err(format!("unsupported color type: {format:?}")),
  }
}

fn find_memory_type_index(
  type_filter: u32,
  properties: vk::MemoryPropertyFlags,
  mem_properties: &vk::PhysicalDeviceMemoryProperties,
) -> Option<u32> {
  (0..mem_properties.memory_type_count).find(|&i| {
    (type_filter & (1 << i)) != 0
      && mem_properties.memory_types[i as usize]
        .property_flags
        .contains(properties)
  })
}
