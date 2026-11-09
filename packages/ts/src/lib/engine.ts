import { createRequire } from 'node:module';
import { readFileSync, existsSync } from 'node:fs';

/** Resolve beside this SDK so project dependencies work without a global install. */
export function engine(): string {
  if (process.env.VORK_BIN) return process.env.VORK_BIN;
  const target = `${process.platform}-${process.arch}`;
  if (!['linux-x64', 'darwin-arm64', 'win32-x64'].includes(target)) {
    throw new Error(`Unsupported vork platform: ${target}`);
  }
  const name = target.replace('win32', 'windows');
  try {
    return createRequire(import.meta.url).resolve(`@voyager-so/vork-${name}/runtime/${process.platform === 'win32' ? 'vork.exe' : 'vork'}`);
  } catch {
    // Registry dist/lib and archive lib have different depths.
    for (const path of ['../package.json', '../../package.json']) {
      const url = new URL(path, import.meta.url);
      if (existsSync(url) && JSON.parse(readFileSync(url, 'utf8')).optionalDependencies) {
        throw new Error('vork native package missing; reinstall @voyager-so/vork with --include=optional');
      }
    }
    // Archive/source SDKs retain the existing PATH install route.
    return 'vork';
  }
}

/** Translate launch failures while preserving other process errors. */
export function launchError(error: unknown): never {
  if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
    throw new Error(`vork executable not found: ${JSON.stringify((error as {path?: string}).path)}. Add the directory containing vork to PATH (for example, export PATH="$HOME/.local/bin:$PATH"), or set VORK_BIN to the full path to the vork executable.`, {cause: error});
  }
  throw error;
}
