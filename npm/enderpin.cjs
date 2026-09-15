#!/usr/bin/env node
'use strict';

const { spawn } = require('node:child_process');
const { constants } = require('node:os');
const metadata = require('./package.json');
const name = `enderpin-${process.platform}-${process.arch}`;
if (!Object.hasOwn(metadata.optionalDependencies, name)) {
  console.error(`Enderpin does not support ${process.platform}/${process.arch}.`);
  process.exit(1);
}

let binary;
try {
  const executable = process.platform === 'win32' ? 'enderpin.exe' : 'enderpin';
  binary = require.resolve(`${name}/bin/${executable}`);
} catch {
  console.error(`Missing ${name}. Reinstall enderpin with optional dependencies enabled and access to GitHub Releases.`);
  process.exit(1);
}

const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit' });
// Ctrl-C reaches both processes in the terminal's foreground group. Let the
// native CLI finish its graceful shutdown before the wrapper exits.
process.on('SIGINT', () => {});
process.on('SIGTERM', () => child.kill('SIGTERM'));
child.on('error', (error) => {
  console.error(`Could not start Enderpin: ${error.message}`);
  process.exitCode = 1;
});
child.on('exit', (code, signal) => {
  process.exitCode = code ?? (128 + (constants.signals[signal] ?? 1));
});
