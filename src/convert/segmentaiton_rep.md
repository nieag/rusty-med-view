

---

## 1. ROI Representations

### Contour Sets
A contour set is a collection of contours where each contour is defined as a sequence of points lying in the same plane.
*   **Creation:** Created using tools (brush, polygon, spline, freehand, smart contour) or imported.
*   **Import Constraints:** Imported contours must lie in a plane corresponding to an image stack slice.
*   **Orientation:** While typically transversal, contours can be drawn or reconstructed in sagittal and coronal planes. If drawn in these planes, the entire set is converted to that orientation.

### Voxel ROIs
A voxel ROI is defined by its corner (patient coordinates), resolution, size, and an array of voxel values.
*   **Values:** RayStation v2025 uses relative volumes with values ranging from **0 to 255**.
*   **Threshold:** If all values are below **127.5**, the geometry is considered empty.
*   **Generation:** Typically generated via margin tools (expansion/contraction) or gray-level thresholding.

### Triangle Meshes
Triangle meshes are defined as a set of triplets of vertices in patient coordinates.
*   **Usage:** Used for model-based (MBS) and atlas-based segmentation (ABS).
*   **Visualization:** Used for 3D visualization and 3D interaction tools.

---

## 2. Shape Management

### Primary Shape
Every ROI has a **primary shape**, determined by the last operation performed on it:
*   **Margins/Algebra:** Usually converts the primary shape to **voxels**.
*   **Manual 2D Drawing:** Converts the primary shape to a **contour set**.
*   **Manual 3D Operations:** Converts the primary shape to a **triangle mesh**.

### Reconstructed Shapes
These are temporary shapes generated for visualization purposes. They are not saved with the case but are cached while the case is open. This allows an ROI to maintain its primary data while being visualized in other formats (e.g., seeing a 3D mesh of a contour-based ROI).

### ROI Volume
The volume of an ROI is **always calculated from its voxel ROI shape**. 
*   If the primary shape is a contour set or triangle mesh, a reconstructed voxel shape is used for the calculation.
*   **Note:** Volume is not calculated for open triangle mesh ROIs as the surface is non-closed.

---

## 3. Input and Output (11.1.2 - 11.1.3)

### Input
The algorithms require the following DICOM-standard data:
*   Patient/Phantom description (CT, CBCT, MR, or PET images).
*   Defined Regions of Interest (ROIs).
*   Visualization type (2D or 3D).
*   For 2D: The specific plane (transversal, sagittal, or coronal).

### Output
*   **2D Visualization:** 2D contours in the requested plane.
*   **3D Visualization:** Triangle meshes.
*   **Export:** ROIs are converted to **transversal contour sets** for DICOM RT-struct export. 
    *   *Note: A DICOM export/import roundtrip is not guaranteed to result in identical ROIs.*

---

## 4. Geometry Conversions (11.1.4)

### Converting Voxel ROIs to Contour Sets
*   **Method:** Marching Squares algorithm.
*   **Logic:** Contours are created as isocontours at an approximate 50% level of the surface voxels.
*   **Note:** Geometry between slices may be lost during this conversion, potentially affecting ROI volume.

### Converting Triangle Meshes to Contour Sets
*   **Method:** Computing intersections between the selected plane and the triangles.
*   **Note:** Open triangle meshes result in open contours and cannot be converted to a standard contour set representation.

### Converting Voxel ROIs to Triangle Meshes
*   **Standard Algorithm:** Creates more/smaller triangles on high curvature parts and fewer/larger triangles on flatter parts.
*   **Alternative:** Marching Cubes (faster, but produces less smooth surfaces).

### Converting Contour Sets to Triangle Meshes
*   **Process:** This is an indirect conversion. The contour set is first converted to a voxel ROI, which is then converted to a triangle mesh.

### Converting Contour Sets to Voxel ROIs
Uses **shape-based interpolation**.

#### Grid Properties
*   **Voxel Size:** Typically close to $0.5^3 \text{ mm}^3$ (Range: $0.1^3 \text{ mm}^3$ to $2.5^3 \text{ mm}^3$).
*   **Bounding Box:** Size of the contours plus a margin of 3.5 voxels.

#### Algorithm Steps
1.  Binarize contours to selected resolution.
2.  Fill binary contours using a seed fill algorithm.
3.  Compute area fraction for intersected pixels (0% to 100%).
4.  Compute a signed distance transform (Fast marching distance transform).
5.  Perform linear interpolation between slices.
6.  Add "hats" to contours without overlapping neighbors (extending half-way to the next slice).

> **Note:** If the contour set contains irregularities, the conversion may not be perfectly accurate. However, a contours-voxels-contours roundtrip usually results in a difference of less than the size of one reconstruction voxel.