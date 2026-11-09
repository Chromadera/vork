import { engine, launchError } from './engine.js';
import { execFileSync } from 'node:child_process';

/** Run `vork std FUNCTION -` (VORK_BIN overrides the executable); failures carry the CLI's message. */
export function stdCommand(name: string, argument: unknown): unknown {
  try {
    return JSON.parse(execFileSync(engine(), ['std', name, '-'], {
      input: JSON.stringify(argument), encoding: 'utf8', maxBuffer: 64 * 1024 * 1024,
      stdio: ['pipe', 'pipe', 'pipe'],
    }));
  } catch (error) {
    const stderr = String((error as { stderr?: unknown }).stderr ?? '').trim();
    if (!stderr) launchError(error);
    throw new Error(`vork std ${name}: ${stderr}`, { cause: error });
  }
}
