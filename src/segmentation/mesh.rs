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

    /// Update GPU buffers with new mesh data. Recreates buffers if size changed.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mesh: &SegmentationMesh) {
        let vertex_data: Vec<f32> = mesh
            .vertices
            .iter()
            .zip(mesh.normals.iter())
            .flat_map(|(v, n)| [v.x, v.y, v.z, n.x, n.y, n.z])
            .collect();

        let vertex_byte_size = (vertex_data.len() * std::mem::size_of::<f32>()) as u64;
        let index_byte_size = (mesh.indices.len() * std::mem::size_of::<u32>()) as u64;

        // Recreate buffers if sizes changed
        if vertex_byte_size > self.vertex_buffer.size()
            || index_byte_size > self.index_buffer.size()
        {
            *self = Self::new(device, mesh);
            return;
        }

        // Write to existing buffers
        if !vertex_data.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertex_data));
        }
        if !mesh.indices.is_empty() {
            queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&mesh.indices));
        }
        self.num_indices = mesh.indices.len() as u32;
        self.is_dirty = false;
    }
}
