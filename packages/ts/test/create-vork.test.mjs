import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync, mkdirSync, writeFileSync, readFileSync, cpSync, readdirSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {spawnSync, execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {createServer} from 'node:http';

// Exercise the creator, real init and real offline npm install without a registry.
// The local SDK fixture only launches the built engine; the registry rehearsal
// separately covers the packaged SDK and native optional dependencies.
test('creator initializes every language without overwriting init files', async () => {
  const root = mkdtempSync(path.join(tmpdir(), 'vork-creator-'));
  const engine = path.resolve(process.env.VORK_BIN ?? '../../target/release/vork');
  const npm = process.env.npm_execpath;
  const env = {...process.env, npm_config_cache: path.join(root, 'cache'), VORK_BIN: engine};
  async function run(args, cwd) {
    const result = await promisify(execFile)(process.execPath, args, {cwd, env});
    return result.stdout;
  }
  let server;
  try {
    const sdk = path.join(root, 'sdk');
    mkdirSync(sdk);
    writeFileSync(path.join(sdk, 'package.json'), JSON.stringify({name: '@voyager-so/vork', version: '0.0.0'}));
    writeFileSync(path.join(sdk, 'cli.cjs'), `const r = require('node:child_process').spawnSync(process.env.VORK_BIN, process.argv.slice(2), {stdio: 'inherit'}); process.exit(r.status ?? 1);`);
    const tarball = JSON.parse(await run([npm, 'pack', '--json', '--ignore-scripts'], sdk))[0].filename;
    server = createServer((request, response) => {
      if (request.url === '/sdk.tgz') response.end(readFileSync(path.join(sdk, tarball)));
      else {
        response.setHeader('Content-Type', 'application/json');
        response.end(JSON.stringify({name: '@voyager-so/vork', 'dist-tags': {latest: '0.0.0'}, versions: {'0.0.0': {name: '@voyager-so/vork', version: '0.0.0', dist: {tarball: `http://127.0.0.1:${server.address().port}/sdk.tgz`}}}}));
      }
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    env.npm_config_registry = `http://127.0.0.1:${server.address().port}`;
    const creator = path.join(root, 'creator');
    mkdirSync(creator);
    cpSync(new URL('../../create-vork/cli.cjs', import.meta.url), path.join(creator, 'cli.cjs'));
    await run([npm, 'install', '@voyager-so/vork@0.0.0', '--ignore-scripts', '--no-audit', '--no-fund'], creator);
    await new Promise(resolve => server.close(resolve));
    for (const language of ['json', 'typescript', 'python']) {
      const project = path.join(root, `${language} starter project`);
      const output = await run([path.join(creator, 'cli.cjs'), project, '--language', language], root);
      assert.equal(output.match(/vork render scene.json --out render.png/g)?.length, 1);
      assert.ok(output.includes('  npx vork render'));
      const readme = readFileSync(path.join(project, 'README.md'), 'utf8');
      assert.ok(readme.includes('npx vork render'));
      assert.ok(!readme.includes('\nvork render'));
      if (language === 'typescript') {
        assert.ok(readme.includes('npm install @voyager-so/vork'));
        assert.ok(!readme.includes('.local/lib/vork/sdk'));
        assert.ok(readme.includes('--experimental-strip-types'));
        assert.ok(readme.includes('EBADENGINE'));
      }
      if (language === 'python') {
        assert.ok(readme.includes('python -m pip install "/path/to/extracted-vork/python"'));
        assert.ok(readme.includes('packages/python'));
        assert.ok(readme.includes('export VORK_BIN='));
        assert.ok(readme.includes('$env:VORK_BIN = node -p'));
        assert.ok(!readme.includes('pip install vork=='));
        assert.ok(readme.includes('npx vork help sdk'));
        assert.ok(!readme.includes('.local/lib/vork/python'));
        assert.ok(!readme.includes('Programs\\vork\\python'));
      }
      assert.ok(!output.includes('--profile std/draft'));
      const baseline = path.join(root, `${language} baseline`);
      await promisify(execFile)(engine, ['init', baseline, '--language', language], {env});
      for (const file of readdirSync(baseline).filter(name => !['package.json', 'README.md'].includes(name))) {
        assert.deepEqual(readFileSync(path.join(project, file)), readFileSync(path.join(baseline, file)));
      }
      const manifest = JSON.parse(readFileSync(path.join(project, 'package.json')));
      assert.equal(manifest.private, true);
      assert.equal(manifest.type, 'module');
      assert.deepEqual(manifest.dependencies, {'@voyager-so/vork': '0.0.0'});
      assert.equal(JSON.parse(readFileSync(path.join(project, 'node_modules/@voyager-so/vork/package.json'))).version, '0.0.0');
      const expected = ['README.md', 'node_modules', 'package-lock.json', 'package.json', 'scene.json'];
      if (language !== 'json') expected.push(language === 'python' ? 'scene.py' : 'scene.ts');
      assert.deepEqual(readdirSync(project).sort(), expected.sort());
      JSON.parse(readFileSync(path.join(project, 'scene.json')));
      const before = readFileSync(path.join(project, 'package.json'), 'utf8');
      const conflict = spawnSync(process.execPath, [path.join(creator, 'cli.cjs'), project, '--language', language], {env, encoding: 'utf8'});
      assert.equal(conflict.status, 1);
      assert.equal(readFileSync(path.join(project, 'package.json'), 'utf8'), before);
    }
  } finally {
    server?.close();
    rmSync(root, {recursive: true, force: true});
  }
});
