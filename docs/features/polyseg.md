# PolySeg - Vector/TSDF Hybrid Draft (SUPERSEDED)

> [!NOTE]
> This document represents the initial draft for the TSDF system. It has been **superseded** by the [Vector-Authoritative Segmentation Architecture](./segmentation_architecture.md). Please refer to the new architecture and [Milestones](./segmentation_milestones.md) for current implementation details.

Targeting **WASM** fundamentally changes the architecture because you lose two luxuries of native development:
1.  **Shared Memory Multithreading:** While `SharedArrayBuffer` exists, it requires strict security headers (COOP/COEP) that make deployment specific. We should design for **Message Passing** (Actors) to be safe.
2.  **Unlimited Memory/Bandwidth:** You cannot re-upload a 512MB 3D texture to the GPU every frame in a browser without killing the framerate.

Here is the **WASM-First Implementation Plan** for the TSDF (Signed Distance Field) approach.

---

### **Core Architecture: The Chunked Actor Model**

Instead of one giant volume, we break the world into **32x32x32 Chunks**. This allows us to update small bits of memory and pass them between the Main Thread (UI/Rendering) and a Web Worker (Physics/Meshing) with zero UI freezing.

#### **1. The Data Structure (`ChunkedMap`)**
To support WASM efficiently, avoid a single contiguous `Vec`.
*   **Struct:** `HashMap<(i16, i16, i16), Chunk>`
*   **Chunk:** A flat `Vec<i8>` or `Box<[i8; 32768]>`.
*   **Benefit:** 32KB per chunk. This is small enough to clone or transfer between threads instantly.

#### **2. The Pipeline**

```mermaid
graph TD
    subgraph "Main Thread (UI & GPU)"
        Input[Mouse Input] -->|1. Raycast| GPU_Brush[Render Brush Cursor]
        Input -->|2. Event| WorkerBridge
        WorkerBridge <-->|5. Recv Mesh & Update Buffers| WGPU[WGPU Renderer]
    end

    subgraph "Worker Thread (Logic)"
        WorkerBridge -->|3. Cmd: Paint(Pos, Radius)| Logic[TSDF Logic]
        Logic -->|4. Generate Surface Nets| Mesher[Meshing]
    end
```

---

### **Phase 1: The "Dual-Backend" Interaction**
We need a system that feels instant even if the Worker takes 50ms to reply.

**The "Visual Lie" (Main Thread):**
1.  **Cursor:** Render the brush as a simple **SDF Sphere** in the shader, or a wireframe sphere actor.
2.  **State:** When the user clicks, you **do not** wait for the mesh to update. You maintain a local "Pending Edits" list if needed, but usually, just showing the brush cursor is enough feedback until the mesh snaps in (approx 20-50ms latency).

---

### **Phase 2: The Worker Logic (Rust)**
This code runs in a `WebWorker` on the browser and a `std::thread` on Native.

**1. The Paint Operation**
When the worker receives `Paint { center, radius }`:
1.  Identify which **Chunks** intersect the brush bounding box (usually 1-8 chunks).
2.  For each Chunk:
    *   Apply the TSDF math (`min(old_val, new_dist)`).
    *   Mark chunk as **Dirty**.

**2. The Meshing (Surface Nets)**
Run Surface Nets *only* on the Dirty Chunks.
*   **Algorithm:**
    *   Iterate 32x32x32 voxels.
    *   If edge crosses 0 (one voxel +ve, one -ve):
        *   Calculate interpolation factor: `t = abs(v1) / (abs(v1) + abs(v2))`.
        *   Output vertex and indices.
*   **Optimization:** Since chunks are small, this takes microseconds.

**3. The Serialization**
*   **Browser:** Convert the resulting Vertex/Index vectors into `Float32Array` / `Uint32Array`. Use `postMessage` with **Transferables** (zero-copy ownership transfer) to send them back to Main.

---

### **Phase 3: The Renderer (WGPU)**
Use `wgpu` because it abstracts WebGL2/WebGPU and Native Vulkan/Metal.

