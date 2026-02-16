//! Marching Cubes algorithm for isosurface extraction.
//!
//! Generates a triangular mesh from a 3D signed distance field.

use crate::app::segment::{MeshData, SdfVolume};

// ============================================================================
// Lookup Tables (Standard Marching Cubes)
// ============================================================================

/// Edge table: for each cube configuration (256), which edges are intersected.
/// Each bit represents an edge (0-11).
#[rustfmt::skip]
const EDGE_TABLE: [u16; 256] = [
    0x000, 0x109, 0x203, 0x30a, 0x406, 0x50f, 0x605, 0x70c,
    0x80c, 0x905, 0xa0f, 0xb06, 0xc0a, 0xd03, 0xe09, 0xf00,
    0x190, 0x099, 0x393, 0x29a, 0x596, 0x49f, 0x795, 0x69c,
    0x99c, 0x895, 0xb9f, 0xa96, 0xd9a, 0xc93, 0xf99, 0xe90,
    0x230, 0x339, 0x033, 0x13a, 0x636, 0x73f, 0x435, 0x53c,
    0xa3c, 0xb35, 0x83f, 0x936, 0xe3a, 0xf33, 0xc39, 0xd30,
    0x3a0, 0x2a9, 0x1a3, 0x0aa, 0x7a6, 0x6af, 0x5a5, 0x4ac,
    0xbac, 0xaa5, 0x9af, 0x8a6, 0xfaa, 0xea3, 0xda9, 0xca0,
    0x460, 0x569, 0x663, 0x76a, 0x066, 0x16f, 0x265, 0x36c,
    0xc6c, 0xd65, 0xe6f, 0xf66, 0x86a, 0x963, 0xa69, 0xb60,
    0x5f0, 0x4f9, 0x7f3, 0x6fa, 0x1f6, 0x0ff, 0x3f5, 0x2fc,
    0xdfc, 0xcf5, 0xfff, 0xef6, 0x9fa, 0x8f3, 0xbf9, 0xaf0,
    0x650, 0x759, 0x453, 0x55a, 0x256, 0x35f, 0x055, 0x15c,
    0xe5c, 0xf55, 0xc5f, 0xd56, 0xa5a, 0xb53, 0x859, 0x950,
    0x7c0, 0x6c9, 0x5c3, 0x4ca, 0x3c6, 0x2cf, 0x1c5, 0x0cc,
    0xfcc, 0xec5, 0xdcf, 0xcc6, 0xbca, 0xac3, 0x9c9, 0x8c0,
    0x8c0, 0x9c9, 0xac3, 0xbca, 0xcc6, 0xdcf, 0xec5, 0xfcc,
    0x0cc, 0x1c5, 0x2cf, 0x3c6, 0x4ca, 0x5c3, 0x6c9, 0x7c0,
    0x950, 0x859, 0xb53, 0xa5a, 0xd56, 0xc5f, 0xf55, 0xe5c,
    0x15c, 0x055, 0x35f, 0x256, 0x55a, 0x453, 0x759, 0x650,
    0xaf0, 0xbf9, 0x8f3, 0x9fa, 0xef6, 0xfff, 0xcf5, 0xdfc,
    0x2fc, 0x3f5, 0x0ff, 0x1f6, 0x6fa, 0x7f3, 0x4f9, 0x5f0,
    0xb60, 0xa69, 0x963, 0x86a, 0xf66, 0xe6f, 0xd65, 0xc6c,
    0x36c, 0x265, 0x16f, 0x066, 0x76a, 0x663, 0x569, 0x460,
    0xca0, 0xda9, 0xea3, 0xfaa, 0x8a6, 0x9af, 0xaa5, 0xbac,
    0x4ac, 0x5a5, 0x6af, 0x7a6, 0x0aa, 0x1a3, 0x2a9, 0x3a0,
    0xd30, 0xc39, 0xf33, 0xe3a, 0x936, 0x83f, 0xb35, 0xa3c,
    0x53c, 0x435, 0x73f, 0x636, 0x13a, 0x033, 0x339, 0x230,
    0xe90, 0xf99, 0xc93, 0xd9a, 0xa96, 0xb9f, 0x895, 0x99c,
    0x69c, 0x795, 0x49f, 0x596, 0x29a, 0x393, 0x099, 0x190,
    0xf00, 0xe09, 0xd03, 0xc0a, 0xb06, 0xa0f, 0x905, 0x80c,
    0x70c, 0x605, 0x50f, 0x406, 0x30a, 0x203, 0x109, 0x000,
];

