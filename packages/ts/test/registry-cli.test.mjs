import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {EventEmitter} from 'node:events';
import vm from 'node:vm';

const source = readFileSync(new URL('../../../scripts/registry-cli.cjs', import.meta.url), 'utf8');
test('registry launcher preserves Windows console shutdown and forwards Unix signals', () => {
  for (const platform of ['win32', 'linux']) {
    const process = Object.assign(new EventEmitter(), {platform, arch: 'x64', argv: ['node', 'cli', 'render'], env: {}, pid: 123});
    const child = new EventEmitter();
    const killed = [];
    child.kill = signal => killed.push(signal);
    const require = Object.assign(() => ({spawn: (_binary, args, options) => {
      assert.deepEqual(Array.from(args), ['render']);
      assert.equal(options.env.VORK_NPM_CONTEXT, '1');
      return child;
    }}), {resolve: () => '/native/vork'});
    vm.runInNewContext(source, {process, require, console});
    for (const signal of platform === 'win32' ? ['SIGINT', 'SIGBREAK'] : ['SIGINT', 'SIGTERM']) process.emit(signal);
    assert.deepEqual(killed, platform === 'win32' ? [] : ['SIGINT', 'SIGTERM']);
    child.emit('exit', 130, null);
    assert.equal(process.exitCode, 130);
  }
});
