import { checkOptions } from './validate.js';
import { stdCommand } from './cli.js';
import type { Scene } from '../schema.js';

export type EnvironmentFragment = Pick<Scene, 'images' | 'materials' | 'nodes' | 'animation'>;
export interface EnvironmentImageOptions {
  file: string;
  output: string;
  name?: string;
  threshold?: number;
  fps?: number;
  colorSpace?: 'sRGB' | 'Linear Rec.709' | 'Non-Color';
}
/** Preprocess HDR lighting into an ordinary image program and declared sun. */
export function environmentImage(options: EnvironmentImageOptions): EnvironmentFragment {
  checkOptions(options, ['file','output','name','threshold','fps','colorSpace']);
  const {colorSpace, ...rest} = options;
  return stdCommand('environment', {...rest, color_space: colorSpace}) as EnvironmentFragment;
}
/** Nishita atmosphere and an ordinary solar emitter. */
export function sky(options: {name?: string; sunElevation?: number; sunRotation?: number; altitude?: number; airDensity?: number; aerosolDensity?: number; ozoneDensity?: number; sunSize?: number; sunIntensity?: number} = {}): EnvironmentFragment {
  checkOptions(options, ['name','altitude','sunElevation','sunRotation','airDensity','aerosolDensity','ozoneDensity','sunSize','sunIntensity']);
  const {sunElevation, sunRotation, airDensity, aerosolDensity, ozoneDensity, sunSize, sunIntensity, ...rest} = options;
  return stdCommand('sky', {...rest, sun_elevation: sunElevation, sun_rotation: sunRotation, air_density: airDensity, aerosol_density: aerosolDensity, ozone_density: ozoneDensity, sun_size: sunSize, sun_intensity: sunIntensity}) as EnvironmentFragment;
}
