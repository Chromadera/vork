#!/usr/bin/env node
'use strict';
const {spawnSync} = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const args = process.argv.slice(2);
const metadata = require.resolve('@voyager-so/vork/package.json');
const cli = path.join(path.dirname(metadata), 'cli.cjs');
function run(command, argv, cwd, stdio = 'inherit') {
  const result = spawnSync(command, argv, {stdio, cwd});
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
  return result;
}
if (args.includes('--help') || args.includes('-h')) {
  run(process.execPath, [cli, 'init', '--help']);
  process.exit(0);
}
// Match init's positional directory; clap owns validation of all options.
let dir = '.';
let positional = false;
for (let i = 0; i < args.length; ++i) {
  if (positional) { dir = args[i]; continue; }
  if (args[i] === '--') { positional = true; continue; }
  if (args[i] === '--language') { ++i; continue; }
  if (!args[i].startsWith('-')) dir = args[i];
}
const project = path.resolve(dir);
if (fs.existsSync(path.join(project, 'package.json')) || fs.existsSync(path.join(project, 'node_modules'))) {
  console.error('create-vork: package.json or node_modules already exists; choose another directory.');
  process.exit(1);
}
const initialized = run(process.execPath, [cli, 'init', ...args], undefined, ['inherit', 'pipe', 'inherit']);
process.stdout.write(initialized.stdout.toString().replace(/^  vork /gm, '  npx vork '));
const version = require(metadata).version;
fs.writeFileSync(path.join(project, 'package.json'), JSON.stringify({private: true, type: 'module', dependencies: {'@voyager-so/vork': version}}, null, 2) + '\n', {flag: 'wx'});
// npm create has already cached this exact dependency and its selected native package.
// Never fetch during initialization; propagate the caller's private registry settings.
run(process.execPath, [process.env.npm_execpath, 'install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund'], project);
console.log('Ready. Enter the project and run:\n  npx vork render scene.json --profile std/draft --out render.png\n  npx vork help');
