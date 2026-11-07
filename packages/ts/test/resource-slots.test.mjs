import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, readFile, rm, realpath } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { writeScene } from '../dist/index.js';

test('every schema resource slot preserves identity, including recursive and opaque names', async () => {
  const cases = JSON.parse(execFileSync(process.platform === 'win32' ? 'python' : 'python3', ['../python/test_resource_slots.py'], {encoding: 'utf8'}));
  assert.ok(cases.length > 100);
  const root = await mkdtemp(join(tmpdir(), 'vork-resource-slots-'));
  try {
    const source = join(root, 'source');
    await mkdir(source);
    await writeFile(join(source, 'resource.bin'), 'resource');
    for (const {path, scene} of cases) {
      const original = structuredClone(scene);
      const output = join(root, 'export', 'scene.json');
      await writeScene(scene, output, source);
      const saved = JSON.parse(await readFile(output, 'utf8'));
      const file = path.reduce((value, key) => value[key], saved);
      assert.equal(await realpath(resolve(root, 'export', file)), await realpath(join(source, 'resource.bin')), path.join('.'));
      assert.deepEqual(scene, original);
    }
  } finally { await rm(root, {recursive: true, force: true}); }
});