/// Triangle table: for each cube configuration, list of edge triplets forming triangles.
/// -1 terminates the list.
#[rustfmt::skip]
const TRI_TABLE: [[i8; 16]; 256] = [
    [-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,8,3,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,1,9,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [1,8,3,9,8,1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [1,2,10,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,8,3,1,2,10,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [9,2,10,0,2,9,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [2,8,3,2,10,8,10,9,8,-1,-1,-1,-1,-1,-1,-1],
    [3,11,2,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,11,2,8,11,0,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [1,9,0,2,3,11,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [1,11,2,1,9,11,9,8,11,-1,-1,-1,-1,-1,-1,-1],
    [3,10,1,11,10,3,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,10,1,0,8,10,8,11,10,-1,-1,-1,-1,-1,-1,-1],
    [3,9,0,3,11,9,11,10,9,-1,-1,-1,-1,-1,-1,-1],
    [9,8,10,10,8,11,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [4,7,8,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [4,3,0,7,3,4,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,1,9,8,4,7,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [4,1,9,4,7,1,7,3,1,-1,-1,-1,-1,-1,-1,-1],
    [1,2,10,8,4,7,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [3,4,7,3,0,4,1,2,10,-1,-1,-1,-1,-1,-1,-1],
    [9,2,10,9,0,2,8,4,7,-1,-1,-1,-1,-1,-1,-1],
    [2,10,9,2,9,7,2,7,3,7,9,4,-1,-1,-1,-1],
    [8,4,7,3,11,2,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [11,4,7,11,2,4,2,0,4,-1,-1,-1,-1,-1,-1,-1],
    [9,0,1,8,4,7,2,3,11,-1,-1,-1,-1,-1,-1,-1],
    [4,7,11,9,4,11,9,11,2,9,2,1,-1,-1,-1,-1],
    [3,10,1,3,11,10,7,8,4,-1,-1,-1,-1,-1,-1,-1],
    [1,11,10,1,4,11,1,0,4,7,11,4,-1,-1,-1,-1],
    [4,7,8,9,0,11,9,11,10,11,0,3,-1,-1,-1,-1],
    [4,7,11,4,11,9,9,11,10,-1,-1,-1,-1,-1,-1,-1],
    [9,5,4,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [9,5,4,0,8,3,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,5,4,1,5,0,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [8,5,4,8,3,5,3,1,5,-1,-1,-1,-1,-1,-1,-1],
    [1,2,10,9,5,4,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [3,0,8,1,2,10,4,9,5,-1,-1,-1,-1,-1,-1,-1],
    [5,2,10,5,4,2,4,0,2,-1,-1,-1,-1,-1,-1,-1],
    [2,10,5,3,2,5,3,5,4,3,4,8,-1,-1,-1,-1],
    [9,5,4,2,3,11,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,11,2,0,8,11,4,9,5,-1,-1,-1,-1,-1,-1,-1],
    [0,5,4,0,1,5,2,3,11,-1,-1,-1,-1,-1,-1,-1],
    [2,1,5,2,5,8,2,8,11,4,8,5,-1,-1,-1,-1],
    [10,3,11,10,1,3,9,5,4,-1,-1,-1,-1,-1,-1,-1],
    [4,9,5,0,8,1,8,10,1,8,11,10,-1,-1,-1,-1],
    [5,4,0,5,0,11,5,11,10,11,0,3,-1,-1,-1,-1],
    [5,4,8,5,8,10,10,8,11,-1,-1,-1,-1,-1,-1,-1],
    [9,7,8,5,7,9,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [9,3,0,9,5,3,5,7,3,-1,-1,-1,-1,-1,-1,-1],
    [0,7,8,0,1,7,1,5,7,-1,-1,-1,-1,-1,-1,-1],
    [1,5,3,3,5,7,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [9,7,8,9,5,7,10,1,2,-1,-1,-1,-1,-1,-1,-1],
    [10,1,2,9,5,0,5,3,0,5,7,3,-1,-1,-1,-1],
    [8,0,2,8,2,5,8,5,7,10,5,2,-1,-1,-1,-1],
    [2,10,5,2,5,3,3,5,7,-1,-1,-1,-1,-1,-1,-1],
    [7,9,5,7,8,9,3,11,2,-1,-1,-1,-1,-1,-1,-1],
    [9,5,7,9,7,2,9,2,0,2,7,11,-1,-1,-1,-1],
    [2,3,11,0,1,8,1,7,8,1,5,7,-1,-1,-1,-1],
    [11,2,1,11,1,7,7,1,5,-1,-1,-1,-1,-1,-1,-1],
    [9,5,8,8,5,7,10,1,3,10,3,11,-1,-1,-1,-1],
    [5,7,0,5,0,9,7,11,0,1,0,10,11,10,0,-1],
    [11,10,0,11,0,3,10,5,0,8,0,7,5,7,0,-1],
    [11,10,5,7,11,5,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [10,6,5,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,8,3,5,10,6,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [9,0,1,5,10,6,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [1,8,3,1,9,8,5,10,6,-1,-1,-1,-1,-1,-1,-1],
    [1,6,5,2,6,1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [1,6,5,1,2,6,3,0,8,-1,-1,-1,-1,-1,-1,-1],
    [9,6,5,9,0,6,0,2,6,-1,-1,-1,-1,-1,-1,-1],
    [5,9,8,5,8,2,5,2,6,3,2,8,-1,-1,-1,-1],
    [2,3,11,10,6,5,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [11,0,8,11,2,0,10,6,5,-1,-1,-1,-1,-1,-1,-1],
    [0,1,9,2,3,11,5,10,6,-1,-1,-1,-1,-1,-1,-1],
    [5,10,6,1,9,2,9,11,2,9,8,11,-1,-1,-1,-1],
    [6,3,11,6,5,3,5,1,3,-1,-1,-1,-1,-1,-1,-1],
    [0,8,11,0,11,5,0,5,1,5,11,6,-1,-1,-1,-1],
    [3,11,6,0,3,6,0,6,5,0,5,9,-1,-1,-1,-1],
    [6,5,9,6,9,11,11,9,8,-1,-1,-1,-1,-1,-1,-1],
    [5,10,6,4,7,8,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [4,3,0,4,7,3,6,5,10,-1,-1,-1,-1,-1,-1,-1],
    [1,9,0,5,10,6,8,4,7,-1,-1,-1,-1,-1,-1,-1],
    [10,6,5,1,9,7,1,7,3,7,9,4,-1,-1,-1,-1],
    [6,1,2,6,5,1,4,7,8,-1,-1,-1,-1,-1,-1,-1],
    [1,2,5,5,2,6,3,0,4,3,4,7,-1,-1,-1,-1],
    [8,4,7,9,0,5,0,6,5,0,2,6,-1,-1,-1,-1],
    [7,3,9,7,9,4,3,2,9,5,9,6,2,6,9,-1],
    [3,11,2,7,8,4,10,6,5,-1,-1,-1,-1,-1,-1,-1],
    [5,10,6,4,7,2,4,2,0,2,7,11,-1,-1,-1,-1],
    [0,1,9,4,7,8,2,3,11,5,10,6,-1,-1,-1,-1],
    [9,2,1,9,11,2,9,4,11,7,11,4,5,10,6,-1],
    [8,4,7,3,11,5,3,5,1,5,11,6,-1,-1,-1,-1],
    [5,1,11,5,11,6,1,0,11,7,11,4,0,4,11,-1],
    [0,5,9,0,6,5,0,3,6,11,6,3,8,4,7,-1],
    [6,5,9,6,9,11,4,7,9,7,11,9,-1,-1,-1,-1],
    [10,4,9,6,4,10,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [4,10,6,4,9,10,0,8,3,-1,-1,-1,-1,-1,-1,-1],
    [10,0,1,10,6,0,6,4,0,-1,-1,-1,-1,-1,-1,-1],
    [8,3,1,8,1,6,8,6,4,6,1,10,-1,-1,-1,-1],
    [1,4,9,1,2,4,2,6,4,-1,-1,-1,-1,-1,-1,-1],
    [3,0,8,1,2,9,2,4,9,2,6,4,-1,-1,-1,-1],
    [0,2,4,4,2,6,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [8,3,2,8,2,4,4,2,6,-1,-1,-1,-1,-1,-1,-1],
    [10,4,9,10,6,4,11,2,3,-1,-1,-1,-1,-1,-1,-1],
    [0,8,2,2,8,11,4,9,10,4,10,6,-1,-1,-1,-1],
    [3,11,2,0,1,6,0,6,4,6,1,10,-1,-1,-1,-1],
    [6,4,1,6,1,10,4,8,1,2,1,11,8,11,1,-1],
    [9,6,4,9,3,6,9,1,3,11,6,3,-1,-1,-1,-1],
    [8,11,1,8,1,0,11,6,1,9,1,4,6,4,1,-1],
    [3,11,6,3,6,0,0,6,4,-1,-1,-1,-1,-1,-1,-1],
    [6,4,8,11,6,8,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [7,10,6,7,8,10,8,9,10,-1,-1,-1,-1,-1,-1,-1],
    [0,7,3,0,10,7,0,9,10,6,7,10,-1,-1,-1,-1],
    [10,6,7,1,10,7,1,7,8,1,8,0,-1,-1,-1,-1],
    [10,6,7,10,7,1,1,7,3,-1,-1,-1,-1,-1,-1,-1],
    [1,2,6,1,6,8,1,8,9,8,6,7,-1,-1,-1,-1],
    [2,6,9,2,9,1,6,7,9,0,9,3,7,3,9,-1],
    [7,8,0,7,0,6,6,0,2,-1,-1,-1,-1,-1,-1,-1],
    [7,3,2,6,7,2,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [2,3,11,10,6,8,10,8,9,8,6,7,-1,-1,-1,-1],
    [2,0,7,2,7,11,0,9,7,6,7,10,9,10,7,-1],
    [1,8,0,1,7,8,1,10,7,6,7,10,2,3,11,-1],
    [11,2,1,11,1,7,10,6,1,6,7,1,-1,-1,-1,-1],
    [8,9,6,8,6,7,9,1,6,11,6,3,1,3,6,-1],
    [0,9,1,11,6,7,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [7,8,0,7,0,6,3,11,0,11,6,0,-1,-1,-1,-1],
    [7,11,6,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [7,6,11,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [3,0,8,11,7,6,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,1,9,11,7,6,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [8,1,9,8,3,1,11,7,6,-1,-1,-1,-1,-1,-1,-1],
    [10,1,2,6,11,7,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [1,2,10,3,0,8,6,11,7,-1,-1,-1,-1,-1,-1,-1],
    [2,9,0,2,10,9,6,11,7,-1,-1,-1,-1,-1,-1,-1],
    [6,11,7,2,10,3,10,8,3,10,9,8,-1,-1,-1,-1],
    [7,2,3,6,2,7,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [7,0,8,7,6,0,6,2,0,-1,-1,-1,-1,-1,-1,-1],
    [2,7,6,2,3,7,0,1,9,-1,-1,-1,-1,-1,-1,-1],
    [1,6,2,1,8,6,1,9,8,8,7,6,-1,-1,-1,-1],
    [10,7,6,10,1,7,1,3,7,-1,-1,-1,-1,-1,-1,-1],
    [10,7,6,1,7,10,1,8,7,1,0,8,-1,-1,-1,-1],
    [0,3,7,0,7,10,0,10,9,6,10,7,-1,-1,-1,-1],
    [7,6,10,7,10,8,8,10,9,-1,-1,-1,-1,-1,-1,-1],
    [6,8,4,11,8,6,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [3,6,11,3,0,6,0,4,6,-1,-1,-1,-1,-1,-1,-1],
    [8,6,11,8,4,6,9,0,1,-1,-1,-1,-1,-1,-1,-1],
    [9,4,6,9,6,3,9,3,1,11,3,6,-1,-1,-1,-1],
    [6,8,4,6,11,8,2,10,1,-1,-1,-1,-1,-1,-1,-1],
    [1,2,10,3,0,11,0,6,11,0,4,6,-1,-1,-1,-1],
    [4,11,8,4,6,11,0,2,9,2,10,9,-1,-1,-1,-1],
    [10,9,3,10,3,2,9,4,3,11,3,6,4,6,3,-1],
    [8,2,3,8,4,2,4,6,2,-1,-1,-1,-1,-1,-1,-1],
    [0,4,2,4,6,2,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [1,9,0,2,3,4,2,4,6,4,3,8,-1,-1,-1,-1],
    [1,9,4,1,4,2,2,4,6,-1,-1,-1,-1,-1,-1,-1],
    [8,1,3,8,6,1,8,4,6,6,10,1,-1,-1,-1,-1],
    [10,1,0,10,0,6,6,0,4,-1,-1,-1,-1,-1,-1,-1],
    [4,6,3,4,3,8,6,10,3,0,3,9,10,9,3,-1],
    [10,9,4,6,10,4,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [4,9,5,7,6,11,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,8,3,4,9,5,11,7,6,-1,-1,-1,-1,-1,-1,-1],
    [5,0,1,5,4,0,7,6,11,-1,-1,-1,-1,-1,-1,-1],
    [11,7,6,8,3,4,3,5,4,3,1,5,-1,-1,-1,-1],
    [9,5,4,10,1,2,7,6,11,-1,-1,-1,-1,-1,-1,-1],
    [6,11,7,1,2,10,0,8,3,4,9,5,-1,-1,-1,-1],
    [7,6,11,5,4,10,4,2,10,4,0,2,-1,-1,-1,-1],
    [3,4,8,3,5,4,3,2,5,10,5,2,11,7,6,-1],
    [7,2,3,7,6,2,5,4,9,-1,-1,-1,-1,-1,-1,-1],
    [9,5,4,0,8,6,0,6,2,6,8,7,-1,-1,-1,-1],
    [3,6,2,3,7,6,1,5,0,5,4,0,-1,-1,-1,-1],
    [6,2,8,6,8,7,2,1,8,4,8,5,1,5,8,-1],
    [9,5,4,10,1,6,1,7,6,1,3,7,-1,-1,-1,-1],
    [1,6,10,1,7,6,1,0,7,8,7,0,9,5,4,-1],
    [4,0,10,4,10,5,0,3,10,6,10,7,3,7,10,-1],
    [7,6,10,7,10,8,5,4,10,4,8,10,-1,-1,-1,-1],
    [6,9,5,6,11,9,11,8,9,-1,-1,-1,-1,-1,-1,-1],
    [3,6,11,0,6,3,0,5,6,0,9,5,-1,-1,-1,-1],
    [0,11,8,0,5,11,0,1,5,5,6,11,-1,-1,-1,-1],
    [6,11,3,6,3,5,5,3,1,-1,-1,-1,-1,-1,-1,-1],
    [1,2,10,9,5,11,9,11,8,11,5,6,-1,-1,-1,-1],
    [0,11,3,0,6,11,0,9,6,5,6,9,1,2,10,-1],
    [11,8,5,11,5,6,8,0,5,10,5,2,0,2,5,-1],
    [6,11,3,6,3,5,2,10,3,10,5,3,-1,-1,-1,-1],
    [5,8,9,5,2,8,5,6,2,3,8,2,-1,-1,-1,-1],
    [9,5,6,9,6,0,0,6,2,-1,-1,-1,-1,-1,-1,-1],
    [1,5,8,1,8,0,5,6,8,3,8,2,6,2,8,-1],
    [1,5,6,2,1,6,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [1,3,6,1,6,10,3,8,6,5,6,9,8,9,6,-1],
    [10,1,0,10,0,6,9,5,0,5,6,0,-1,-1,-1,-1],
    [0,3,8,5,6,10,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [10,5,6,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [11,5,10,7,5,11,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [11,5,10,11,7,5,8,3,0,-1,-1,-1,-1,-1,-1,-1],
    [5,11,7,5,10,11,1,9,0,-1,-1,-1,-1,-1,-1,-1],
    [10,7,5,10,11,7,9,8,1,8,3,1,-1,-1,-1,-1],
    [11,1,2,11,7,1,7,5,1,-1,-1,-1,-1,-1,-1,-1],
    [0,8,3,1,2,7,1,7,5,7,2,11,-1,-1,-1,-1],
    [9,7,5,9,2,7,9,0,2,2,11,7,-1,-1,-1,-1],
    [7,5,2,7,2,11,5,9,2,3,2,8,9,8,2,-1],
    [2,5,10,2,3,5,3,7,5,-1,-1,-1,-1,-1,-1,-1],
    [8,2,0,8,5,2,8,7,5,10,2,5,-1,-1,-1,-1],
    [9,0,1,5,10,3,5,3,7,3,10,2,-1,-1,-1,-1],
    [9,8,2,9,2,1,8,7,2,10,2,5,7,5,2,-1],
    [1,3,5,3,7,5,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,8,7,0,7,1,1,7,5,-1,-1,-1,-1,-1,-1,-1],
    [9,0,3,9,3,5,5,3,7,-1,-1,-1,-1,-1,-1,-1],
    [9,8,7,5,9,7,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [5,8,4,5,10,8,10,11,8,-1,-1,-1,-1,-1,-1,-1],
    [5,0,4,5,11,0,5,10,11,11,3,0,-1,-1,-1,-1],
    [0,1,9,8,4,10,8,10,11,10,4,5,-1,-1,-1,-1],
    [10,11,4,10,4,5,11,3,4,9,4,1,3,1,4,-1],
    [2,5,1,2,8,5,2,11,8,4,5,8,-1,-1,-1,-1],
    [0,4,11,0,11,3,4,5,11,2,11,1,5,1,11,-1],
    [0,2,5,0,5,9,2,11,5,4,5,8,11,8,5,-1],
    [9,4,5,2,11,3,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [2,5,10,3,5,2,3,4,5,3,8,4,-1,-1,-1,-1],
    [5,10,2,5,2,4,4,2,0,-1,-1,-1,-1,-1,-1,-1],
    [3,10,2,3,5,10,3,8,5,4,5,8,0,1,9,-1],
    [5,10,2,5,2,4,1,9,2,9,4,2,-1,-1,-1,-1],
    [8,4,5,8,5,3,3,5,1,-1,-1,-1,-1,-1,-1,-1],
    [0,4,5,1,0,5,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [8,4,5,8,5,3,9,0,5,0,3,5,-1,-1,-1,-1],
    [9,4,5,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [4,11,7,4,9,11,9,10,11,-1,-1,-1,-1,-1,-1,-1],
    [0,8,3,4,9,7,9,11,7,9,10,11,-1,-1,-1,-1],
    [1,10,11,1,11,4,1,4,0,7,4,11,-1,-1,-1,-1],
    [3,1,4,3,4,8,1,10,4,7,4,11,10,11,4,-1],
    [4,11,7,9,11,4,9,2,11,9,1,2,-1,-1,-1,-1],
    [9,7,4,9,11,7,9,1,11,2,11,1,0,8,3,-1],
    [11,7,4,11,4,2,2,4,0,-1,-1,-1,-1,-1,-1,-1],
    [11,7,4,11,4,2,8,3,4,3,2,4,-1,-1,-1,-1],
    [2,9,10,2,7,9,2,3,7,7,4,9,-1,-1,-1,-1],
    [9,10,7,9,7,4,10,2,7,8,7,0,2,0,7,-1],
    [3,7,10,3,10,2,7,4,10,1,10,0,4,0,10,-1],
    [1,10,2,8,7,4,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [4,9,1,4,1,7,7,1,3,-1,-1,-1,-1,-1,-1,-1],
    [4,9,1,4,1,7,0,8,1,8,7,1,-1,-1,-1,-1],
    [4,0,3,7,4,3,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [4,8,7,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [9,10,8,10,11,8,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [3,0,9,3,9,11,11,9,10,-1,-1,-1,-1,-1,-1,-1],
    [0,1,10,0,10,8,8,10,11,-1,-1,-1,-1,-1,-1,-1],
    [3,1,10,11,3,10,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [1,2,11,1,11,9,9,11,8,-1,-1,-1,-1,-1,-1,-1],
    [3,0,9,3,9,11,1,2,9,2,11,9,-1,-1,-1,-1],
    [0,2,11,8,0,11,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [3,2,11,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [2,3,8,2,8,10,10,8,9,-1,-1,-1,-1,-1,-1,-1],
    [9,10,2,0,9,2,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [2,3,8,2,8,10,0,1,8,1,10,8,-1,-1,-1,-1],
    [1,10,2,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [1,3,8,9,1,8,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,9,1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [0,3,8,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
    [-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],
];

/// Cube corner offsets [x, y, z] for each of the 8 vertices.
const CORNER_OFFSETS: [[u32; 3]; 8] = [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [1, 1, 1],
    [0, 1, 1],
];

/// Edge endpoint indices (which two corners form each edge).
const EDGE_VERTICES: [[usize; 2]; 12] = [
    [0, 1],
    [1, 2],
    [2, 3],
    [3, 0],
    [4, 5],
    [5, 6],
    [6, 7],
    [7, 4],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
];

// ============================================================================
// Marching Cubes Algorithm
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshNormalMode {
    Gradient,
    Flat,
}

#[derive(Clone, Copy, Debug)]
pub struct MarchingCubesOptions {
    pub enforce_outward_winding: bool,
    pub normal_mode: MeshNormalMode,
    pub restrict_to_active_bounds: bool,
    pub chunk_size: u32,
    pub bounds_override: Option<[u32; 6]>,
}

impl Default for MarchingCubesOptions {
    fn default() -> Self {
        Self {
            enforce_outward_winding: true,
            normal_mode: MeshNormalMode::Gradient,
            restrict_to_active_bounds: true,
            chunk_size: 32,
            bounds_override: None,
        }
    }
}

/// Generate a mesh from an SDF using Marching Cubes.
///
/// # Arguments
/// * `sdf` - The signed distance field
/// * `iso_level` - Isosurface level (usually 0.0 for SDF)
/// * `options` - Quality/performance options for winding and normals
pub fn marching_cubes_with_options(
    sdf: &SdfVolume,
    iso_level: f32,
    options: MarchingCubesOptions,
) -> MeshData {
    let mut mesh = MeshData::new();
    let dims = sdf.dimensions;

    if dims[0] < 2 || dims[1] < 2 || dims[2] < 2 {
        return mesh;
    }

    let (x_start, x_end, y_start, y_end, z_start, z_end) = if let Some(b) = options.bounds_override
    {
        (
            b[0].min(dims[0].saturating_sub(2)),
            b[3].min(dims[0].saturating_sub(2)),
            b[1].min(dims[1].saturating_sub(2)),
            b[4].min(dims[1].saturating_sub(2)),
            b[2].min(dims[2].saturating_sub(2)),
            b[5].min(dims[2].saturating_sub(2)),
        )
    } else if options.restrict_to_active_bounds {
        if let Some(b) = sdf.active_bounds {
            // Expand by one cell so cubes that straddle ROI boundaries are still processed.
            (
                b[0].saturating_sub(1),
                b[3].min(dims[0].saturating_sub(2)),
                b[1].saturating_sub(1),
                b[4].min(dims[1].saturating_sub(2)),
                b[2].saturating_sub(1),
                b[5].min(dims[2].saturating_sub(2)),
            )
        } else {
            (
                0,
                dims[0].saturating_sub(2),
                0,
                dims[1].saturating_sub(2),
                0,
                dims[2].saturating_sub(2),
            )
        }
    } else {
        (
            0,
            dims[0].saturating_sub(2),
            0,
            dims[1].saturating_sub(2),
            0,
            dims[2].saturating_sub(2),
        )
    };

    if x_start > x_end || y_start > y_end || z_start > z_end {
        return mesh;
    }

    let chunk = options.chunk_size.max(1);
    let x_chunk_start = (x_start / chunk) * chunk;
    let y_chunk_start = (y_start / chunk) * chunk;
    let z_chunk_start = (z_start / chunk) * chunk;

    let mut cz = z_chunk_start;
    while cz <= z_end {
        let zc_end = cz.saturating_add(chunk - 1).min(z_end);
        let z0 = cz.max(z_start);

        let mut cy = y_chunk_start;
        while cy <= y_end {
            let yc_end = cy.saturating_add(chunk - 1).min(y_end);
            let y0 = cy.max(y_start);

            let mut cx = x_chunk_start;
            while cx <= x_end {
                let xc_end = cx.saturating_add(chunk - 1).min(x_end);
                let x0 = cx.max(x_start);

                for z in z0..=zc_end {
                    for y in y0..=yc_end {
                        for x in x0..=xc_end {
                            process_cube(sdf, [x, y, z], iso_level, &mut mesh);
                        }
                    }
                }

                if x_end - cx < chunk {
                    break;
                }
                cx += chunk;
            }

            if y_end - cy < chunk {
                break;
            }
            cy += chunk;
        }

        if z_end - cz < chunk {
            break;
        }
        cz += chunk;
    }

    if options.enforce_outward_winding {
        // Ensure triangle winding is coherent with SDF outward gradient so
        // backface culling does not incorrectly reveal interior faces.
        enforce_outward_winding(sdf, &mut mesh);
    }

    match options.normal_mode {
        MeshNormalMode::Gradient => compute_normals_from_sdf(sdf, &mut mesh),
        MeshNormalMode::Flat => compute_flat_normals_from_faces(&mut mesh),
    }

    mesh
}

/// Default high-quality mesh extraction path.
pub fn marching_cubes(sdf: &SdfVolume, iso_level: f32) -> MeshData {
    marching_cubes_with_options(sdf, iso_level, MarchingCubesOptions::default())
}

fn enforce_outward_winding(sdf: &SdfVolume, mesh: &mut MeshData) {
    for tri in mesh.indices.chunks_exact_mut(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= mesh.vertices.len() || i1 >= mesh.vertices.len() || i2 >= mesh.vertices.len() {
            continue;
        }

        let v0 = mesh.vertices[i0];
        let v1 = mesh.vertices[i1];
        let v2 = mesh.vertices[i2];

        let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
        let face = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let face_len2 = face[0] * face[0] + face[1] * face[1] + face[2] * face[2];
        if face_len2 <= 1e-16 {
            continue;
        }

        let centroid = [
            (v0[0] + v1[0] + v2[0]) / 3.0,
            (v0[1] + v1[1] + v2[1]) / 3.0,
            (v0[2] + v1[2] + v2[2]) / 3.0,
        ];
        let grad = compute_gradient_at(sdf, centroid);
        let dot = face[0] * grad[0] + face[1] * grad[1] + face[2] * grad[2];
        if dot < 0.0 {
            tri.swap(1, 2);
        }
    }
}

/// Process a single cube and add triangles to the mesh.
fn process_cube(sdf: &SdfVolume, pos: [u32; 3], iso_level: f32, mesh: &mut MeshData) {
    // Get SDF values at 8 corners
    let mut values = [0.0f32; 8];
    for (i, offset) in CORNER_OFFSETS.iter().enumerate() {
        values[i] = sdf.get(pos[0] + offset[0], pos[1] + offset[1], pos[2] + offset[2]);
    }

    // Build cube index from corner signs
    let mut cube_index = 0u8;
    for i in 0..8 {
        if values[i] < iso_level {
            cube_index |= 1 << i;
        }
    }

    // Skip if cube is entirely inside or outside
    let edges = EDGE_TABLE[cube_index as usize];
    if edges == 0 {
        return;
    }

    // Compute edge intersection points
    let mut edge_vertices = [[0.0f32; 3]; 12];
    for i in 0..12 {
        if (edges & (1 << i)) != 0 {
            let [v0, v1] = EDGE_VERTICES[i];
            edge_vertices[i] = interpolate_vertex(
                sdf,
                pos,
                &CORNER_OFFSETS[v0],
                &CORNER_OFFSETS[v1],
                values[v0],
                values[v1],
                iso_level,
            );
        }
    }

    // Generate triangles from the lookup table
    let tri_row = &TRI_TABLE[cube_index as usize];
    let mut i = 0;
    while i < 16 && tri_row[i] >= 0 {
        let base_idx = mesh.vertices.len() as u32;

        // Add three vertices
        mesh.vertices.push(edge_vertices[tri_row[i] as usize]);
        mesh.vertices.push(edge_vertices[tri_row[i + 1] as usize]);
        mesh.vertices.push(edge_vertices[tri_row[i + 2] as usize]);

        // Add triangle indices
        mesh.indices.push(base_idx);
        mesh.indices.push(base_idx + 1);
        mesh.indices.push(base_idx + 2);

        i += 3;
    }
}

/// Interpolate vertex position along an edge.
fn interpolate_vertex(
    sdf: &SdfVolume,
    cube_pos: [u32; 3],
    offset0: &[u32; 3],
    offset1: &[u32; 3],
    val0: f32,
    val1: f32,
    iso_level: f32,
) -> [f32; 3] {
    // Linear interpolation factor
    let t = if (val1 - val0).abs() > 1e-10 {
        (iso_level - val0) / (val1 - val0)
    } else {
        0.5
    };

    let t = t.clamp(0.0, 1.0);

    // Interpolate in grid space
    let p0 = [
        (cube_pos[0] + offset0[0]) as f32,
        (cube_pos[1] + offset0[1]) as f32,
        (cube_pos[2] + offset0[2]) as f32,
    ];
    let p1 = [
        (cube_pos[0] + offset1[0]) as f32,
        (cube_pos[1] + offset1[1]) as f32,
        (cube_pos[2] + offset1[2]) as f32,
    ];

    let grid_pos = [
        p0[0] + t * (p1[0] - p0[0]),
        p0[1] + t * (p1[1] - p0[1]),
        p0[2] + t * (p1[2] - p0[2]),
    ];

    // Convert to world space without snapping back to integer indices.
    [
        sdf.origin[0] + grid_pos[0] * sdf.spacing[0],
        sdf.origin[1] + grid_pos[1] * sdf.spacing[1],
        sdf.origin[2] + grid_pos[2] * sdf.spacing[2],
    ]
}

/// Compute vertex normals from SDF gradient.
pub fn compute_normals_from_sdf(sdf: &SdfVolume, mesh: &mut MeshData) {
    mesh.normals.clear();
    mesh.normals.reserve(mesh.vertices.len());

    for vertex in &mesh.vertices {
        let normal = compute_gradient_at(sdf, *vertex);
        mesh.normals.push(normal);
    }
}

/// Compute per-vertex flat normals from face orientation.
fn compute_flat_normals_from_faces(mesh: &mut MeshData) {
    mesh.normals.clear();
    mesh.normals.resize(mesh.vertices.len(), [0.0, 0.0, 1.0]);

    for tri in mesh.indices.chunks_exact(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= mesh.vertices.len() || i1 >= mesh.vertices.len() || i2 >= mesh.vertices.len() {
            continue;
        }

        let v0 = mesh.vertices[i0];
        let v1 = mesh.vertices[i1];
        let v2 = mesh.vertices[i2];

        let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
        let nx = e1[1] * e2[2] - e1[2] * e2[1];
        let ny = e1[2] * e2[0] - e1[0] * e2[2];
        let nz = e1[0] * e2[1] - e1[1] * e2[0];
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        let n = if len > 1e-10 {
            [nx / len, ny / len, nz / len]
        } else {
            [0.0, 0.0, 1.0]
        };

        mesh.normals[i0] = n;
        mesh.normals[i1] = n;
        mesh.normals[i2] = n;
    }
}

/// Compute SDF gradient (normalized) at a world position using central differences.
fn compute_gradient_at(sdf: &SdfVolume, world_pos: [f32; 3]) -> [f32; 3] {
    let h = sdf.spacing[0].min(sdf.spacing[1]).min(sdf.spacing[2]) * 0.5;

    let dx = sample_sdf_world(sdf, [world_pos[0] + h, world_pos[1], world_pos[2]])
        - sample_sdf_world(sdf, [world_pos[0] - h, world_pos[1], world_pos[2]]);
    let dy = sample_sdf_world(sdf, [world_pos[0], world_pos[1] + h, world_pos[2]])
        - sample_sdf_world(sdf, [world_pos[0], world_pos[1] - h, world_pos[2]]);
    let dz = sample_sdf_world(sdf, [world_pos[0], world_pos[1], world_pos[2] + h])
        - sample_sdf_world(sdf, [world_pos[0], world_pos[1], world_pos[2] - h]);

    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    if len > 1e-10 {
        [dx / len, dy / len, dz / len]
    } else {
        [0.0, 1.0, 0.0] // Default up normal
    }
}

/// Sample SDF at world position with clamping.
fn sample_sdf_world(sdf: &SdfVolume, world_pos: [f32; 3]) -> f32 {
    let gx = (world_pos[0] - sdf.origin[0]) / sdf.spacing[0];
    let gy = (world_pos[1] - sdf.origin[1]) / sdf.spacing[1];
    let gz = (world_pos[2] - sdf.origin[2]) / sdf.spacing[2];

    let max_x = sdf.dimensions[0] as f32 - 1.0;
    let max_y = sdf.dimensions[1] as f32 - 1.0;
    let max_z = sdf.dimensions[2] as f32 - 1.0;
    if gx < 0.0 || gy < 0.0 || gz < 0.0 || gx > max_x || gy > max_y || gz > max_z {
        return f32::MAX;
    }

    let x0 = gx.floor() as u32;
    let y0 = gy.floor() as u32;
    let z0 = gz.floor() as u32;
    let x1 = (x0 + 1).min(sdf.dimensions[0] - 1);
    let y1 = (y0 + 1).min(sdf.dimensions[1] - 1);
    let z1 = (z0 + 1).min(sdf.dimensions[2] - 1);

    let tx = gx - x0 as f32;
    let ty = gy - y0 as f32;
    let tz = gz - z0 as f32;

    let c000 = sdf.get(x0, y0, z0);
    let c100 = sdf.get(x1, y0, z0);
    let c010 = sdf.get(x0, y1, z0);
    let c110 = sdf.get(x1, y1, z0);
    let c001 = sdf.get(x0, y0, z1);
    let c101 = sdf.get(x1, y0, z1);
    let c011 = sdf.get(x0, y1, z1);
    let c111 = sdf.get(x1, y1, z1);

    let c00 = c000 * (1.0 - tx) + c100 * tx;
    let c10 = c010 * (1.0 - tx) + c110 * tx;
    let c01 = c001 * (1.0 - tx) + c101 * tx;
    let c11 = c011 * (1.0 - tx) + c111 * tx;

    let c0 = c00 * (1.0 - ty) + c10 * ty;
    let c1 = c01 * (1.0 - ty) + c11 * ty;

    c0 * (1.0 - tz) + c1 * tz
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_marching_cubes_empty() {
        let sdf = SdfVolume::new([10, 10, 10], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let mesh = marching_cubes(&sdf, 0.0);
        // All values are MAX (outside), so no triangles
        assert!(mesh.is_empty());
    }

    #[test]
    fn test_marching_cubes_sphere() {
        // Create SDF for a sphere centered at (5,5,5) with radius 3
        let mut sdf = SdfVolume::new([10, 10, 10], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let center = [5.0, 5.0, 5.0];
        let radius = 3.0;

        for z in 0..10 {
            for y in 0..10 {
                for x in 0..10 {
                    let world = sdf.index_to_world([x, y, z]);
                    let dist = ((world[0] - center[0]).powi(2)
                        + (world[1] - center[1]).powi(2)
                        + (world[2] - center[2]).powi(2))
                    .sqrt()
                        - radius;
                    sdf.set(x, y, z, dist);
                }
            }
        }

        let mesh = marching_cubes(&sdf, 0.0);

        // Should have generated some triangles
        assert!(!mesh.is_empty(), "Mesh should not be empty");
        assert!(mesh.triangle_count() > 0, "Should have triangles");

        // Normals should be computed
        assert_eq!(mesh.normals.len(), mesh.vertices.len());
    }

    #[test]
    fn test_marching_cubes_cube() {
        // Create SDF for a cube
        let mut sdf = SdfVolume::new([10, 10, 10], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);

        for z in 0..10u32 {
            for y in 0..10u32 {
                for x in 0..10u32 {
                    // Inside box [3,7]^3
                    let inside = x >= 3 && x <= 7 && y >= 3 && y <= 7 && z >= 3 && z <= 7;
                    sdf.set(x, y, z, if inside { -1.0 } else { 1.0 });
                }
            }
        }

        let mesh = marching_cubes(&sdf, 0.0);
        assert!(!mesh.is_empty());

        // A box should have ~12 triangles (2 per face)
        // But with non-interpolated edges, we get more
        assert!(
            mesh.triangle_count() >= 12,
            "Box should have at least 12 triangles"
        );
    }

    #[test]
    fn test_edge_table_symmetry() {
        // Edge table entry 0 and 255 should be 0 (all in or all out)
        assert_eq!(EDGE_TABLE[0], 0);
        assert_eq!(EDGE_TABLE[255], 0);
    }

    #[test]
    fn test_tri_table_termination() {
        // Entry 0 should have only -1s
        assert_eq!(TRI_TABLE[0][0], -1);

        // Entry 1 should have some valid entries
        assert!(TRI_TABLE[1][0] >= 0);
    }

    #[test]
    fn test_interpolated_vertices_not_snapped_to_grid() {
        let mut sdf = SdfVolume::new([12, 12, 12], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        for z in 0..12 {
            for y in 0..12 {
                for x in 0..12 {
                    let wx = sdf.index_to_world([x, y, z])[0];
                    sdf.set(x, y, z, wx - 4.3);
                }
            }
        }
        let mesh = marching_cubes(&sdf, 0.0);
        assert!(!mesh.vertices.is_empty());
        let has_fractional_x = mesh
            .vertices
            .iter()
            .any(|v| (v[0].fract()).abs() > 1e-3 && (1.0 - v[0].fract()).abs() > 1e-3);
        assert!(
            has_fractional_x,
            "expected interpolated (non-grid-snapped) vertices"
        );
    }

    #[test]
    fn test_active_bounds_matches_full_scan_result() {
        let mut sdf_full = SdfVolume::new([20, 20, 20], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        for z in 0..20 {
            for y in 0..20 {
                for x in 0..20 {
                    let wx = x as f32 - 10.0;
                    let wy = y as f32 - 10.0;
                    let wz = z as f32 - 10.0;
                    let dist = (wx * wx + wy * wy + wz * wz).sqrt() - 5.0;
                    sdf_full.set(x, y, z, dist);
                }
            }
        }

        let mut sdf_roi = sdf_full.clone();
        sdf_roi.active_bounds = Some([4, 4, 4, 16, 16, 16]);

        let mesh_full = marching_cubes(&sdf_full, 0.0);
        let mesh_roi = marching_cubes(&sdf_roi, 0.0);
        assert_eq!(mesh_full.triangle_count(), mesh_roi.triangle_count());
    }
}
