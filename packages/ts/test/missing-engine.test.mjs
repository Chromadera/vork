import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {text, render} from '../dist/index.js';

test('missing PATH and explicit executables explain PATH and VORK_BIN', () => {
  const directory = mkdtempSync(join(tmpdir(), 'vork-missing-'));
  const previous = {...process.env};
  try {
    for (const binary of ['vork', join(directory, 'missing-vork')]) {
      process.env.VORK_BIN = binary;
      process.env.PATH = directory;
      for (const call of [() => text('label', {body: 'hello', font: 'font.ttf'}), () => render('scene.json', {out: 'out.png'})]) {
        assert.throws(call, /vork executable not found:.*PATH.*VORK_BIN.*full path/);
      }
    }
  } finally {
    process.env = previous;
    rmSync(directory, {recursive: true});
  }
});
