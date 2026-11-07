/** Preserve the original renderer's sphere/cylinder azimuth tessellation. */
export const SHAPE_LONGITUDES = 32;
/** Preserve the original renderer's pole-to-pole sphere tessellation. */
export const SHAPE_LATITUDES = 24;
/** Subdivisions per rounded-box face: enough to resolve default rounded edges. */
export const ROUNDED_BOX_SEGMENTS = 8;
/** Blender 5.2.1 Shade Auto Smooth / Smooth by Angle default: a profile or outline corner
 * turning by more than this is sharp (split normals); gentler corners share a smooth normal.
 * vork-std's native extrude repeats it (crates/vork-std/src/extrude.rs). */
export const SMOOTH_ANGLE_DEGREES = 30;
/** SDK authoring choice: externalize buffers of at least 4 KiB to keep scene JSON readable. */
export const BINARY_BUFFER_MIN_BYTES = 4096;