**1. Vertex Buffers**
Don't use one giant buffer for the whole organ.
*   **Strategy:** Each Chunk has its own `wgpu::VertexBuffer` and `wgpu::IndexBuffer`.
*   **Draw Calls:** Modern GPUs (even mobile) can handle thousands of draw calls if the geometry is static. Drawing 500 chunks is fine.
*   **Update:** When the Worker sends back a new mesh for Chunk (5,5,0), you only destroy and recreate the buffer for that specific chunk.

**2. Smooth Shading (The "SDF Look")**
Since we are using TSDF, we can calculate **high-quality normals** directly from the grid data in the worker (using central differences) and send them with the vertices.
*   *Result:* The mesh looks smooth and organic, not faceted.

---

### **Implementation Steps (Rust + WASM)**

#### **Step 1: Dependencies**
```toml
[dependencies]
wgpu = "0.19"
bytemuck = "1.14"
nalgebra = "0.32"
# Crucial for WASM async messaging
gloo-worker = "0.4" 
serde = { version = "1.0", features = ["derive"] }
```

#### **Step 2: The Chunk Logic (Shared Crate)**
Isolate this so it compiles without DOM access.
```rust
// shared/src/lib.rs
pub const CHUNK_SIZE: usize = 32;

pub struct Chunk {
    pub data: Vec<i8>, // Flattened 32x32x32
}

impl Chunk {
    pub fn paint_sphere(&mut self, local_center: Vec3, radius: f32) {
        // ... math implementation ...
    }
    
    pub fn extract_mesh(&self) -> MeshData {
        // ... surface nets implementation ...
    }
}
```

#### **Step 3: The Worker Bridge**
You need an abstraction to handle the difference between Native Threads and Web Workers.

*   **Native:** Just spawn a thread and use `std::sync::mpsc`.
*   **Web:** Use a wrapper.
    ```rust
    // Logic to handle message routing
    pub enum WorkerMessage {
        Paint { pos: [f32;3], radius: f32 },
        LoadVolume { ... }
    }
    
    pub enum MainMessage {
        ChunkUpdate { id: [i32;3], vertices: Vec<f32>, indices: Vec<u32> }
    }
    ```

#### **Step 4: The 3D Viewer (Main Thread)**
```rust
// In your WGPU render loop
fn update(&mut self) {
    // 1. Poll messages from worker
    while let Some(msg) = self.bridge.poll() {
        match msg {
            MainMessage::ChunkUpdate { id, vertices, indices } => {
                // 2. Upload to GPU immediately
                self.renderer.update_chunk_buffer(id, &vertices, &indices);
            }
        }
    }
    
    // 3. Handle Input
    if input.is_dragging {
        // Send command to worker (don't wait for result)
        self.bridge.send(WorkerMessage::Paint { ... });
    }
}
```

### **Why this specific plan for WASM?**

1.  **Memory Safety:** We are not keeping the whole volume in the Main Thread (UI). The UI only holds the *Meshes* (GPU memory) and the *Input State*. The heavy voxel data lives in the Worker.
2.  **No UI Blocking:** Surface Nets calculation happens off-thread. Even if a user creates a massive stroke that touches 50 chunks, the UI stays at 60FPS while the chunks pop in asynchronously.
3.  **Low Bandwidth:** By chunking, we only send small vertex arrays over the JS boundary, not massive 3D textures.

### **Limitations to Watch For**
*   **Undo/Redo:** In this system, "Undo" means rolling back the state of the chunks. You need to store "History States" in the Worker. Since memory is tight in WASM, store Undo history as **Compressed Deltas** (XOR difference) of the affected chunks only.
*   **Seams:** When Meshing Chunk A, you need access to the border of Chunk B to ensure the mesh connects.
    *   *Solution:* Your `Chunk` struct should actually store a 1-voxel "Ghost Region" border (34x34x34), or the Worker needs to look up neighbor chunks during meshing. Storing a 1-voxel overlap is the easiest solution for independent chunk processing.