import { execFileSync } from 'node:child_process';
import { test } from 'node:test';

// Regenerates in memory from Rust: catches changes to schema/defaults, either help
// surface without accepting stale snapshots.
test('schema, defaults and help references agree', () => {
  execFileSync(process.execPath, ['generate.mjs', '--check'], {
    cwd: new URL('..', import.meta.url), stdio: 'pipe',
  });
});
