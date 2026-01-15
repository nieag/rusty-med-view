use crate::components::VolumeData;
use crate::nifti_loader::LoadedVolume;

/// Create a 3D texture from float intensity data (R32Float format)
/// Used for HU-based windowing where raw intensity values are needed
pub fn create_texture_from_float(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    float_data: &[f32],
    dimensions: [u32; 3],
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let [width, height, depth] = dimensions;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Volume Texture (R32Float)"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: depth,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    // Convert f32 to bytes
    let byte_data: &[u8] = bytemuck::cast_slice(float_data);
    let bytes_per_row = width * 4; // 4 bytes per f32

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        byte_data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: depth,
        },
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    // R32Float requires Nearest filtering on WebGL2
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    (texture, view, sampler)
}

/// Create texture from a LoadedVolume (from NIfTI loader)
pub fn create_texture_from_nifti(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    loaded: &LoadedVolume,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler, VolumeData) {
    let (texture, view, sampler) =
        create_texture_from_float(device, queue, &loaded.float_data, loaded.dimensions);

    let volume_data = VolumeData {
        dimensions: loaded.dimensions,
        spacing: loaded.spacing,
        intensities: loaded.float_data.clone(),
        intensity_range: loaded.intensity_range,
        orientation: loaded.orientation,
    };

    (texture, view, sampler, volume_data)
}

// --- NEW Helper Functions for Labelmap Support ---

/// Creates a "dummy" 1x1x1 R8Uint texture initialized to 0.
/// Used for empty texture slots in the shader.
pub fn create_dummy_r8_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let size = wgpu::Extent3d {
        width: 1,
        height: 1,
        depth_or_array_layers: 1,
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Dummy R8 Texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::R8Uint, // Important: UINT format
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    // Initialize with 0
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[0u8],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(1),
            rows_per_image: Some(1),
        },
        size,
    );

    (texture, view, sampler)
}

/// Creates a "dummy" 1x1x1 R32Float texture initialized to 0.0.
/// Used for the main volume slot when no data is loaded.
pub fn create_dummy_r32_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    create_texture_from_float(device, queue, &[0.0], [1, 1, 1])
}

/// Creates a 1D colormap texture (Red/Blue/Green/etc.) for label IDs.
pub fn create_default_colormap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    let width = 256;
    let size = wgpu::Extent3d {
        width,
        height: 1,
        depth_or_array_layers: 1,
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Colormap Texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D1,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut data = Vec::with_capacity((width * 4) as usize);

    for i in 0..width {
        if i == 0 {
            // Label 0 is transparent
            data.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            // Generate distinct colors based on ID
            // Simple hashing strategy to get random-looking but deterministic colors
            let r = (i * 123) % 255;
            let g = (i * 231) % 255;
            let b = (i * 73) % 255;
            data.push(r as u8);
            data.push(g as u8);
            data.push(b as u8);
            data.push(255); // Full opacity (modulated by layer opacity later)
        }
    }

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(1),
        },
        size,
    );

    (texture, view)
}

/// Creates a GPU texture from loaded Labelmap data (R8Uint)
pub fn create_texture_from_labelmap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label_data: &crate::components::LoadedLabel,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let size = wgpu::Extent3d {
        width: label_data.dimensions[0],
        height: label_data.dimensions[1],
        depth_or_array_layers: label_data.dimensions[2],
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("NIfTI Labelmap"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::R8Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &label_data.data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(label_data.dimensions[0]),
            rows_per_image: Some(label_data.dimensions[1]),
        },
        size,
    );

    (texture, view, sampler)
}

/// Create a blank labelmap of given dimensions initialized to 0.
pub fn create_blank_labelmap(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    dimensions: [u32; 3],
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler, Vec<u8>) {
    let [width, height, depth] = dimensions;
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: depth,
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Blank Labelmap"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::R8Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    // Create zeroed CPU data
    let count = (width * height * depth) as usize;
    let data = vec![0u8; count];

    // NOTE: We don't need to write to the texture since create_texture initializes memory to 0 (usually)
    // but to be safe/explicit or if we reused memory, we might want to cleared it.
    // However, wgpu textures from create_texture are lazy-cleared to 0 by default.

    (texture, view, sampler, data)
}
