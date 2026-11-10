import {execFileSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import {parse, join} from 'node:path';
import {engine, launchError} from './engine.js';
import {checkOptions} from './validate.js';

export interface ExportOptions {
  out: string;
  root?: string;
  /** Seconds, or an exact rational such as "1/24". */
  at?: number | string;
  /** Inclusive source frames; either endpoint enables a clip and conflicts with at. */
  frameStart?: number;
  frameEnd?: number;
  /** Individually acknowledge reported appearance losses. Refusals cannot be bypassed. */
  allowLosses?: string[];
}
export interface ExportIssue {
  code: string;
  source: string;
  severity: 'error' | 'warning';
  requested: string;
  dropped: string;
  remediation: string;
  acknowledged: boolean;
}
export interface ExportReport {
  version: number;
  status: 'complete' | 'failed';
  issues: ExportIssue[];
  counts: {skins?: number; animations?: number; joints?: number; cameras?: number; lights?: number; nodes: number; meshes: number; materials: number; images: number; vertices?: number; triangles?: number; buffer_bytes?: number};
  [key: string]: unknown;
}
/** Export an evaluated snapshot or an inclusive frame-range clip; return NAME.export.json. */
export function exportAsset(scene: string, options: ExportOptions): ExportReport {
  checkOptions(options, ['out', 'root', 'at', 'frameStart', 'frameEnd', 'allowLosses']);
  const args = ['export', scene, '--out', options.out];
  if (options.root !== undefined) args.push('--root', options.root);
  if (options.at !== undefined) args.push('--at', String(options.at));
  if (options.frameStart !== undefined) args.push('--frame-start', String(options.frameStart));
  if (options.frameEnd !== undefined) args.push('--frame-end', String(options.frameEnd));
  for (const code of options.allowLosses ?? []) args.push('--allow-loss', code);
  try { execFileSync(engine(), args, {stdio: ['ignore', 'inherit', 'inherit']}); }
  catch (error) { launchError(error); }
  const path = parse(options.out);
  return JSON.parse(readFileSync(join(path.dir, `${path.name}.export.json`), 'utf8'));
}
