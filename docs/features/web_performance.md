# 🛠️ Web/WASM Performance Top‑10 for Labelmap Loading

When running as **web via wasm+webgpu**, the bottlenecks are the contour‑and‑mesh
algorithms running on the main thread.  The table below orders changes by
likely payoff, with browser‑specific notes where relevant.

---

## 1. Avoid full 3‑plane extraction on load  
Generate only the plane(s) the user is viewing/editing.  
Lazy‑compute others when the view changes or on demand.  
⚡ Cuts work by ≈⅔ for most loads.

## 2. Throttle the work over time  
Process one slice or one label at a time and yield (via `requestAnimationFrame`  
or `setTimeout(0)`) so the UI can repaint.  
Add a progress bar.  
This mimics threading on browsers that lack wasm threads.

## 3. Use lighter data structures  
Replace `HashMap<GridPoint,…>` with a pre‑allocated 2‑D buffer or simple edge
list.  
Reuse `Vec`/`HashMap` across slices (`clear()` instead of reallocating).  
Fewer allocations = huge WASM speed/memory win.

## 4. Cache results  
Serialize `ContourSet` or mesh to IndexedDB/localStorage or keep in memory.
Re‑loading the same file becomes instant; progress can be resumed after a
refresh.

## 5. Off‑load to web‑worker / wasm thread  
If threading is enabled, run heavy work in a worker and send back the contours
or mesh via `postMessage`.  
Fallback to slicing (#2) when threading isn’t available.

## 6. Progressive / multi‑resolution mesh  
Build a coarse mesh first (down‑sampled volume or low‑res SDF) for immediate
feedback, then refine in background.

## 7. Tune parameters & expose options  
Default to surface‑nets (cheaper) and moderate `mesh_chunk_size` /
`resolution_multiplier`.  
Add checkboxes like “Import contours” / “Generate mesh” / “High‑quality SDF”
to let users skip expensive paths.

## 8. Profile in the browser  
Use DevTools CPU profiler or `console.time` on the WASM functions to see if
hashing, allocations, or math dominate.  
Target your optimisations accordingly.

## 9. Minimise texture‑upload copies  
Ensure you only copy volume/label data once into a staging buffer, and keep
the original `LoadedLabel.data` for painting.

## 10. Watch memory footprint  
Free unused buffers/contours promptly; WebGPU memory is limited.  
Avoid keeping duplicate full‑volume copies.

---

> ### Quick summary
>  
> **Bytes → loader → `AppEvent` → handlers → ECS/GPU → bind groups → redraw**
>  
> The **heavy** parts are:
> * voxel‑to‑contours (runs during `VolumeLoaded` for labels), and  
> * contours‑to‑SDF/mesh (during frame systems).
>  
> On the web, make these faster, selective, or asynchronous; the event loop
> itself is already lean.
