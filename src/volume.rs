use crate::nifti_loader::LoadedVolume;

/// Information about the created volume for use in ECS
pub struct VolumeInfo {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub intensities: Vec<f32>,
    pub intensity_range: [f32; 2],
}

/// Create a 3D texture and its view/sampler from RGBA data
pub fn create_texture_from_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    rgba_data: &[u8],
    dimensions: [u32; 3],
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let [width, height, depth] = dimensions;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Voxel Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: depth,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let block_size = 4; // Rgba8
    let bytes_per_row = width * block_size;
    let padding = (256 - (bytes_per_row % 256)) % 256;
    let padded_bytes_per_row = bytes_per_row + padding;

    if padding > 0 {
        let mut padded_data = Vec::with_capacity((padded_bytes_per_row * height * depth) as usize);
        for z in 0..depth {
            for y in 0..height {
                let start = ((z * height + y) * bytes_per_row) as usize;
                let end = start + bytes_per_row as usize;
                padded_data.extend_from_slice(&rgba_data[start..end]);
                padded_data.extend(std::iter::repeat(0).take(padding as usize));
            }
        }

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &padded_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: depth,
            },
        );
    } else {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba_data,
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
    }

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    (texture, view, sampler)
}

/// Create texture from a LoadedVolume (from NIfTI loader)
pub fn create_texture_from_nifti(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    loaded: &LoadedVolume,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler, VolumeInfo) {
    let (texture, view, sampler) =
        create_texture_from_rgba(device, queue, &loaded.rgba_data, loaded.dimensions);

    let info = VolumeInfo {
        dimensions: loaded.dimensions,
        spacing: loaded.spacing,
        intensities: loaded.intensity_data.clone(),
        intensity_range: loaded.intensity_range,
    };

    (texture, view, sampler, info)
}

/// Create a demo voxel texture with synthetic data (fallback when no file loaded)
pub fn create_demo_voxel_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler, VolumeInfo) {
    let size = 64u32;
    let mut rgba_data = vec![0u8; (size * size * size * 4) as usize];
    let mut intensities = Vec::with_capacity((size * size * size) as usize);

    let center = size as f32 / 2.0;
    let max_radius = size as f32 / 2.0;

    for z in 0..size {
        for y in 0..size {
            for x in 0..size {
                let i = ((z * size * size + y * size + x) * 4) as usize;

                // Distance from center
                let dx = x as f32 - center;
                let dy = y as f32 - center;
                let dz = z as f32 - center;
                let dist = (dx * dx + dy * dy + dz * dz).sqrt();

                // Generate "Organs"
                let mut density = 0.0;

                // Outer Shell (Soft)
                if dist < max_radius {
                    density += 0.2;
                }

                // Inner Core (Hard/Bone)
                if dist < max_radius * 0.5 {
                    density += 0.8;
                }

                // Noise/interference
                let noise =
                    ((x as f32 * 0.1).sin() + (y as f32 * 0.2).cos() + (z as f32 * 0.3).sin())
                        * 0.1;
                if dist < max_radius {
                    density += noise;
                }

                density = density.clamp(0.0, 1.0);
                intensities.push(density);

                // Heatmap coloring
                let r = (density * 255.0) as u8;
                let g = (density * density * 255.0) as u8;
                let b = ((1.0 - density) * 100.0) as u8;
                let a = (density * 255.0) as u8;

                rgba_data[i] = r;
                rgba_data[i + 1] = g;
                rgba_data[i + 2] = b;
                rgba_data[i + 3] = a;
            }
        }
    }

    let (texture, view, sampler) =
        create_texture_from_rgba(device, queue, &rgba_data, [size, size, size]);

    let info = VolumeInfo {
        dimensions: [size, size, size],
        spacing: [1.0, 1.0, 1.0],
        intensities,
        intensity_range: [0.0, 1.0],
    };

    (texture, view, sampler, info)
}
