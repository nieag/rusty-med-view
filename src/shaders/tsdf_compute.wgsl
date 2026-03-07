// TSDF compute shader using Jump Flood Algorithm (JFA).
//
// Builds a truncated signed distance field from a binary labelmap in three
// GPU passes dispatched from `TsdfComputePipeline::build_tsdf_sync`:
//
//   init_seeds   → mark fg/bg boundary voxels as seeds
//   jfa_pass     → propagate nearest seed (repeated ⌈log₂ max_dim⌉ times)
//   finalize_tsdf → convert nearest-seed distance to signed, quantized i16
//
// All shaders work in **ROI space** — a cropped sub-volume containing the
// label boundary plus truncation padding.  The full volume labelmap is also
// uploaded (needed for fast fg/bg lookups by index without offset arithmetic).

// ── Uniforms ─────────────────────────────────────────────────────────────────

struct TsdfUniforms {
    roi_dims:      vec3<u32>,    // ROI sub-volume dimensions [x, y, z]
    step:          u32,          // JFA step size for the current pass
    roi_origin:    vec3<u32>,    // ROI origin in full-volume coordinates
    _pad0:         u32,
    vol_dims:      vec3<u32>,    // Full volume dimensions
    label:         u32,          // Target label value (1–255; 0 → any fg)
    spacing:       vec3<f32>,    // Voxel spacing in mm (x, y, z)
    truncation_mm: f32,          // Truncation distance in mm
    trunc_scale:   f32,          // 32767.0 / truncation_mm  (quantisation scale)
    _pad1:         vec3<f32>,
}

// Group 0: uniforms — bound once and shared across all three passes.
@group(0) @binding(0) var<uniform> u: TsdfUniforms;

// Group 1: data buffers.
//   binding 0  labelmap   — 1 u32 per voxel (label value in low 8 bits)
//   binding 1  seeds      — 1 u32 per ROI voxel; packed (x‖y‖z) seed coords
//                           or NO_SEED sentinel
//   binding 2  tsdf_out   — 1 i32 per ROI voxel; quantised i16 TSDF value
@group(1) @binding(0) var<storage, read>       labelmap:  array<u32>;
@group(1) @binding(1) var<storage, read_write> seeds:     array<u32>;
@group(1) @binding(2) var<storage, read_write> tsdf_out:  array<i32>;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Sentinel value used in `seeds` to signal "no nearest seed found yet".
const NO_SEED: u32 = 0xFFFFFFFFu;

/// Pack ROI-space (x, y, z) into a u32 using 10 bits per component.
/// Maximum representable coordinate per axis: 1023.
fn pack_seed(x: u32, y: u32, z: u32) -> u32 {
    return x | (y << 10u) | (z << 20u);
}

/// Unpack a packed seed into its (x, y, z) ROI-space coordinates.
fn unpack_seed(s: u32) -> vec3<u32> {
    return vec3<u32>(s & 0x3FFu, (s >> 10u) & 0x3FFu, (s >> 20u) & 0x3FFu);
}

/// Flat index into the ROI buffer.
fn roi_idx(pos: vec3<u32>) -> u32 {
    return pos.x + pos.y * u.roi_dims.x + pos.z * u.roi_dims.x * u.roi_dims.y;
}

/// Flat index into the full-volume labelmap.
fn vol_idx(pos: vec3<u32>) -> u32 {
    return pos.x + pos.y * u.vol_dims.x + pos.z * u.vol_dims.x * u.vol_dims.y;
}

/// True when the full-volume voxel at `vol_pos` belongs to the target label.
fn is_fg(vol_pos: vec3<u32>) -> bool {
    if any(vol_pos >= u.vol_dims) { return false; }
    let v = labelmap[vol_idx(vol_pos)];
    if u.label == 0u {
        return v != 0u;
    }
    return v == u.label;
}

/// Squared world-space distance between two ROI-space voxel positions.
fn dist_sq(a: vec3<u32>, b: vec3<u32>) -> f32 {
    let d  = vec3<f32>(a) - vec3<f32>(b);
    let dw = d * u.spacing;
    return dot(dw, dw);
}

