import { stdCommand } from './cli.js';
import type { Scene } from '../schema.js';

export type EnvironmentFragment = Pick<Scene, 'images' | 'materials' | 'nodes' | 'animation'>;
export interface EnvironmentImageOptions {
  file: string;
  output: string;
  name?: string;
  threshold?: number;
  fps?: number;
  color_space?: 'sRGB' | 'Linear Rec.709' | 'Non-Color';
}
/** Preprocess HDR lighting into an ordinary image program and declared sun. */
export function environmentImage(options: EnvironmentImageOptions): EnvironmentFragment {
  return stdCommand('environment', options) as EnvironmentFragment;
}
/** Nishita atmosphere and an ordinary solar emitter. */
export function sky(options: {name?: string; sun_elevation?: number; sun_rotation?: number; altitude?: number; air_density?: number; aerosol_density?: number; ozone_density?: number; sun_size?: number; sun_intensity?: number} = {}): EnvironmentFragment {
  return stdCommand('sky', options) as EnvironmentFragment;
}
