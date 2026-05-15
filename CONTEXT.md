# Medical Volume Viewer Context

Domain language for the medical volume viewer, ROI editing, segmentation representations, and viewport rendering model.

## Language

**Volume**:
A 3D medical image dataset sampled on a spatial grid.
_Avoid_: Texture, image when referring to domain data

**Labelmap**:
A voxel grid whose values identify segmented regions.
_Avoid_: Overlay when referring to source segmentation data

**ROI**:
A named region of interest that may be viewed, edited, converted, or measured.
_Avoid_: Segmentation, layer when referring to the domain object

**Voxel ROI**:
An ROI whose authoritative shape is discrete occupancy or label data on a voxel grid.
_Avoid_: Label overlay

**Contour ROI**:
An ROI whose authoritative shape is one or more closed planar loops.
_Avoid_: 2D mask

**Mesh ROI**:
An ROI whose authoritative shape is a surface mesh.
_Avoid_: 3D overlay

**Primary Representation**:
The one representation of an ROI that is currently authoritative for editing.
_Avoid_: Source cache, active cache

**Derived Cache**:
A non-authoritative representation rebuilt from the primary representation.
_Avoid_: Secondary source, synced copy

**Plane Family**:
A coherent set of editable planes used for contour authoring.
_Avoid_: Slice stack when oblique planes are possible

**Plane Definition**:
A single plane with enough spatial information to map between plane-local and patient/world space.
_Avoid_: Slice index when the plane is not necessarily axis-aligned

**Patient/World Space**:
The shared physical millimetre coordinate space used to align volumes, ROIs, contours, meshes, and picking.
_Avoid_: Screen space, texture space

**Viewport Space**:
The displayed viewer coordinate space after projection from patient/world space.
_Avoid_: Egui space

**Display-Compatible Voxel Source**:
A voxel source whose geometry can be projected consistently with the displayed volume.
_Avoid_: Texture-aligned overlay

## Relationships

- An **ROI** has exactly one **Primary Representation** at a time.
- A **Primary Representation** may produce zero or more **Derived Caches**.
- A **Labelmap** may be imported as a **Voxel ROI**.
- A **Voxel ROI** owns voxel geometry in **Patient/World Space**.
- A **Contour ROI** uses one editable **Plane Family** at a time.
- A **Plane Definition** maps contour points between plane-local space and **Patient/World Space**.
- **Viewport Space** is derived from **Patient/World Space**, not from GUI layout coordinates.

## Example Dialogue

> **Dev:** "If a user edits a contour, do we update the voxel data immediately?"
> **Domain expert:** "No. The **Contour ROI** remains the **Primary Representation**. The voxel data is a **Derived Cache** rebuilt when volume measurement or voxel overlay display needs it."

## Flagged Ambiguities

- "Overlay" can mean either displayed pixels or source segmentation data. Use **Labelmap** for imported voxel segmentation data.
- "Slice" can imply an axis-aligned image index. Use **Plane Definition** when oblique planes are possible.
- "Segmentation" is broad. Use **ROI** for the domain object and **Primary Representation** for its editable shape.
