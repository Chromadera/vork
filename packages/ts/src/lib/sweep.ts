import { stdCommand } from './cli.js';
import { checkName, checkOptions } from './validate.js';
import type { GeometryFragment } from './shapes.js';

type V2 = [number, number];
type V3 = [number, number, number];
export interface SweepOptions {
  /** Profile polyline in metres; repeat the first point to close the section. */
  profile: V2[];
  /** Path polyline in metres; repeat the first point to close the path. */
  path: V3[];
  /** Positive multiplier, uniform or one value per path point. */
  scale?: number | number[];
  /** Total rotation in radians, distributed by path arc length; whole turns for closed paths. */
  twist?: number;
  /** Fill ends of a closed section along an open path. */
  caps?: boolean;
}
/** Sweep a profile along a polyline with rotation-minimising frames. Initial profile y
 * follows the most perpendicular world axis, preferring +Y, then +Z; x = tangent cross y.
 * Closed paths require equal endpoint scales and whole turns of twist.
 * @help profile:[x,y][] in the plane normal to the path, x to the right, y up at the path start; path:polyline in metres; consecutive duplicates ignored; profile last=first closes the section (caps allowed), otherwise an open strip; path last=first closes the loop (no caps); frames are rotation-minimising along the path; scale: one value or one per path point, applied to the profile; twist: total rotation of the profile from path start to end in radians, linear in arc length; corners >30° sharp; u along profile, v along path
 */
export function sweep(name: string = "sweep", {profile, path, scale = 1, twist = 0, caps = true, ...unknown}: SweepOptions): GeometryFragment {
  checkName(name, 'sweep name');
  checkOptions(unknown, ['profile', 'path', 'scale', 'twist', 'caps']);
  return stdCommand('sweep', {name, profile, path, scale, twist, caps}) as GeometryFragment;
}
