#!/usr/bin/env node
'use strict';
const {spawn} = require('node:child_process');
const target = `${process.platform === 'win32' ? 'windows' : process.platform}-${process.arch}`;
if (!['linux-x64', 'darwin-arm64', 'windows-x64'].includes(target)) {
  console.error(`vork: unsupported platform ${target}`);
  process.exit(1);
}
let binary;
try {
  binary = require.resolve(`@voyager-so/vork-${target}/runtime/${process.platform === 'win32' ? 'vork.exe' : 'vork'}`);
} catch {
  console.error('vork: native package missing. Reinstall @voyager-so/vork with optional dependencies enabled (--include=optional).');
  process.exit(1);
}
const args = process.argv.slice(2);
const child = spawn(binary, args, {stdio: 'inherit'});
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
child.on('error', error => {
  console.error(`vork: ${error.message}`);
  process.exitCode = 1;
});
child.on('exit', (code, signal) => {
  if (signal) {
    for (const forwarded of ['SIGINT', 'SIGTERM']) process.removeAllListeners(forwarded);
    process.kill(process.pid, signal);
  }
  else process.exitCode = code ?? 1;
});
