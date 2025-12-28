pub fn create_voxel_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler, Vec<u8>) {
    let size = 64u32; // Increased resolution for better details
    let mut texture_data = vec![0u8; (size * size * size * 4) as usize];

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

                // --- Generate "Organs" ---
                let mut density = 0.0;

                // 1. Outer Shell (Soft)
                if dist < max_radius {
                    density += 0.2;
                }

                // 2. Inner Core (Hard/Bone)
                if dist < max_radius * 0.5 {
                    density += 0.8; // High density
                }

                // 3. Some noise/interference to make it look organic
                let noise =
                    ((x as f32 * 0.1).sin() + (y as f32 * 0.2).cos() + (z as f32 * 0.3).sin())
                        * 0.1;
                if dist < max_radius {
                    density += noise;
                }

                // Clamp density 0.0 to 1.0
                density = density.clamp(0.0, 1.0);

                // COLOR MAPPING (Heatmap: Low=Blue, High=Red/White)
                let r = (density * 255.0) as u8;
                let g = (density * density * 255.0) as u8; // Quadratic falloff
                let b = ((1.0 - density) * 100.0) as u8;
                let a = (density * 255.0) as u8;

                texture_data[i] = r;
                texture_data[i + 1] = g;
                texture_data[i + 2] = b;
                texture_data[i + 3] = a;
            }
        }
    }

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Voxel Texture"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: size,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &texture_data,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(4 * size),
            rows_per_image: Some(size),
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: size,
        },
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear, // Linear makes the X-ray smooth!
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    (texture, view, sampler, texture_data)
}
