import { stdCommand } from './cli.js';
import { checkName, checkOptions } from './validate.js';
import type { GeometryFragment } from './shapes.js';

type V2 = [number, number];
export interface ExtrudeOptions {
  /** Closed [x, y] polygon in metres; either winding; a repeated first point is ignored. */
  outline: V2[];
  /** Polygons cut through the outline; inside it, not crossing or nesting. */
  holes?: V2[][];
  /** Full thickness along Z in metres, centred on z = 0. */
  depth?: number;
  /** Rounded inward rim radius in metres. */
  bevelDepth?: number;
  /** Quarter-circle subdivisions, 1..32. */
  bevelResolution?: number;
}
/** Polygon with holes → closed prism, in XY with +Z front, centred in Z. Uses the installed
 * vork std loader (override its executable with VORK_BIN). Wall corners turning more than 30°
 * are sharp. Cap UVs span the outline's bounds (u along +X, mirrored on the back; v down from
 * the top); wall u runs by length along each contour from its first point, v front to back.
 * @help XY outline/holes: either winding; depth:full Z thickness, centred; +Z front; bevel_depth:inward rim radius; bevel_resolution:1..32; wall corners >30° sharp; cap uv spans outline bounds (v down); wall u along contour, v front→back
 */
export function extrude(name: string = "extrude", {depth = 1, bevelDepth = 0, bevelResolution = 4, ...options}: ExtrudeOptions): GeometryFragment {
  checkName(name, 'extrude name');
  checkOptions(options, ['depth','outline','holes','bevelDepth','bevelResolution']);
  return stdCommand('extrude', {name,depth,bevel_depth:bevelDepth,bevel_resolution:bevelResolution,...options}) as GeometryFragment;
}