// ── Pass 1 — init_seeds ───────────────────────────────────────────────────────
//
// Classify every ROI voxel as a boundary seed (fg/bg transition) or NO_SEED.
// A voxel is a seed when it or any 6-connected neighbour has a different
// fg/bg label — edge-of-volume neighbours count as bg.

@compute @workgroup_size(8, 8, 4)
fn init_seeds(@builtin(global_invocation_id) gid: vec3<u32>) {
    if any(gid >= u.roi_dims) { return; }

    let vol_pos  = gid + u.roi_origin;
    let self_fg  = is_fg(vol_pos);
    var boundary = false;

    let dx = array<i32, 6>( 1, -1,  0,  0,  0,  0);
    let dy = array<i32, 6>( 0,  0,  1, -1,  0,  0);
    let dz = array<i32, 6>( 0,  0,  0,  0,  1, -1);

    for (var i = 0u; i < 6u; i++) {
        let np = vec3<i32>(vol_pos) + vec3<i32>(dx[i], dy[i], dz[i]);
        if any(np < vec3<i32>(0)) || any(vec3<u32>(np) >= u.vol_dims) {
            boundary = true;   // volume edge → always a boundary
            break;
        }
        if is_fg(vec3<u32>(np)) != self_fg {
            boundary = true;
            break;
        }
    }

    seeds[roi_idx(gid)] = select(NO_SEED, pack_seed(gid.x, gid.y, gid.z), boundary);
}

// ── Pass 2 — jfa_pass ────────────────────────────────────────────────────────
//
// One Jump Flood step.  Each voxel examines its 26 neighbours at `u.step`
// distance and keeps the closest known seed.  In-place update is safe because
// each thread writes only to its own index, and JFA tolerates reading stale
// neighbour values (it converges correctly over ⌈log₂ max_dim⌉ passes).

@compute @workgroup_size(8, 8, 4)
fn jfa_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    if any(gid >= u.roi_dims) { return; }

    let idx  = roi_idx(gid);
    let s    = u.step;
    var best = seeds[idx];

    var best_dist: f32;
    if best != NO_SEED {
        best_dist = dist_sq(gid, unpack_seed(best));
    } else {
        best_dist = 1e30;
    }

    for (var dz: i32 = -1; dz <= 1; dz++) {
        for (var dy: i32 = -1; dy <= 1; dy++) {
            for (var dx: i32 = -1; dx <= 1; dx++) {
                if dx == 0 && dy == 0 && dz == 0 { continue; }

                let np = vec3<i32>(gid) + vec3<i32>(dx, dy, dz) * i32(s);
                if any(np < vec3<i32>(0)) || any(vec3<u32>(np) >= u.roi_dims) { continue; }

                let ns = seeds[roi_idx(vec3<u32>(np))];
                if ns == NO_SEED { continue; }

                let d = dist_sq(gid, unpack_seed(ns));
                if d < best_dist {
                    best_dist = d;
                    best      = ns;
                }
            }
        }
    }

    seeds[idx] = best;
}

// ── Pass 3 — finalize_tsdf ───────────────────────────────────────────────────
//
// Convert nearest-seed distance into a signed, truncated, quantised TSDF value:
//   • Negative  → inside  (fg voxel)
//   • Positive  → outside (bg voxel)
//   • Quantised to i16 range via trunc_scale = 32767 / truncation_mm

@compute @workgroup_size(8, 8, 4)
fn finalize_tsdf(@builtin(global_invocation_id) gid: vec3<u32>) {
    if any(gid >= u.roi_dims) { return; }

    let idx     = roi_idx(gid);
    let vol_pos = gid + u.roi_origin;
    let nearest = seeds[idx];

    var sdf_mm: f32;
    if nearest == NO_SEED {
        sdf_mm = u.truncation_mm;
    } else {
        sdf_mm = sqrt(dist_sq(gid, unpack_seed(nearest)));
    }

    // Negate for interior voxels (fg = inside the surface).
    if is_fg(vol_pos) {
        sdf_mm = -sdf_mm;
    }

    sdf_mm = clamp(sdf_mm, -u.truncation_mm, u.truncation_mm);
    tsdf_out[idx] = i32(round(sdf_mm * u.trunc_scale));
}
