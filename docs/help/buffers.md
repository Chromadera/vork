BUFFERS
Use inline data for small geometry; binary files for large arrays.
Binary data is tightly packed, little-endian. Exact fields: buffers --all.
<!-- all -->
COMPUTED PLACEMENT REFERENCE
Use computed placement for opaque, unskinned geometry and opaque instances.
TRANSLATION/ROTATION/SCALE are node-relative; ROTATION is a unit XYZW quaternion.
instances.count selects a live prefix from zero to installed capacity.
Computed instance material attributes follow GPU updates, including shutter samples
and seeks. Displacement cannot read computed instance attributes. Transparent
placements must be authored, not compute-bound.
