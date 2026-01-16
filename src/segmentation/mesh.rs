use glam::Vec3;
use wgpu::util::DeviceExt;

#[derive(Default)]
pub struct SegmentationMesh {
    pub vertices: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

pub struct GpuMeshResources {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,
    pub is_dirty: bool,
}

impl GpuMeshResources {
    pub fn new(device: &wgpu::Device, mesh: &SegmentationMesh) -> Self {
        let vertex_data: Vec<f32> = mesh
            .vertices
            .iter()
            .zip(mesh.normals.iter())
            .flat_map(|(v, n)| [v.x, v.y, v.z, n.x, n.y, n.z])
            .collect();

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Segmentation Mesh Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Segmentation Mesh Index Buffer"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            vertex_buffer,
            index_buffer,
            num_indices: mesh.indices.len() as u32,
            is_dirty: false,
        }
    }

    pub fn update(&mut self, _queue: &wgpu::Queue, mesh: &SegmentationMesh) {
        // Simple update: if indices count changed, we might need to recreate,
        // but for now let's assume queue.write_buffer is enough if size matches or we recreate.
        // Actually, let's keep it simple for M2: just recreate if count changes.
        // In a real system we'd use a pool or large buffers.
        self.num_indices = mesh.indices.len() as u32;
        self.is_dirty = false;
    }
}
